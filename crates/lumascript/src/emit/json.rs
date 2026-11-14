//! json: `json.Value` literals, `json.encode`, `json.decode<T>` as one generated reader per type
//! (`lm_jd_<n>`, no interpretation of the data's shape at run time beyond lumart's reader), and json.Value's methods.
//!
//! A json.Value is one complete JSON text in json.encode's form, held as a `lumart_text`. A reader function reads
//! one value of its type into a place that is zero (or empty) when it starts, and gives 0, 1 after a script error
//! (S0911), or 2 when the data there is not that type (the decode is then none); the place may then hold part of a
//! value, which its caller gives back.

use super::expr::number_text;
use super::{c_bytes, c_string, components, counted, scalar_c, signed, unescape, Ex, Fx, Gen, NotWritten};
use crate::ast::{Expr, ExprKind, ItemKind, StrPart, StrPrefix};
use crate::check::{ItemRef, Ty};
use crate::game::{GameTy, Scalar, Value};

/// A part of a json.Value literal: bytes known when compiling, or a value written when it runs.
enum Piece<'e> {
    Bytes(Vec<u8>),
    /// A value of json data (its own type), written as json.
    Data(&'e Expr),
    /// A string with interpolations, as a JSON string.
    Text(&'e Expr),
}

fn put(out: &mut Vec<Piece<'_>>, b: &[u8]) {
    if let Some(Piece::Bytes(last)) = out.last_mut() {
        last.extend_from_slice(b);
    } else {
        out.push(Piece::Bytes(b.to_vec()));
    }
}

/// Whether an expression of type json.Value is a literal written into one.
pub(super) fn json_literal(e: &ExprKind) -> bool {
    match e {
        ExprKind::Number(_) | ExprKind::Str(_) | ExprKind::True | ExprKind::False | ExprKind::None | ExprKind::Color(_) | ExprKind::List(_) | ExprKind::Braces(_) => true,
        ExprKind::Neg(x) => matches!(x.kind, ExprKind::Number(_)),
        _ => false,
    }
}

/// Bytes as a JSON string, as lumart_sb_json_str writes them (a byte that starts no UTF-8 is U+FFFD).
pub(super) fn json_quote(s: &[u8]) -> Vec<u8> {
    let mut out = vec![b'"'];
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c >= 0x80 {
            let k = (2..=4).find(|k| i + k <= s.len() && std::str::from_utf8(&s[i..i + k]).is_ok());
            match k {
                Some(k) => {
                    out.extend_from_slice(&s[i..i + k]);
                    i += k;
                }
                None => {
                    out.extend_from_slice(b"\\ufffd");
                    i += 1;
                }
            }
            continue;
        }
        if c == b'"' || c == b'\\' {
            out.push(b'\\');
            out.push(c);
        } else if c < 0x20 {
            out.extend_from_slice(format!("\\u00{c:02x}").as_bytes());
        } else {
            out.push(c);
        }
        i += 1;
    }
    out.push(b'"');
    out
}

/// A number literal as a JSON number: an integer in decimal (exact, any size), anything with a fraction, an
/// exponent or a float suffix as the shortest decimal of its f64 that reads back. None past f64.
fn json_number(text: &str) -> Option<String> {
    let (digits, radix) = number_text(text);
    let float = text.ends_with("f32") || text.ends_with("f64") || (radix == 10 && digits.contains(['.', 'e', 'E']));
    if !float {
        return u128::from_str_radix(&digits, radix).ok().map(|v| v.to_string());
    }
    let v: f64 = if radix == 10 { digits.parse().ok()? } else { u128::from_str_radix(&digits, radix).ok()? as f64 };
    // Debug writes the shortest round trip, always with a `.` or an exponent: valid JSON for every finite value.
    v.is_finite().then(|| format!("{v:?}"))
}

/// The signed range of an integer type, as C.
fn int_range(s: Scalar) -> (&'static str, &'static str) {
    match s {
        Scalar::I8 => ("(int64_t)-128", "(int64_t)127"),
        Scalar::I16 => ("(int64_t)-32768", "(int64_t)32767"),
        Scalar::I32 => ("(int64_t)-2147483648", "(int64_t)2147483647"),
        _ => ("(-9223372036854775807LL - 1)", "9223372036854775807LL"),
    }
}

fn uint_max(s: Scalar) -> &'static str {
    match s {
        Scalar::U8 => "255ull",
        Scalar::U16 => "65535ull",
        Scalar::U32 => "4294967295ull",
        _ => "18446744073709551615ull",
    }
}

impl<'a> Gen<'a> {
    // ------------------------------------------------------------ literals

    /// A json.Value literal: a static text when every part is known, else written into a builder when it runs.
    pub(super) fn json_value(&mut self, fx: &mut Fx, e: &Expr) -> Ex {
        let mut pieces = Vec::new();
        self.json_pieces(fx, e, &mut pieces)?;
        if let [Piece::Bytes(b)] = pieces.as_slice() {
            let b = b.clone();
            return Ok(self.text_literal(fx.module, &b));
        }
        let (file, line) = (c_string(&fx.file), e.at.line);
        let b = fx.builder();
        for p in pieces {
            match p {
                Piece::Bytes(v) => fx.emit(format!("lumart_sb_bytes(&{b}, {}, {}u);", c_bytes(&v), v.len())),
                Piece::Data(x) => self.value_text_into(fx, &b, x, true)?,
                Piece::Text(x) => {
                    let inner = fx.builder();
                    self.text_into(fx, &inner, x)?;
                    fx.fail_if(&format!("lumart_sb_done(&{inner}, {file}, {})", x.at.line));
                    fx.emit(format!("lumart_sb_json_str(&{b}, {inner}.p, {inner}.len);"));
                }
            }
        }
        let r = fx.fresh("lumart_text");
        fx.fail_if(&format!("lumart_text_from_sb(&{r}, &{b}, {file}, {line})"));
        Ok(r)
    }

    fn json_pieces<'e>(&mut self, fx: &Fx, e: &'e Expr, out: &mut Vec<Piece<'e>>) -> Result<(), NotWritten> {
        // A string literal (a map's key is text): its JSON string when compiling, below.
        let constant_text = matches!(&e.kind, ExprKind::Str(l) if matches!(l.prefix, StrPrefix::None | StrPrefix::Raw) && l.parts.iter().all(|p| matches!(p, StrPart::Text(_))));
        let t = self.ty(fx.module, e);
        if t != Ty::Json && !(constant_text && t == Ty::Text) {
            out.push(Piece::Data(e));
            return Ok(());
        }
        match &e.kind {
            ExprKind::Paren(x) => self.json_pieces(fx, x, out)?,
            ExprKind::Number(text) => match json_number(text) {
                Some(n) => put(out, n.as_bytes()),
                None => return self.not_yet(fx, e, "this number in JSON"),
            },
            ExprKind::Neg(x) => {
                let ExprKind::Number(text) = &x.kind else { return self.not_yet(fx, e, "this value in JSON") };
                match json_number(text) {
                    Some(n) => put(out, format!("-{n}").as_bytes()),
                    None => return self.not_yet(fx, e, "this number in JSON"),
                }
            }
            ExprKind::True => put(out, b"true"),
            ExprKind::False => put(out, b"false"),
            ExprKind::None => put(out, b"null"),
            ExprKind::Str(lit) => {
                if lit.prefix != StrPrefix::None && lit.prefix != StrPrefix::Raw {
                    return self.not_yet(fx, e, "bytes in JSON");
                }
                if lit.parts.iter().all(|p| matches!(p, StrPart::Text(_))) {
                    let mut bytes = Vec::new();
                    for p in &lit.parts {
                        if let StrPart::Text(raw) = p {
                            bytes.extend(if lit.prefix == StrPrefix::Raw { raw.as_bytes().to_vec() } else { unescape(raw) });
                        }
                    }
                    put(out, &json_quote(&bytes));
                } else {
                    out.push(Piece::Text(e));
                }
            }
            // A colour literal: `[r, g, b, a]`, each channel's f32 as the shortest decimal that reads back.
            ExprKind::Color(text) => {
                let hex = text.trim_start_matches('#');
                let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f32 / 255.0;
                let a = if hex.len() >= 8 { byte(6) } else { 1.0 };
                put(out, format!("[{:?}, {:?}, {:?}, {a:?}]", byte(0), byte(2), byte(4)).as_bytes());
            }
            ExprKind::List(items) => {
                put(out, b"[");
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        put(out, b", ");
                    }
                    self.json_pieces(fx, x, out)?;
                }
                put(out, b"]");
            }
            ExprKind::Braces(entries) => {
                put(out, b"{");
                for (i, (k, v)) in entries.iter().enumerate() {
                    let Some(v) = v else { return self.not_yet(fx, e, "a set in JSON") };
                    if i > 0 {
                        put(out, b", ");
                    }
                    // A key is text (the checker's): a JSON string.
                    self.json_pieces(fx, k, out)?;
                    put(out, b": ");
                    self.json_pieces(fx, v, out)?;
                }
                put(out, b"}");
            }
            _ => out.push(Piece::Data(e)),
        }
        Ok(())
    }

    // ------------------------------------------------------------ json.encode, json.decode<T>

    /// `json.encode(x)`: x (already read, of type `xt`) as json, a new text.
    pub(super) fn json_encode(&mut self, fx: &mut Fx, e: &Expr, x: &str, xt: &Ty, arg: &Expr) -> Ex {
        let (file, line) = (c_string(&fx.file), e.at.line);
        let b = fx.builder();
        self.value_text(fx, &b, x, xt, arg, true)?;
        let r = fx.fresh("lumart_text");
        fx.fail_if(&format!("lumart_text_from_sb(&{r}, &{b}, {file}, {line})"));
        Ok(r)
    }

    /// `json.decode<T>(data)`: `data` (already read: text, or bytes as UTF-8) read as a whole `T` by T's reader; none
    /// when it does not read, or when anything but white space follows.
    pub(super) fn json_decode(&mut self, fx: &mut Fx, e: &Expr, data: &str, dt: &Ty, t: &Ty) -> Ex {
        let Ty::Opt(target) = t else { return self.not_yet(fx, e, "json.decode of this type") };
        let (file, line) = (c_string(&fx.file), e.at.line);
        let (p, n) = match dt {
            Ty::Bytes => (format!("lumart_bytes_p({data})"), format!("lumart_list_n({data})")),
            _ => (format!("lumart_text_p({data})"), format!("lumart_text_n({data})")),
        };
        let reader = self.json_reader(fx, target, e)?;
        let ct = self.c_type(t).ok_or_else(super::lost)?;
        let r = fx.fresh(&ct);
        let jr = fx.fresh("lumart_jr");
        let st = fx.fresh("int32_t");
        fx.emit(format!("lumart_jr_init(&{jr}, {p}, {n});"));
        // `some` first, so that giving the option back gives back a value read in part.
        fx.emit(format!("{r}.some = 1;"));
        fx.emit(format!("{st} = {reader}(&{jr}, &{r}.value, {file}, {line});"));
        fx.emit(format!("if ({st} == 0 && !lumart_jr_done(&{jr})) {st} = 2;"));
        if counted(&ct) {
            fx.emit(format!("if ({st} != 0) lm_rel_{ct}(&{r});"));
        } else {
            fx.emit(format!("if ({st} != 0) {r} = ({ct}){{0}};"));
        }
        fx.fail_if(&format!("{st} == 1"));
        Ok(r)
    }

    /// The reader function of `t` (made once per type; a struct holding a list of itself calls its own).
    fn json_reader(&mut self, fx: &Fx, t: &Ty, e: &Expr) -> Result<String, NotWritten> {
        let key = format!("{t:?}");
        if let Some(n) = self.json_readers.get(&key) {
            return Ok(n.clone());
        }
        let name = format!("lm_jd_{}", self.json_readers.len());
        self.json_readers.insert(key, name.clone());
        let ct = self.c_type(t).ok_or_else(super::lost)?;
        let head = format!("int32_t {name}(lumart_jr* r, {ct}* out, const char* file, uint32_t line)");
        self.protos.push(format!("{head};"));
        let mut dx = Fx::new(&fx.file, fx.module);
        dx.emit("(void)r;".into());
        dx.emit("(void)file;".into());
        dx.emit("(void)line;".into());
        self.reader_inline = true;
        self.json_read(&mut dx, t, "(*out)", e)?;
        let text = dx.function(&head, e.at);
        self.json_code.push_str(&text);
        Ok(name)
    }

    /// Code reading one `t` from `r` into `place` (zero when it starts).
    fn json_read(&mut self, dx: &mut Fx, t: &Ty, place: &str, e: &Expr) -> Result<(), NotWritten> {
        let inline = std::mem::replace(&mut self.reader_inline, false);
        if !inline && matches!(t, Ty::Struct(_) | Ty::Enum(_)) {
            let reader = self.json_reader(dx, t, e)?;
            dx.emit(format!("LUMART_JR({reader}(r, &{place}, file, line));"));
            return Ok(());
        }
        match t {
            Ty::Bool => dx.emit(format!("LUMART_JR(lumart_jr_bool(r, &{place}));")),
            Ty::Int(s) if signed(*s) => {
                let (lo, hi) = int_range(*s);
                let v = dx.fresh("int64_t");
                dx.emit(format!("LUMART_JR(lumart_jr_i64(r, {lo}, {hi}, &{v}));"));
                dx.emit(format!("{place} = ({}){v};", scalar_c(*s)));
            }
            Ty::Int(s) => {
                let v = dx.fresh("uint64_t");
                dx.emit(format!("LUMART_JR(lumart_jr_u64(r, {}, &{v}));", uint_max(*s)));
                dx.emit(format!("{place} = ({}){v};", scalar_c(*s)));
            }
            Ty::Float(Scalar::F64) => dx.emit(format!("LUMART_JR(lumart_jr_f64(r, &{place}));")),
            Ty::Float(_) => dx.emit(format!("LUMART_JR(lumart_jr_f32(r, &{place}));")),
            Ty::Text => dx.emit(format!("LUMART_JR(lumart_jr_text(r, &{place}, file, line));")),
            Ty::Json => dx.emit(format!("LUMART_JR(lumart_jr_raw(r, &{place}, file, line));")),
            Ty::Value(Value::Mat34) => {
                let (i, more) = self.json_open(dx, '[');
                for row in 0..3 {
                    self.json_next(dx, ']', &i, &more);
                    let (j, more2) = self.json_open(dx, '[');
                    for c in 0..4 {
                        self.json_next(dx, ']', &j, &more2);
                        dx.emit(format!("LUMART_JR(lumart_jr_f32(r, &{place}.m[{row}][{c}]));"));
                    }
                    self.json_close(dx, ']', &j, &more2);
                }
                self.json_close(dx, ']', &i, &more);
            }
            Ty::Value(v) if !components(*v).is_empty() => {
                let (i, more) = self.json_open(dx, '[');
                for c in components(*v) {
                    self.json_next(dx, ']', &i, &more);
                    dx.emit(format!("LUMART_JR(lumart_jr_f32(r, &{place}.{c}));"));
                    // A colour's channels are 0..1 (docs/modding/language/language.md, section 3): json.encode writes no other.
                    if *v == Value::Color {
                        dx.emit(format!("if (!({place}.{c} >= 0.0f && {place}.{c} <= 1.0f)) {{ lm_st = 2; goto lm_out; }}"));
                    }
                }
                self.json_close(dx, ']', &i, &more);
            }
            Ty::Opt(x) => {
                dx.emit("if (lumart_jr_is_null(r)) {".into());
                dx.emit("    LUMART_JR(lumart_jr_null(r));".into());
                dx.emit("} else {".into());
                dx.depth += 1;
                dx.emit(format!("{place}.some = 1;"));
                self.json_read(dx, x, &format!("{place}.value"), e)?;
                dx.depth -= 1;
                dx.emit("}".into());
            }
            Ty::Tuple(xs) => {
                let (i, more) = self.json_open(dx, '[');
                for (k, x) in xs.iter().enumerate() {
                    self.json_next(dx, ']', &i, &more);
                    self.json_read(dx, x, &format!("{place}._{k}"), e)?;
                }
                self.json_close(dx, ']', &i, &more);
            }
            Ty::List(x) | Ty::Set(x) => {
                let x = (**x).clone();
                let ec = self.c_type(&x).ok_or_else(super::lost)?;
                let d = self.desc(&x).ok_or_else(super::lost)?;
                let (i, more) = self.json_open(dx, '[');
                let item = dx.fresh_kept(&ec);
                dx.emit("for (;;) {".into());
                dx.depth += 1;
                dx.emit(format!("LUMART_JR(lumart_jr_more(r, ']', &{i}, &{more}));"));
                dx.emit(format!("if (!{more}) break;"));
                self.json_read(dx, &x, &item, e)?;
                if matches!(t, Ty::List(_)) {
                    dx.fail_if(&format!("lumart_list_push(&{place}, {d}, &{item}, file, line)"));
                } else {
                    let added = dx.fresh("uint8_t");
                    dx.fail_if(&format!("lumart_map_put(&{place}, {d}, 0, &{item}, 0, &{added}, file, line)"));
                }
                self.json_empty(dx, &item, &ec);
                dx.depth -= 1;
                dx.emit("}".into());
            }
            Ty::Map(k, v) => {
                let (k, v) = ((**k).clone(), (**v).clone());
                let (kc, vc) = (self.c_type(&k).ok_or_else(super::lost)?, self.c_type(&v).ok_or_else(super::lost)?);
                let (kd, vd) = (self.desc(&k).ok_or_else(super::lost)?, self.desc(&v).ok_or_else(super::lost)?);
                let (i, more) = self.json_open(dx, '{');
                let key = dx.fresh_kept(&kc);
                let value = dx.fresh_kept(&vc);
                dx.emit("for (;;) {".into());
                dx.depth += 1;
                dx.emit(format!("LUMART_JR(lumart_jr_more(r, '}}', &{i}, &{more}));"));
                dx.emit(format!("if (!{more}) break;"));
                self.json_key(dx, &k, &key, e)?;
                self.json_read(dx, &v, &value, e)?;
                // A key written twice keeps its last value, as a map literal does.
                let added = dx.fresh("uint8_t");
                dx.fail_if(&format!("lumart_map_put(&{place}, {kd}, {vd}, &{key}, &{value}, &{added}, file, line)"));
                self.json_empty(dx, &key, &kc);
                self.json_empty(dx, &value, &vc);
                dx.depth -= 1;
                dx.emit("}".into());
            }
            Ty::Struct(item) => {
                let item = *item;
                let ItemKind::Struct(s) = &self.item(item).kind else { return Err(super::lost()) };
                let fields: Vec<(String, Option<Expr>)> = s.fields.iter().map(|f| (f.name.name.clone(), f.default.clone())).collect();
                self.json_fields(dx, item, ".", &fields, &format!("{place}."), e)?;
            }
            Ty::Enum(item) => {
                let item = *item;
                let ItemKind::Enum(d) = &self.item(item).kind else { return Err(super::lost()) };
                let cases: Vec<(String, Vec<(String, Option<Expr>)>)> =
                    d.cases.iter().map(|c| (c.name.name.clone(), c.fields.iter().map(|f| (f.name.name.clone(), f.default.clone())).collect())).collect();
                let name = dx.fresh_kept("lumart_text");
                // A case without values: its name, a string.
                let plain = cases.iter().all(|(_, f)| f.is_empty());
                if !plain {
                    dx.emit("if (lumart_jr_next(r) == '\"') {".into());
                    dx.depth += 1;
                }
                dx.emit(format!("LUMART_JR(lumart_jr_text(r, &{name}, file, line));"));
                let mut chain = String::new();
                for (k, (c, f)) in cases.iter().enumerate() {
                    if f.is_empty() {
                        chain.push_str(&format!("if ({}) {place}.tag = {k}u; else ", text_is(&name, c)));
                    }
                }
                dx.emit(format!("{chain}{{ lm_st = 2; goto lm_out; }}"));
                if !plain {
                    // A case with values: `{"Case": {fields by name}}`, that one key.
                    dx.depth -= 1;
                    dx.emit("} else {".into());
                    dx.depth += 1;
                    let (i, more) = self.json_open(dx, '{');
                    self.json_next(dx, '}', &i, &more);
                    dx.emit(format!("LUMART_JR(lumart_jr_key(r, &{name}, file, line));"));
                    let mut first = true;
                    for (k, (c, f)) in cases.iter().enumerate() {
                        if f.is_empty() {
                            continue;
                        }
                        dx.emit(format!("{}if ({}) {{", if first { "" } else { "} else " }, text_is(&name, c)));
                        first = false;
                        dx.depth += 1;
                        dx.emit(format!("{place}.tag = {k}u;"));
                        self.json_fields(dx, item, &format!("{c}."), f, &format!("{place}.u.{c}."), e)?;
                        dx.depth -= 1;
                    }
                    dx.emit("} else {".into());
                    dx.emit("    lm_st = 2;".into());
                    dx.emit("    goto lm_out;".into());
                    dx.emit("}".into());
                    self.json_close(dx, '}', &i, &more);
                    dx.depth -= 1;
                    dx.emit("}".into());
                }
            }
            Ty::Game(GameTy::Enum(en)) => {
                let name = dx.fresh_kept("lumart_text");
                dx.emit(format!("LUMART_JR(lumart_jr_text(r, &{name}, file, line));"));
                let mut chain = String::new();
                for (v, n) in self.game_enum_values(en) {
                    chain.push_str(&format!("if ({}) {place} = (int32_t){n}; else ", text_is(&name, &v)));
                }
                dx.emit(format!("{chain}{{ lm_st = 2; goto lm_out; }}"));
            }
            other => {
                let what = format!("json.decode of {}", self.ty_text(other));
                return self.not_yet(dx, e, &what);
            }
        }
        Ok(())
    }

    /// A struct's fields (or an enum case's: `key` "Case." and `path` its union member) from a JSON object by name:
    /// other keys skipped, a key written twice its last value, a missing field its default, else the object does not
    /// read.
    fn json_fields(&mut self, dx: &mut Fx, item: ItemRef, key: &str, fields: &[(String, Option<Expr>)], path: &str, e: &Expr) -> Result<(), NotWritten> {
        let seen: Vec<String> = fields.iter().map(|_| dx.fresh_kept("uint8_t")).collect();
        for s in &seen {
            dx.emit(format!("{s} = 0;"));
        }
        let name = dx.fresh_kept("lumart_text");
        let is_text = dx.fresh_kept("uint8_t");
        let (i, more) = self.json_open(dx, '{');
        dx.emit("for (;;) {".into());
        dx.depth += 1;
        dx.emit(format!("LUMART_JR(lumart_jr_more(r, '}}', &{i}, &{more}));"));
        dx.emit(format!("if (!{more}) break;"));
        dx.emit(format!("LUMART_JR(lumart_jr_field(r, &{name}, &{is_text}, file, line));"));
        dx.emit(format!("if (!{is_text}) {{"));
        dx.emit("    LUMART_JR(lumart_jr_skip(r));".into());
        for (k, (f, _)) in fields.iter().enumerate() {
            let ft = self.declared_ty(item, &format!("{key}{f}"));
            let fc = self.c_type(&ft).ok_or_else(super::lost)?;
            let fp = format!("{path}{f}");
            dx.emit(format!("}} else if ({}) {{", text_is(&name, f)));
            dx.depth += 1;
            dx.emit(format!("if ({}) {{", seen[k]));
            dx.depth += 1;
            self.json_empty(dx, &fp, &fc);
            dx.depth -= 1;
            dx.emit("}".into());
            dx.emit(format!("{} = 1;", seen[k]));
            self.json_read(dx, &ft, &fp, e)?;
            dx.depth -= 1;
        }
        dx.emit("} else {".into());
        dx.emit("    LUMART_JR(lumart_jr_skip(r));".into());
        dx.emit("}".into());
        dx.depth -= 1;
        dx.emit("}".into());
        for (k, (f, default)) in fields.iter().enumerate() {
            dx.emit(format!("if (!{}) {{", seen[k]));
            dx.depth += 1;
            match default {
                Some(d) => {
                    // The default as a struct literal takes it: evaluated in the struct's module.
                    let ft = self.declared_ty(item, &format!("{key}{f}"));
                    let fc = self.c_type(&ft).ok_or_else(super::lost)?;
                    let (live, made) = (dx.live.len(), dx.made.len());
                    let saved = dx.module;
                    dx.module = item.module;
                    let v = self.expr(dx, d);
                    let from = self.ty(dx.module, d);
                    dx.module = saved;
                    let v = self.coerce(dx, &v?, &from, &ft, d.at)?;
                    dx.set(&format!("{path}{f}"), &v, &fc);
                    dx.release_since(live);
                    dx.recycle_since(made);
                }
                None => {
                    dx.emit("lm_st = 2;".into());
                    dx.emit("goto lm_out;".into());
                }
            }
            dx.depth -= 1;
            dx.emit("}".into());
        }
        Ok(())
    }

    /// A map's key: text as it is; a whole number as json.encode writes it; `bool` as "true"/"false"; an enum's case
    /// or a game enum's value by its name.
    fn json_key(&mut self, dx: &mut Fx, k: &Ty, key: &str, e: &Expr) -> Result<(), NotWritten> {
        match k {
            Ty::Text => dx.emit(format!("LUMART_JR(lumart_jr_key(r, &{key}, file, line));")),
            Ty::Int(s) if signed(*s) => {
                let (lo, hi) = int_range(*s);
                let v = dx.fresh("int64_t");
                dx.emit(format!("LUMART_JR(lumart_jr_key_i64(r, {lo}, {hi}, &{v}));"));
                dx.emit(format!("{key} = ({}){v};", scalar_c(*s)));
            }
            Ty::Int(s) => {
                let v = dx.fresh("uint64_t");
                dx.emit(format!("LUMART_JR(lumart_jr_key_u64(r, {}, &{v}));", uint_max(*s)));
                dx.emit(format!("{key} = ({}){v};", scalar_c(*s)));
            }
            Ty::Bool | Ty::Enum(_) | Ty::Game(GameTy::Enum(_)) => {
                let name = dx.fresh_kept("lumart_text");
                dx.emit(format!("LUMART_JR(lumart_jr_key(r, &{name}, file, line));"));
                let mut chain = String::new();
                match k {
                    Ty::Bool => chain.push_str(&format!("if ({}) {key} = 1; else if ({}) {key} = 0; else ", text_is(&name, "true"), text_is(&name, "false"))),
                    Ty::Enum(item) if self.enum_plain(*item) => {
                        let ItemKind::Enum(d) = &self.item(*item).kind else { return Err(super::lost()) };
                        for (n, c) in d.cases.iter().enumerate() {
                            chain.push_str(&format!("if ({}) {key}.tag = {n}u; else ", text_is(&name, &c.name.name)));
                        }
                    }
                    Ty::Game(GameTy::Enum(en)) => {
                        for (v, n) in self.game_enum_values(en) {
                            chain.push_str(&format!("if ({}) {key} = (int32_t){n}; else ", text_is(&name, &v)));
                        }
                    }
                    _ => return self.not_yet(dx, e, "a map key of an enum with values in JSON"),
                }
                dx.emit(format!("{chain}{{ lm_st = 2; goto lm_out; }}"));
                dx.emit(format!("lm_rel_lumart_text(&{name});"));
            }
            other => {
                let what = format!("json.decode of a map keyed by {}", self.ty_text(other));
                return self.not_yet(dx, e, &what);
            }
        }
        Ok(())
    }

    /// `[` or `{` read; the counter and flag of its elements.
    fn json_open(&mut self, dx: &mut Fx, open: char) -> (String, String) {
        let i = dx.fresh("uint64_t");
        let more = dx.fresh("uint8_t");
        dx.emit(format!("LUMART_JR(lumart_jr_open(r, '{open}'));"));
        (i, more)
    }

    /// One more element, which must be there.
    fn json_next(&mut self, dx: &mut Fx, close: char, i: &str, more: &str) {
        dx.emit(format!("LUMART_JR(lumart_jr_more(r, '{close}', &{i}, &{more}));"));
        dx.emit(format!("if (!{more}) {{ lm_st = 2; goto lm_out; }}"));
    }

    /// The close, which must come now.
    fn json_close(&mut self, dx: &mut Fx, close: char, i: &str, more: &str) {
        dx.emit(format!("LUMART_JR(lumart_jr_more(r, '{close}', &{i}, &{more}));"));
        dx.emit(format!("if ({more}) {{ lm_st = 2; goto lm_out; }}"));
    }

    /// `place` zero again (its counts given back).
    fn json_empty(&mut self, dx: &mut Fx, place: &str, ct: &str) {
        if counted(ct) {
            dx.emit(format!("lm_rel_{ct}(&{place});"));
        } else {
            dx.emit(format!("{place} = ({ct}){{0}};"));
        }
    }

    /// A game enum's values by name, in declared order (each name reads; json.encode writes a value's first).
    pub(super) fn game_enum_values(&self, en: &str) -> Vec<(String, i64)> {
        self.game.types.as_ref().and_then(|t| t.enums.get(en)).map(|e| e.values.iter().map(|(v, n)| (v.clone(), *n)).collect()).unwrap_or_default()
    }

    /// value_text's writer of a struct or an enum with values (`json`: as JSON, else as text), made once per type and
    /// form; its reports name the place of the call that writes (`file`, `line`, `src`).
    pub(super) fn text_writer(&mut self, fx: &Fx, t: &Ty, e: &Expr, json: bool) -> Result<String, NotWritten> {
        let key = format!("{t:?} {json}");
        if let Some(n) = self.writers.get(&key) {
            return Ok(n.clone());
        }
        let name = format!("lm_{}_{}", if json { "jw" } else { "tw" }, self.writers.len());
        self.writers.insert(key, name.clone());
        let ct = self.c_type(t).ok_or_else(super::lost)?;
        let head = format!("int32_t {name}(lumart_sb* b, const {ct}* v, const char* file, uint32_t line, const char* src)");
        self.protos.push(format!("{head};"));
        let mut dx = Fx::new(&fx.file, fx.module);
        dx.site = true;
        dx.emit("(void)file;".into());
        dx.emit("(void)line;".into());
        dx.emit("(void)src;".into());
        self.writer_inline = true;
        self.value_text(&mut dx, "(*b)", "(*v)", t, e, json)?;
        let text = dx.function(&head, e.at);
        self.json_code.push_str(&text);
        Ok(name)
    }

    // ------------------------------------------------------------ json.Value's methods

    /// `v.text() .int() .float() .bool() .list() .map() .get(k) .at(i)`: each an option.
    pub(super) fn json_method(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, name: &str, args: &[crate::ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let (file, line) = (c_string(&fx.file), e.at.line);
        let v = self.expr(fx, base)?;
        let v = fx.temp("lumart_text", &v);
        let mut a = Vec::new();
        for arg in args {
            let want = if name == "at" { Ty::Int(Scalar::I32) } else { Ty::Text };
            let x = self.expr(fx, &arg.value)?;
            let from = self.ty(m, &arg.value);
            let x = self.coerce(fx, &x, &from, &want, arg.value.at)?;
            let ct = self.c_type(&want).ok_or_else(super::lost)?;
            a.push(fx.temp(&ct, &x));
        }
        let ct = self.c_type(t).ok_or_else(super::lost)?;
        let r = fx.fresh(&ct);
        let some = fx.fresh("uint8_t");
        match name {
            "int" => fx.emit(format!("{r}.some = lumart_json_int({v}, &{r}.value);")),
            "float" => fx.emit(format!("{r}.some = lumart_json_float({v}, &{r}.value);")),
            "bool" => fx.emit(format!("{r}.some = lumart_json_bool({v}, &{r}.value);")),
            "text" | "list" | "map" | "get" | "at" => {
                let call = match name {
                    "text" => format!("lumart_json_text({v}, &{r}.value, &{some}, {file}, {line})"),
                    "list" => {
                        let d = self.desc(&Ty::Json).ok_or_else(super::lost)?;
                        format!("lumart_json_list({v}, &{r}.value, {d}, &{some}, {file}, {line})")
                    }
                    "map" => {
                        let kd = self.desc(&Ty::Text).ok_or_else(super::lost)?;
                        let vd = self.desc(&Ty::Json).ok_or_else(super::lost)?;
                        format!("lumart_json_map({v}, &{r}.value, {kd}, {vd}, &{some}, {file}, {line})")
                    }
                    "get" => format!("lumart_json_get({v}, {}, &{r}.value, &{some}, {file}, {line})", a[0]),
                    _ => format!("lumart_json_at({v}, (int64_t){}, &{r}.value, &{some}, {file}, {line})", a[0]),
                };
                fx.fail_if(&call);
                fx.emit(format!("{r}.some = {some};"));
            }
            _ => return self.not_yet(fx, e, &format!("json.Value.{name}()")),
        }
        Ok(r)
    }
}

/// Whether the text in C place `t` is `s`, as C.
fn text_is(t: &str, s: &str) -> String {
    format!("lumart_text_is({t}, {}, {}u)", c_string(s), s.len())
}
