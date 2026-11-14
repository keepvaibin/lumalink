//! The game's places (docs/modding/language/compiled-form.md, section 3 and): a field is reached through its kind 1 row's slot (`lm_off_<n>`,
//! the offset the loader binds), a global through its kind 7 row's slot (`lm_gl_<n>`, its address), a bit-field
//! through LUMA_BITS_GET and LUMA_BITS_SET with its slot's position in bits. No offset, size or address is taken from
//! the self-description: a mod built against one build of the game loads on another with the same names and types.
//! A constant the game's module does not define is folded to its value.

use super::{c_string, scalar_c, scalar_name, signed, value_c, Ex, Fx, Gen, NotWritten};
use crate::ast::Expr;
use crate::check::{Resolved, Ty};
use crate::ctype::{self, CType};
use crate::game::{GameTy, Scalar};

/// Where a game place is, as C.
#[derive(Clone, Debug)]
pub(super) enum GamePlace {
    /// An lvalue of C type (a number, a vector, a pointer to an object).
    Lvalue(String),
    /// An object held inside another, or a global record: its address (a view; never written as a whole).
    Address(String),
    /// A bit-field: the object, its slot (position in bits), its width, the type it is read as.
    Bits { object: String, slot: String, width: u32, of: Scalar },
    /// An array: its first element's address (a `char*` expression), the C type of one innermost element,
    /// and the extents still to index, outermost first.
    Array { at: String, elem: CType, extents: Vec<u64> },
}

impl<'a> Gen<'a> {
    /// The slot of the field row "<record>.<field>" with its type text, added once.
    fn field_slot(&mut self, row: &str, ty: &str) -> String {
        let n = self.fields.len();
        self.fields.entry((row.to_string(), ty.to_string())).or_insert_with(|| format!("lm_off_{n}")).clone()
    }

    /// The slot of the class row of record `name` (kind 3: its number, for is_a and as_class).
    pub(super) fn class_slot(&mut self, name: &str) -> String {
        let n = self.classes.len();
        self.classes.entry(name.to_string()).or_insert_with(|| format!("lm_class_{n}")).clone()
    }

    /// Whether game record `a` is made from `b` (through its bases, any depth).
    pub(super) fn derives(&self, a: &str, b: &str) -> bool {
        let Some(types) = self.game.types.as_ref() else { return false };
        let mut queue = vec![a.to_string()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(r) = queue.pop() {
            if r == b {
                return true;
            }
            if !seen.insert(r.clone()) {
                continue;
            }
            if let Some((_, rec)) = types.record(&r) {
                for base in &rec.bases {
                    queue.push(types.record(&base.name).map_or(base.name.clone(), |(row, _)| row.to_string()));
                }
            }
        }
        false
    }

    /// The slot of the size row of game record `name` (kind 2: its size in bytes on this build of the game).
    pub(super) fn size_slot(&mut self, name: &str) -> String {
        let n = self.sizes.len();
        self.sizes.entry(name.to_string()).or_insert_with(|| format!("lm_size_{n}")).clone()
    }

    /// The size of one element of C type `elem` as C: the type's own for a number, a vector, a matrix, an enum or a
    /// pointer; a record's from its size row (never the self-description's number).
    fn elem_size(&mut self, fx: &mut Fx, e: &Expr, elem: &CType) -> Ex {
        if let CType::Ptr { .. } = elem {
            return Ok("sizeof(void*)".into());
        }
        match self.game.map(elem, crate::game::Place::Field, 0) {
            Ok(GameTy::Scalar(s)) => Ok(format!("sizeof({})", scalar_c(s))),
            Ok(GameTy::Value(v)) => Ok(format!("sizeof({})", value_c(v))),
            Ok(GameTy::Enum(name)) => match self.enum_storage(&name) {
                Some(s) => Ok(format!("sizeof({})", scalar_c(s))),
                None => self.not_yet(fx, e, "an array of a game enum whose storage does not read"),
            },
            Ok(GameTy::View { record, .. } | GameTy::Struct(record)) => Ok(format!("(size_t){}", self.size_slot(&record))),
            _ => self.not_yet(fx, e, "an array of this element type"),
        }
    }

    /// `base[index]` where `base` is a game array: the index checked against its extent (S0902), then the element's
    /// place, or the array of the extents left.
    fn game_index(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, index: &Expr) -> Result<GamePlace, NotWritten> {
        let Some(p) = self.game_place_of(fx, base) else { return self.not_yet(fx, e, "[ ] on this game place") };
        let GamePlace::Array { at, elem, extents } = p? else { return self.not_yet(fx, e, "[ ] on this game place") };
        let i = self.expr(fx, index)?;
        let i = fx.temp("int64_t", &format!("(int64_t){i}"));
        let n = extents[0];
        // A literal inside the extent needs no check.
        let literal = match &index.kind {
            crate::ast::ExprKind::Number(text) => {
                let (digits, radix) = super::expr::number_text(text);
                u128::from_str_radix(&digits, radix).ok()
            }
            _ => None,
        };
        if !literal.is_some_and(|v| v < u128::from(n)) {
            let file = c_string(&fx.file);
            let line = e.at.line;
            fx.emit(format!("if ({i} < 0 || {i} >= {n}LL) {{"));
            fx.depth += 1;
            let shown = fx.temp_text_int(&i, Scalar::I64);
            let length = c_string(&n.to_string());
            fx.fail_if(&format!("lumart_raise({file}, {line}, 902, 2, (const char* const[]){{{shown}, {length}}})"));
            fx.depth -= 1;
            fx.emit("}".into());
        }
        let size = self.elem_size(fx, e, &elem)?;
        let inner: u64 = extents[1..].iter().product();
        let scale = if inner == 1 { String::new() } else { format!(" * {inner}u") };
        let at = format!("({at} + (size_t){i}{scale} * {size})");
        if extents.len() > 1 {
            return Ok(GamePlace::Array { at, elem, extents: extents[1..].to_vec() });
        }
        let t = self.ty(fx.module, e);
        self.place_at(fx, e, &elem, &at, &t)
    }

    fn global_slot(&mut self, name: &str, ty: &str) -> String {
        let n = self.game_globals.len();
        self.game_globals.entry((name.to_string(), ty.to_string())).or_insert_with(|| format!("lm_gl_{n}")).clone()
    }

    /// A field `field` of game record `record` on `object` (the object's address, read already), as the script reads
    /// it (`of`: the field's own type).
    pub(super) fn game_field_place(&mut self, fx: &mut Fx, e: &Expr, object: String, of: &Ty, record: &str, field: &str) -> Result<GamePlace, NotWritten> {
        let found = self.game.types.as_ref().and_then(|t| t.record(record)).and_then(|(row, rec)| rec.fields.get(field).map(|ty| (row.to_string(), ty.clone())));
        let Some((row, ty_text)) = found else { return self.not_yet(fx, e, "a field the types file does not give") };
        let slot = self.field_slot(&format!("{row}.{field}"), &ty_text);
        let Ok((ct, width)) = ctype::parse_field_type(&ty_text) else { return self.not_yet(fx, e, "a field whose type does not read") };
        if let Some(width) = width {
            // LUMA_BITS_GET and LUMA_BITS_SET carry 32 bits.
            if width == 0 || width > 32 {
                return self.not_yet(fx, e, "a bit-field wider than 32 bits");
            }
            let of = match of {
                Ty::Bool => Scalar::Bool,
                Ty::Int(s) => *s,
                // An enum's bit-field is signed when its storage is: a value of an unsigned one at or above
                // 2^(width-1) reads as itself, and writing it fits.
                Ty::Game(GameTy::Enum(name)) => match self.enum_storage(name) {
                    Some(s) if signed(s) => Scalar::I32,
                    Some(_) => Scalar::U32,
                    None => return self.not_yet(fx, e, "a bit-field of a game enum whose storage does not read"),
                },
                _ => return self.not_yet(fx, e, "a bit-field of this type"),
            };
            return Ok(GamePlace::Bits { object, slot, width, of });
        }
        let at = format!("((char*){object} + {slot})");
        self.place_at(fx, e, &ct, &at, of)
    }

    /// A game global by name.
    pub(super) fn game_global_place(&mut self, fx: &mut Fx, e: &Expr, name: &str) -> Result<GamePlace, NotWritten> {
        let ty_text = self.game.types.as_ref().and_then(|t| t.global_facts.get(name)).map(|g| g.ty.clone());
        let Some(ty_text) = ty_text else { return self.not_yet(fx, e, "a global the types file does not give") };
        let Ok(ct) = ctype::parse_type(&ty_text) else { return self.not_yet(fx, e, "a global whose type does not read") };
        let slot = self.global_slot(name, &ty_text);
        let at = format!("((char*){slot})");
        let t = self.ty(fx.module, e);
        self.place_at(fx, e, &ct, &at, &t)
    }

    /// The place at address `at` (a `char*` expression) holding C type `ct`, as the script reads it (its checked type).
    fn place_at(&mut self, fx: &mut Fx, e: &Expr, ct: &CType, at: &str, t: &Ty) -> Result<GamePlace, NotWritten> {
        match (ct, t) {
            // A pointer: the pointer itself is the place (a view, or none when null).
            (CType::Ptr { .. }, Ty::Game(GameTy::View { .. })) => Ok(GamePlace::Lvalue(format!("(*(void**){at})"))),
            (CType::Ptr { .. }, Ty::Opt(x)) if super::is_view(x) => Ok(GamePlace::Lvalue(format!("(*(void**){at})"))),
            // `const char *` (read only to the script): a new text of its bytes when read, none for null.
            (CType::Ptr { .. }, Ty::Opt(x)) if **x == Ty::Text => {
                let oc = self.c_type(t).ok_or_else(super::lost)?;
                let s = fx.temp("const char*", &format!("(*(const char* const*){at})"));
                let tx = fx.fresh("lumart_text");
                fx.fail_if(&format!("lumart_text_from_game(&{tx}, {s}, {}, {})", c_string(&fx.file), e.at.line));
                Ok(GamePlace::Lvalue(fx.temp(&oc, &format!("({oc}){{{s} != 0, {tx}}}"))))
            }
            // An object or a plain record inside: its address (a plain record's fields are read in place along a
            // chain of places; one kept in a local is a copy, D9.2).
            (CType::Named { .. }, Ty::Game(GameTy::View { .. } | GameTy::Struct(_))) => Ok(GamePlace::Address(format!("((void*){at})"))),
            (CType::Named { .. }, Ty::Bool) => Ok(GamePlace::Lvalue(format!("(*(uint8_t*){at})"))),
            (CType::Named { .. }, Ty::Int(s) | Ty::Float(s)) => Ok(GamePlace::Lvalue(format!("(*({}*){at})", scalar_c(*s)))),
            (CType::Named { .. }, Ty::Value(v)) if !super::components(*v).is_empty() => Ok(GamePlace::Lvalue(format!("(*({}*){at})", value_c(*v)))),
            // The game's matrix (`Mtx`, `TPos3f`, `float[3][4]`): twelve floats in rows of four.
            (_, Ty::Value(crate::game::Value::Mat34)) => Ok(GamePlace::Lvalue(format!("(*(lm_mat34*){at})"))),
            // An array: its element's C type (a field's text is the element's; a global's is the whole array's).
            (_, Ty::Game(GameTy::Array { extents, .. })) => {
                let mut elem = ct.clone();
                while let CType::Array { of, .. } = elem {
                    elem = *of;
                }
                Ok(GamePlace::Array { at: at.to_string(), elem, extents: extents.clone() })
            }
            // An enum: stored as its underlying integer; the script holds it as int32_t.
            (CType::Named { .. }, Ty::Game(GameTy::Enum(name))) => {
                let under = self.enum_storage(name);
                match under {
                    Some(Scalar::I32 | Scalar::U32) => Ok(GamePlace::Lvalue(format!("(*(int32_t*){at})"))),
                    _ => self.not_yet(fx, e, "a game enum stored in other than 32 bits"),
                }
            }
            _ => {
                let what = format!("a game place of type {}", self.ty_text(t));
                self.not_yet(fx, e, &what)
            }
        }
    }

    fn enum_storage(&self, name: &str) -> Option<Scalar> {
        let e = self.game.types.as_ref()?.enums.get(name)?;
        let t = ctype::parse_type(&e.underlying).ok()?;
        match self.game.map(&t, crate::game::Place::Field, 0).ok() {
            Some(GameTy::Scalar(s)) => Some(s),
            _ => None,
        }
    }

    /// A game place read as a value.
    pub(super) fn game_read(&mut self, fx: &mut Fx, p: GamePlace) -> Ex {
        Ok(match p {
            GamePlace::Lvalue(l) | GamePlace::Address(l) => l,
            GamePlace::Array { .. } => return Err(super::lost()),
            GamePlace::Bits { object, slot, width, of } => {
                let raw = format!("LUMA_BITS_GET({object}, {slot}, {width}u)");
                match of {
                    Scalar::Bool => format!("((uint8_t)({raw} != 0))"),
                    s if signed(s) && width < 32 => {
                        // Sign-extended by its declared type (addition 5).
                        let shift = 32 - width;
                        fx.temp(scalar_c(s), &format!("({})(((int32_t)({raw} << {shift}u)) >> {shift})", scalar_c(s)))
                    }
                    s => fx.temp(scalar_c(s), &format!("({}){raw}", scalar_c(s))),
                }
            }
        })
    }

    /// Writes C value `v` into a bit-field; a value that does not fit its width is a script error (addition 5).
    pub(super) fn bits_write(&mut self, fx: &mut Fx, e: &Expr, object: &str, slot: &str, width: u32, of: Scalar, v: &str) {
        let file = c_string(&fx.file);
        let line = e.at.line;
        let ct = scalar_c(of);
        let tmp = fx.temp(ct, v);
        let fits = if of == Scalar::Bool {
            None
        } else if signed(of) {
            let lo = -(1i64 << (width - 1));
            let hi = (1i64 << (width - 1)) - 1;
            Some(format!("(int64_t){tmp} >= {lo}LL && (int64_t){tmp} <= {hi}LL"))
        } else {
            let hi = if width >= 64 { u64::MAX } else { (1u64 << width) - 1 };
            Some(format!("(uint64_t){tmp} <= {hi}ull"))
        };
        if let Some(fits) = fits {
            fx.emit(format!("if (!({fits})) {{"));
            fx.depth += 1;
            let shown = fx.temp_text_int(&tmp, of);
            let ty = c_string(&format!("{} : {width}", scalar_name(of)));
            fx.fail_if(&format!("lumart_raise({file}, {line}, 908, 2, (const char* const[]){{{shown}, {ty}}})"));
            fx.depth -= 1;
            fx.emit("}".into());
        }
        fx.emit(format!("LUMA_BITS_SET({object}, {slot}, {width}u, (uint32_t){tmp});"));
    }

    /// A folded game constant of the expression's type.
    pub(super) fn game_constant(&mut self, fx: &mut Fx, e: &Expr, value: &str) -> Ex {
        let t = self.ty(fx.module, e);
        let text = value.trim();
        let int = if let Some(h) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            i128::from_str_radix(h, 16).ok()
        } else if let Some(h) = text.strip_prefix("-0x") {
            i128::from_str_radix(h, 16).ok().map(|v| -v)
        } else {
            text.parse::<i128>().ok()
        };
        match (&t, int) {
            (Ty::Int(s), Some(v)) => Ok(int_literal(*s, v)),
            (Ty::Game(GameTy::Enum(_)), Some(v)) => Ok(int_literal(Scalar::I32, v)),
            (Ty::Bool, Some(v)) => Ok(format!("((uint8_t){})", u8::from(v != 0))),
            (Ty::Float(Scalar::F64), _) => match text.trim_end_matches(['f', 'F']).parse::<f64>() {
                Ok(v) => Ok(super::f64_c(v)),
                Err(_) => self.not_yet(fx, e, "a game constant whose value does not read"),
            },
            (Ty::Float(_), _) => match text.trim_end_matches(['f', 'F']).parse::<f32>() {
                Ok(v) => Ok(super::f32_c(v)),
                Err(_) => self.not_yet(fx, e, "a game constant whose value does not read"),
            },
            _ => self.not_yet(fx, e, "a game constant of this type"),
        }
    }

    /// The address of the bytes of the game record copy `base` (a value of the script, type `record`): read, a counted
    /// copy held for the statement; written, the variable's own block (copy on write), its check made again before the
    /// store as a container's is (the address is a temporary the check sets). A copy that has no bytes yet (a zero
    /// value) gets its size row's bytes, zeroed.
    fn copy_address(&mut self, fx: &mut Fx, base: &Expr, record: &str, writing: bool) -> Result<String, NotWritten> {
        let size = self.size_slot(record);
        let (file, line) = (c_string(&fx.file), base.at.line);
        let b = if writing {
            self.place(fx, base)?
        } else {
            let v = self.expr(fx, base)?;
            fx.temp("lumart_bytes", &v)
        };
        let p = fx.fresh("uint8_t*");
        let from = fx.lines.len();
        fx.fail_if(&format!("({p} = lumart_record_at(&{b}, {size}, {}u, {file}, {line})) == 0", u8::from(writing)));
        if writing {
            let checks = fx.lines[from..].to_vec();
            fx.rechecks.push(checks);
        }
        Ok(format!("((void*){p})"))
    }

    /// A copy of the game record at address `a` (a game place), the script's own: its size row's bytes.
    pub(super) fn record_copy(&mut self, fx: &mut Fx, line: u32, a: &str, record: &str) -> String {
        let size = self.size_slot(record);
        let r = fx.fresh("lumart_bytes");
        fx.fail_if(&format!("lumart_record_copy(&{r}, (const void*)({a}), {size}, {}, {line})", c_string(&fx.file)));
        r
    }

    /// A game place an expression names, when it names one.
    pub(super) fn game_place_of(&mut self, fx: &mut Fx, e: &Expr) -> Option<Result<GamePlace, NotWritten>> {
        let m = fx.module;
        // A write's mark is this call's alone: what it reads on the way (an object, an index) is read as values.
        let writing = std::mem::replace(&mut fx.writing, false);
        if let crate::ast::ExprKind::Index { base, index } = &e.kind {
            if !matches!(self.ty(m, base), Ty::Game(GameTy::Array { .. })) {
                return None;
            }
            fx.writing = writing;
            return Some(self.game_index(fx, e, base, index));
        }
        match self.resolved(m, e)? {
            Resolved::GameField { record, field } => {
                let (record, field) = (record.clone(), field.clone());
                let object = match &e.kind {
                    crate::ast::ExprKind::Member { base, optional: false, .. } => match self.ty(m, base) {
                        // A mod class's object is its game object (docs/modding/language/classes.md).
                        Ty::Game(GameTy::View { .. }) | Ty::Class(_) => match self.expr(fx, base) {
                            Ok(v) => fx.temp("void*", &v),
                            Err(x) => return Some(Err(x)),
                        },
                        // A plain record along a chain of places: its address. One the script holds is its copy:
                        // the copy's bytes, made the variable's own for a write.
                        Ty::Game(GameTy::Struct(rec)) => {
                            let rec = rec.clone();
                            fx.writing = writing;
                            match self.game_place_of(fx, base) {
                                Some(Ok(GamePlace::Address(a))) => a,
                                Some(Err(x)) => return Some(Err(x)),
                                Some(Ok(_)) => return Some(self.not_yet(fx, e, "a field of this game place")),
                                None => match self.copy_address(fx, base, &rec, writing) {
                                    Ok(a) => a,
                                    Err(x) => return Some(Err(x)),
                                },
                            }
                        }
                        _ => return Some(self.not_yet(fx, e, "a field of this value")),
                    },
                    // `a?.f` reads its field where the optional's value is known (expr.rs).
                    crate::ast::ExprKind::Member { optional: true, .. } => return None,
                    // A field named alone: `self`'s, in a hook.
                    _ if fx.hook.is_some() => "call->args[0]".to_string(),
                    _ => return Some(self.not_yet(fx, e, "a field of self outside a hook")),
                };
                let of = self.ty(m, e);
                Some(self.game_field_place(fx, e, object, &of, &record, &field))
            }
            Resolved::GameGlobal(name) => {
                let name = name.clone();
                Some(self.game_global_place(fx, e, &name))
            }
            _ => None,
        }
    }
}

/// An integer of type `s` as C, exact for every value of the type (the minimum of a signed type included).
fn int_literal(s: Scalar, v: i128) -> String {
    let ct = scalar_c(s);
    if v >= 0 {
        format!("(({ct})0x{v:x}ull)")
    } else {
        // -(m) where m = |v| - 1 fits: (-(m) - 1).
        let m = (-v - 1) as u128;
        format!("(({ct})(-(int64_t)0x{m:x}ll - 1))")
    }
}
