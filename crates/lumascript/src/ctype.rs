//! The game's C++ type text, as the function rows and luma_types.json write it (clang's printing after the game's own
//! typedefs: `const TVec3f &`, `const f32 (*)[4]`, `void (LiveActor::*)()`), read into a tree the checker maps to
//! LumaScript types (docs/modding/language/language.md, section 7.2, docs/modding/language/compiled-form.md). A text this reader does not take is an error naming where,
//! never a guess: the checker then says the function cannot be called from LumaScript.

use std::fmt;

/// A C++ type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CType {
    /// A named type: a builtin (`unsigned int`, written in one canonical spelling) or a qualified name with its
    /// template arguments as written (`JGeometry::TVec3<f32>`); `tag` is an elaborated name's keyword as written
    /// (`struct LumaCall`), which names the same type.
    Named { name: String, konst: bool, volatile: bool, tag: Option<&'static str> },
    Ptr { to: Box<CType>, konst: bool },
    /// `T &`.
    Ref(Box<CType>),
    /// `T &&`.
    RRef(Box<CType>),
    /// `T[N]`, `T[]`.
    Array { of: Box<CType>, len: Option<u64> },
    /// A function type: its result, parameters, `...`, and the `const` of a member function's type.
    Func(Box<Signature>),
    /// `T (C::*)`: a pointer to a member of `class`.
    MemberPtr { class: String, to: Box<CType>, konst: bool },
}

/// A function's result and parameters. A constructor's or destructor's has no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub result: Option<CType>,
    pub params: Vec<CType>,
    pub variadic: bool,
    /// A `const` member function.
    pub konst: bool,
}

/// Why a type text does not read: the byte offset in it and what was there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CTypeError {
    pub at: usize,
    pub found: String,
}

impl fmt::Display for CTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.at, if self.found.is_empty() { "the end" } else { &self.found })
    }
}

impl std::error::Error for CTypeError {}

/// The builtin words, and the one spelling each combination is given (`unsigned` is `unsigned int`).
const BUILTIN_WORDS: [&str; 18] = [
    "void", "bool", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "wchar_t", "char8_t",
    "char16_t", "char32_t", "__int64", "__int32", "__int16", "__int8",
];

fn builtin_spelling(words: &[&str]) -> Option<String> {
    let has = |w: &str| words.contains(&w);
    let longs = words.iter().filter(|w| **w == "long").count();
    let count = |w: &str| words.iter().filter(|x| **x == w).count();
    if words.iter().any(|w| *w != "long" && count(w) > 1) || longs > 2 {
        return None;
    }
    // At most one of these, and `short` or `long` only with `int` (or `double` for one `long`).
    let kinds = ["void", "bool", "char", "float", "double", "wchar_t", "char8_t", "char16_t", "char32_t", "__int64", "__int32", "__int16", "__int8"];
    let base_words = words.iter().filter(|w| kinds.contains(w)).count() + usize::from(has("int"));
    if base_words > 1 || (has("signed") && has("unsigned")) || (has("short") && longs > 0) {
        return None;
    }
    if (has("short") || longs > 0) && words.iter().any(|w| kinds.contains(w) && !(*w == "double" && longs == 1)) {
        return None;
    }
    let unsigned = has("unsigned");
    let signed = has("signed");
    let base = if has("void") {
        "void"
    } else if has("bool") {
        "bool"
    } else if has("float") {
        "float"
    } else if has("double") {
        if longs == 1 { "long double" } else { "double" }
    } else if has("wchar_t") {
        "wchar_t"
    } else if has("char8_t") {
        "char8_t"
    } else if has("char16_t") {
        "char16_t"
    } else if has("char32_t") {
        "char32_t"
    } else if has("char") || has("__int8") {
        // `char` alone is its own type; `signed char` and `unsigned char` are two more.
        return Some(if unsigned { "unsigned char" } else if signed { "signed char" } else { "char" }.to_string());
    } else if has("short") || has("__int16") {
        "short"
    } else if longs == 2 || has("__int64") {
        "long long"
    } else if longs == 1 {
        "long"
    } else {
        "int"
    };
    let numeric = !matches!(base, "void" | "bool" | "float" | "double" | "long double" | "wchar_t" | "char8_t" | "char16_t" | "char32_t");
    if (unsigned || signed) && !numeric {
        return None;
    }
    Some(if unsigned { format!("unsigned {base}") } else { base.to_string() })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// A name (qualified, template arguments included, as written) or a word.
    Word(String),
    /// `C::*`.
    MemberStar(String),
    Num(u64),
    P(&'static str),
}

fn lex(text: &str) -> Result<Vec<(usize, Tok)>, CTypeError> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let bad = |at: usize| CTypeError { at, found: text[at..].chars().take(12).collect() };
    while i < b.len() {
        let c = b[i];
        if c == b' ' {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_alphabetic() || c == b'_' || (c == b':' && b.get(i + 1) == Some(&b':')) {
            // A name: words joined by `::`, each with its template arguments (balanced `<>`, kept as written).
            let mut name = String::new();
            loop {
                if text[i..].starts_with("::") {
                    if text[i + 2..].starts_with('*') {
                        // `C::*` ends the name: a pointer to member.
                        if name.is_empty() {
                            return Err(bad(start));
                        }
                        out.push((start, Tok::MemberStar(name)));
                        i += 3;
                        name = String::new();
                        break;
                    }
                    name.push_str("::");
                    i += 2;
                }
                let w = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if i == w {
                    return Err(bad(i));
                }
                name.push_str(&text[w..i]);
                if i < b.len() && b[i] == b'<' {
                    let open = i;
                    let mut depth = 0i32;
                    while i < b.len() {
                        match b[i] {
                            b'<' => depth += 1,
                            b'>' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    if depth != 0 {
                        return Err(bad(open));
                    }
                    name.push_str(&text[open..i]);
                }
                if !text[i..].starts_with("::") {
                    break;
                }
            }
            if !name.is_empty() {
                out.push((start, Tok::Word(name)));
            }
            continue;
        }
        if c.is_ascii_digit() {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let n = text[start..i].parse().map_err(|_| bad(start))?;
            out.push((start, Tok::Num(n)));
            continue;
        }
        let p = match c {
            b'*' => "*",
            b'&' if b.get(i + 1) == Some(&b'&') => "&&",
            b'&' => "&",
            b'(' => "(",
            b')' => ")",
            b'[' => "[",
            b']' => "]",
            b',' => ",",
            b'.' if text[i..].starts_with("...") => "...",
            _ => return Err(bad(i)),
        };
        i += p.len();
        out.push((start, Tok::P(p)));
    }
    Ok(out)
}

/// What a declarator does to the type it is given, in the order applied.
enum Op {
    Ptr(bool),
    Ref,
    RRef,
    MemberPtr(String, bool),
    Array(Option<u64>),
    Func(Vec<CType>, bool, bool),
}

struct Reader<'t> {
    toks: Vec<(usize, Tok)>,
    i: usize,
    text: &'t str,
}

impl Reader<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|(_, t)| t)
    }

    fn fail(&self) -> CTypeError {
        match self.toks.get(self.i) {
            Some((at, _)) => CTypeError { at: *at, found: self.text[*at..].chars().take(12).collect() },
            None => CTypeError { at: self.text.len(), found: String::new() },
        }
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }

    fn word_is(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x == w)
    }

    fn expect(&mut self, p: &str) -> Result<(), CTypeError> {
        if self.is(p) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.fail())
        }
    }

    /// `const` and `volatile` words here.
    fn cv(&mut self) -> (bool, bool) {
        let (mut konst, mut volatile) = (false, false);
        loop {
            if self.word_is("const") {
                konst = true;
            } else if self.word_is("volatile") {
                volatile = true;
            } else {
                return (konst, volatile);
            }
            self.i += 1;
        }
    }

    /// `const` after `*`, `C::*` or a member function's parameters. `volatile` there is refused, never dropped: the
    /// tree has no place for it, and a volatile pointer or member function does not cross.
    fn konst_only(&mut self) -> Result<bool, CTypeError> {
        let at = self.i;
        let (konst, volatile) = self.cv();
        if volatile {
            let word = (at..self.i).find(|&i| matches!(self.toks.get(i), Some((_, t)) if matches!(t, Tok::Word(w) if w == "volatile")));
            self.i = word.unwrap_or(at);
            return Err(self.fail());
        }
        Ok(konst)
    }

    /// A type-id: specifiers, then an abstract declarator.
    fn type_id(&mut self) -> Result<CType, CTypeError> {
        let base = self.specifiers()?;
        let ops = self.declarator()?;
        Ok(apply(base, ops))
    }

    fn specifiers(&mut self) -> Result<CType, CTypeError> {
        let (mut konst, mut volatile) = (false, false);
        let mut words: Vec<String> = Vec::new();
        let mut name: Option<String> = None;
        let mut tag = None;
        while let Some(Tok::Word(w)) = self.peek() {
            let w = w.clone();
            match w.as_str() {
                "const" => konst = true,
                "volatile" => volatile = true,
                "struct" | "class" | "union" | "enum" | "typename" if tag.is_none() && name.is_none() && words.is_empty() => {
                    tag = ["struct", "class", "union", "enum", "typename"].into_iter().find(|t| *t == w);
                }
                _ if BUILTIN_WORDS.contains(&w.as_str()) && name.is_none() => words.push(w),
                _ if name.is_none() && words.is_empty() => name = Some(w),
                _ => break,
            }
            self.i += 1;
        }
        let name = match (name, words.is_empty()) {
            (Some(n), true) => n,
            (None, false) => {
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                builtin_spelling(&words).ok_or_else(|| self.fail())?
            }
            _ => return Err(self.fail()),
        };
        // A keyword names a name, not a builtin (`struct int`).
        if tag.is_some() && !words.is_empty() {
            return Err(self.fail());
        }
        Ok(CType::Named { name, konst, volatile, tag })
    }

    /// An abstract declarator's operations, in the order they apply to the specifiers' type.
    fn declarator(&mut self) -> Result<Vec<Op>, CTypeError> {
        let mut ops = Vec::new();
        loop {
            if self.is("*") {
                self.i += 1;
                ops.push(Op::Ptr(self.konst_only()?));
            } else if self.is("&") {
                self.i += 1;
                ops.push(Op::Ref);
            } else if self.is("&&") {
                self.i += 1;
                ops.push(Op::RRef);
            } else if let Some(Tok::MemberStar(c)) = self.peek() {
                let c = c.clone();
                self.i += 1;
                ops.push(Op::MemberPtr(c, self.konst_only()?));
            } else {
                break;
            }
        }
        // `( inner )` is a group only when it holds a declarator, never a parameter list (`void (*)(int)`).
        let mut inner = Vec::new();
        if self.is("(") && matches!(self.toks.get(self.i + 1).map(|(_, t)| t), Some(Tok::P("*" | "&" | "&&" | "(")) | Some(Tok::MemberStar(_))) {
            self.i += 1;
            inner = self.declarator()?;
            self.expect(")")?;
        }
        let mut suffixes = Vec::new();
        loop {
            if self.is("[") {
                self.i += 1;
                let len = match self.peek() {
                    Some(Tok::Num(n)) => {
                        let n = *n;
                        self.i += 1;
                        Some(n)
                    }
                    _ => None,
                };
                self.expect("]")?;
                suffixes.push(Op::Array(len));
            } else if self.is("(") {
                let (params, variadic) = self.params()?;
                let konst = self.konst_only()?;
                suffixes.push(Op::Func(params, variadic, konst));
            } else {
                break;
            }
        }
        // `T * [3][4]`: the pointer first, then the last suffix (the innermost array), then the group's own.
        let mut all = ops;
        all.extend(suffixes.into_iter().rev());
        all.extend(inner);
        Ok(all)
    }

    fn params(&mut self) -> Result<(Vec<CType>, bool), CTypeError> {
        self.expect("(")?;
        let mut params = Vec::new();
        let mut variadic = false;
        if self.is(")") {
            self.i += 1;
            return Ok((params, false));
        }
        loop {
            if self.is("...") {
                self.i += 1;
                variadic = true;
                break;
            }
            params.push(self.type_id()?);
            if self.is(",") {
                self.i += 1;
            } else {
                break;
            }
        }
        self.expect(")")?;
        // `(void)` is no parameters.
        if params.len() == 1 && params[0] == (CType::Named { name: "void".into(), konst: false, volatile: false, tag: None }) {
            params.clear();
        }
        Ok((params, variadic))
    }
}

fn apply(base: CType, ops: Vec<Op>) -> CType {
    ops.into_iter().fold(base, |t, op| match op {
        Op::Ptr(konst) => CType::Ptr { to: Box::new(t), konst },
        Op::Ref => CType::Ref(Box::new(t)),
        Op::RRef => CType::RRef(Box::new(t)),
        Op::MemberPtr(class, konst) => CType::MemberPtr { class, to: Box::new(t), konst },
        Op::Array(len) => CType::Array { of: Box::new(t), len },
        Op::Func(params, variadic, konst) => CType::Func(Box::new(Signature { result: Some(t), params, variadic, konst })),
    })
}

/// Reads a field's type text from the types file: a type, and for a bit-field its width (`u32 : 3`, docs/modding/language/types.md).
pub fn parse_field_type(text: &str) -> Result<(CType, Option<u32>), CTypeError> {
    let Some((ty, bits)) = text.rsplit_once(" : ") else {
        return parse_type(text).map(|t| (t, None));
    };
    let at = ty.len() + 3;
    let bits = bits.parse::<u32>().ok().filter(|b| *b > 0 && bits.bytes().all(|c| c.is_ascii_digit())).ok_or_else(|| CTypeError { at, found: bits.into() })?;
    Ok((parse_type(ty)?, Some(bits)))
}

/// Reads one type text (`const TVec3f &`, `f32 (*)[4]`).
pub fn parse_type(text: &str) -> Result<CType, CTypeError> {
    let mut r = Reader { toks: lex(text)?, i: 0, text };
    let t = r.type_id()?;
    if r.i != r.toks.len() {
        return Err(r.fail());
    }
    Ok(t)
}

/// Reads a function row's signature: `<result>(<parameters>)` and a member function's `const`, the result left out for
/// a constructor or destructor (`(const char *)`). The parameter list is the last group of the text, so a result that
/// is itself a pointer to an array or a function reads whole (`const f32 (*)[4]()`).
pub fn parse_signature(text: &str) -> Result<Signature, CTypeError> {
    let trimmed = text.trim_end();
    // A volatile member function is refused at the word.
    if let Some(close) = trimmed.rfind(')') {
        if let Some(v) = trimmed[close..].find("volatile") {
            return Err(CTypeError { at: close + v, found: "volatile".into() });
        }
    }
    let (body, konst) = match trimmed.strip_suffix("const") {
        Some(b) if b.ends_with(')') || b.ends_with(") ") => (b.trim_end(), true),
        _ => (trimmed, false),
    };
    let bad = |at: usize| CTypeError { at, found: text[at.min(text.len())..].chars().take(12).collect() };
    if !body.ends_with(')') {
        return Err(bad(body.len()));
    }
    let mut depth = 0i32;
    let mut open = None;
    for (i, c) in body.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let open = open.ok_or_else(|| bad(0))?;
    let result_text = body[..open].trim();
    let result = if result_text.is_empty() { None } else { Some(parse_type(result_text)?) };
    let mut r = Reader { toks: lex(&body[open..]).map_err(|e| CTypeError { at: e.at + open, ..e })?, i: 0, text: &body[open..] };
    let (params, variadic) = r.params().map_err(|e| CTypeError { at: e.at + open, ..e })?;
    if r.i != r.toks.len() {
        return Err(bad(open));
    }
    Ok(Signature { result, params, variadic, konst })
}

impl fmt::Display for CType {
    /// The type in C++'s own spelling, as clang prints it, so a message shows the text the game's rows have.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&spell(self, Decl::default()).text)
    }
}

/// What stands where a name would be, and whether it begins with an array or parameter suffix (which the type before
/// it touches: `f32[4]`, `void(int)`) or with anything else (after a space: `char *`, `f32 (*)[4]`).
#[derive(Default)]
struct Decl {
    text: String,
    suffix: bool,
}

/// `t` with the declarator `inner`, spaced as clang prints it.
fn spell(t: &CType, inner: Decl) -> Decl {
    // A pointer-like declarator: its mark, its `const`, then what follows (`*const *`, `*const[12]`, `**`).
    let pointer = |mark: String, konst: bool, to: &CType, inner: Decl| {
        let mut text = mark;
        if konst {
            text.push_str("const");
            if !inner.text.is_empty() && !inner.suffix {
                text.push(' ');
            }
        }
        text.push_str(&inner.text);
        // Around an array or a function type it needs brackets: `f32 (*)[4]`.
        let text = if matches!(to, CType::Array { .. } | CType::Func(_)) { format!("({text})") } else { text };
        spell(to, Decl { text, suffix: false })
    };
    match t {
        CType::Named { name, konst, volatile, tag } => {
            let mut s = String::new();
            if *konst {
                s.push_str("const ");
            }
            if *volatile {
                s.push_str("volatile ");
            }
            if let Some(tag) = tag {
                s.push_str(tag);
                s.push(' ');
            }
            s.push_str(name);
            if !inner.text.is_empty() {
                if !inner.suffix {
                    s.push(' ');
                }
                s.push_str(&inner.text);
            }
            Decl { text: s, suffix: false }
        }
        CType::Ptr { to, konst } => pointer("*".into(), *konst, to, inner),
        CType::Ref(to) => pointer("&".into(), false, to, inner),
        CType::RRef(to) => pointer("&&".into(), false, to, inner),
        CType::MemberPtr { class, to, konst } => pointer(format!("{class}::*"), *konst, to, inner),
        CType::Array { of, len } => {
            let suffix = inner.text.is_empty() || inner.suffix;
            spell(of, Decl { text: format!("{}[{}]", inner.text, len.map(|n| n.to_string()).unwrap_or_default()), suffix })
        }
        CType::Func(sig) => {
            // A function type alone is spaced from its result (`void (int)`); after a declarator it touches it.
            let suffix = inner.suffix;
            let text = format!("{}({}){}", inner.text, params_text(sig), if sig.konst { " const" } else { "" });
            match &sig.result {
                Some(r) => spell(r, Decl { text, suffix }),
                None => Decl { text, suffix },
            }
        }
    }
}

fn params_text(sig: &Signature) -> String {
    let params: Vec<String> = sig.params.iter().map(|p| p.to_string()).chain(sig.variadic.then(|| "...".to_string())).collect();
    params.join(", ")
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // As a row writes it: the result's whole text, then the parameters (`const f32 (*)[4]()`).
        let result = self.result.as_ref().map(|r| r.to_string()).unwrap_or_default();
        write!(f, "{result}({}){}", params_text(self), if self.konst { " const" } else { "" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> CType {
        CType::Named { name: name.into(), konst: false, volatile: false, tag: None }
    }
    fn konst(name: &str) -> CType {
        CType::Named { name: name.into(), konst: true, volatile: false, tag: None }
    }
    fn ptr(t: CType) -> CType {
        CType::Ptr { to: Box::new(t), konst: false }
    }
    fn array(t: CType, n: u64) -> CType {
        CType::Array { of: Box::new(t), len: Some(n) }
    }

    #[test]
    fn each_form_reads_and_prints_back_as_written() {
        let cases: [(&str, CType); 14] = [
            ("const TVec3f &", CType::Ref(Box::new(konst("TVec3f")))),
            ("const char *", ptr(konst("char"))),
            ("char *const", CType::Ptr { to: Box::new(named("char")), konst: true }),
            ("JGeometry::TVec3<f32>", named("JGeometry::TVec3<f32>")),
            ("MR::Vector<MR::AssignableArray<int> >", named("MR::Vector<MR::AssignableArray<int> >")),
            ("const f32 (*)[4]", ptr(array(konst("f32"), 4))),
            ("f32[2][3]", array(array(named("f32"), 3), 2)),
            ("float (**)[4]", ptr(ptr(array(named("float"), 4)))),
            ("char (&)[16]", CType::Ref(Box::new(array(named("char"), 16)))),
            ("Vec2f[]", CType::Array { of: Box::new(named("Vec2f")), len: None }),
            ("unsigned int", named("unsigned int")),
            ("TVec3f &&", CType::RRef(Box::new(named("TVec3f")))),
            (
                "void (*)(NameObj *)",
                ptr(CType::Func(Box::new(Signature { result: Some(named("void")), params: vec![ptr(named("NameObj"))], variadic: false, konst: false }))),
            ),
            (
                "bool (SurfRayTutorial::*)() const",
                CType::MemberPtr {
                    class: "SurfRayTutorial".into(),
                    to: Box::new(CType::Func(Box::new(Signature { result: Some(named("bool")), params: vec![], variadic: false, konst: true }))),
                    konst: false,
                },
            ),
        ];
        for (text, want) in cases {
            let got = parse_type(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(got, want, "{text}");
            assert_eq!(got.to_string(), text, "{text} prints back");
        }
        // Spellings of one builtin are one name.
        assert_eq!(parse_type("unsigned").unwrap(), named("unsigned int"));
        assert_eq!(parse_type("long long unsigned int").unwrap(), named("unsigned long long"));
        assert_eq!(parse_type("signed char").unwrap(), named("signed char"));
        // A function type alone, as a typedef of one prints it; a bit-field's width.
        assert_eq!(parse_type("void (LayoutActor *, const char *)").unwrap().to_string(), "void (LayoutActor *, const char *)");
        assert_eq!(parse_field_type("unsigned int : 3").unwrap(), (named("unsigned int"), Some(3)));
        assert_eq!(parse_field_type("u32").unwrap(), (named("u32"), None));
        assert_eq!(parse_field_type("u32 : x").unwrap_err().at, 6);
        assert!(parse_field_type("u32 : 0").is_err());
        let foo = parse_type("struct Foo *").unwrap();
        assert_eq!(foo, ptr(CType::Named { name: "Foo".into(), konst: false, volatile: false, tag: Some("struct") }));
        assert_eq!(foo.to_string(), "struct Foo *");
    }

    #[test]
    fn signatures_read_with_their_result_parameters_and_const() {
        let s = parse_signature("const char *(u32) const").unwrap();
        assert_eq!((s.result, s.params.len(), s.konst), (Some(ptr(konst("char"))), 1, true));
        let ctor = parse_signature("(const ResTable *)").unwrap();
        assert_eq!((ctor.result, ctor.params), (None, vec![ptr(konst("ResTable"))]));
        // A result that is a pointer to an array: the last group is the parameters.
        let s = parse_signature("const f32 (*)[4]()").unwrap();
        assert_eq!((s.result, s.params.len()), (Some(ptr(array(konst("f32"), 4))), 0));
        let s = parse_signature("void(void (*)(lumascript::Program *))").unwrap();
        assert_eq!(s.params.len(), 1);
        assert_eq!(s.to_string(), "void(void (*)(lumascript::Program *))");
        let s = parse_signature("int(const char *, ...)").unwrap();
        assert!(s.variadic);
        assert_eq!(parse_signature("void(void)").unwrap().params, vec![]);
        for text in ["void", "void(", "f(int", "(int) extra", "void(int,)"] {
            assert!(parse_signature(text).is_err(), "{text} reads");
        }
    }

    #[test]
    fn a_text_that_does_not_read_says_where() {
        assert_eq!(parse_type("int $").unwrap_err(), CTypeError { at: 4, found: "$".into() });
        assert_eq!(parse_type("Foo<int").unwrap_err().at, 3);
        assert_eq!(parse_type("const").unwrap_err().at, 5);
        // A volatile pointer, member pointer or member function is refused at the word, never read without it.
        assert_eq!(parse_type("int * volatile").unwrap_err(), CTypeError { at: 6, found: "volatile".into() });
        assert_eq!(parse_type("int *const volatile").unwrap_err().at, 11);
        assert_eq!(parse_type("int C::* volatile").unwrap_err().at, 9);
        assert_eq!(parse_signature("void() volatile").unwrap_err().at, 7);
        // A volatile pointee is kept (the mapping refuses it).
        assert!(matches!(parse_type("volatile int *"), Ok(CType::Ptr { .. })));
        assert!(parse_type("unsigned float").is_err());
        assert!(parse_type("int int").is_err());
        assert!(parse_type("A B").is_err());
    }

    /// Every type text of a kernel build's rows and types file reads and prints back as written
    /// (`LUMA_ROWS=<luma folder> LUMA_TYPES=<luma_types.json> cargo test -p lumascript -- --ignored`).
    #[test]
    #[ignore]
    fn every_type_text_of_the_game_reads() {
        let mut texts: Vec<(String, bool)> = Vec::new();
        if let Ok(folder) = std::env::var("LUMA_ROWS") {
            let mut stack = vec![std::path::PathBuf::from(folder)];
            while let Some(dir) = stack.pop() {
                for e in std::fs::read_dir(&dir).unwrap().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "rows") {
                        for line in std::fs::read_to_string(&p).unwrap().lines() {
                            if let Some(sig) = line.split('\t').nth(1) {
                                texts.push((sig.to_string(), true));
                            }
                        }
                    }
                }
            }
        }
        if let Ok(file) = std::env::var("LUMA_TYPES") {
            let json = std::fs::read_to_string(file).unwrap();
            // Every "type" and "of" string, without a JSON reader of its own here.
            for key in ["\"type\": \"", "\"of\": \""] {
                for piece in json.split(key).skip(1) {
                    texts.push((piece[..piece.find('"').unwrap()].to_string(), false));
                }
            }
        }
        assert!(!texts.is_empty(), "LUMA_ROWS or LUMA_TYPES names nothing");
        texts.sort();
        texts.dedup();
        let mut bad = Vec::new();
        for (text, sig) in &texts {
            let printed = if *sig {
                parse_signature(text).map(|s| s.to_string())
            } else {
                parse_field_type(text).map(|(t, bits)| bits.map_or_else(|| t.to_string(), |b| format!("{t} : {b}")))
            };
            match printed {
                Ok(p) if p == *text => {}
                Ok(p) => bad.push(format!("{text} prints as {p}")),
                Err(e) => bad.push(format!("{text}: {e}")),
            }
        }
        eprintln!("{} texts", texts.len());
        assert!(bad.is_empty(), "{} of {}:\n{}", bad.len(), texts.len(), bad.join("\n"));
    }
}
