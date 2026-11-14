//! A service's header in the registry's one form (docs/modding/services/registry.md, section 3) read into the
//! service's LumaScript face (docs/modding/language/language.md). Two steps: [`Header::read`] takes the C text (only what the form
//! allows: `#define`s, named and anonymous enums, structs, `typedef`s of a type or a function pointer, and service
//! tables of function pointers, each member's comment kept), and [`Face::of`] maps one catalogued service of it to
//! LumaScript. Anything the form does not allow is an error naming the line; a function the face
//! cannot type is kept with the reason ([`Shape::NotCallable`]), never guessed.

use std::collections::BTreeMap;
use std::fmt;

/// A header's text could not be read in the one form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderError {
    pub line: u32,
    pub why: String,
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.why)
    }
}

/// A C type as the form writes it: a base name (`uint32_t`, `LumaTimeNow`, `unsigned int`), its `const`, and how many
/// pointers to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CT {
    pub base: String,
    pub konst: bool,
    pub ptrs: u8,
}

/// A parameter or field: its type and name; a field may be an array (`char game_id[8]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub ty: CT,
    pub name: String,
    pub array: Option<u64>,
    /// The comment after it on its line (a field's), or before it (a table member's).
    pub comment: String,
}

/// A function pointer: a table's member, or a `typedef`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnPtr {
    pub name: String,
    pub result: CT,
    pub params: Vec<Decl>,
    /// The comment before the member.
    pub comment: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CEnum {
    /// None for `enum { ... };`.
    pub name: Option<String>,
    /// (C name, value, the comment after it).
    pub cases: Vec<(String, i64, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Member {
    Field(Decl),
    Fn(FnPtr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CStruct {
    pub name: String,
    pub members: Vec<Member>,
}

/// One header, read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    /// `#define NAME <whole number>`.
    pub numbers: BTreeMap<String, u64>,
    /// `#define NAME "text"`.
    pub texts: BTreeMap<String, String>,
    pub enums: Vec<CEnum>,
    pub structs: Vec<CStruct>,
    /// `typedef T Name;` (Name, T).
    pub aliases: Vec<(String, CT)>,
    /// `typedef R (*Name)(...);`.
    pub fn_types: Vec<FnPtr>,
    /// `#include "x.h"`: the headers it includes by quotes (a shared header such as luma_content.h).
    pub includes: Vec<String>,
    /// The comments before its first line of code (the licence line, then what the service is), joined.
    pub lead: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Num(String),
    Str(String),
    Punct(char),
    Comment(String),
    Directive(String),
}

fn lex(text: &str) -> Result<Vec<(Tok, u32)>, HeaderError> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut line_start) = (0usize, 1u32, true);
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let at = line;
        if c == '#' && line_start {
            // To the line's end, a backslash at the end going on. A comment in it is no part of it: a block comment
            // runs to its `*/` over as many lines as it takes (luma_assets.h's `#define X 1u /* ... ` on four lines),
            // a line comment to the line's end; a string is kept whole (`"a//b"`).
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != '\n' {
                if b[i] == '\\' && b.get(i + 1) == Some(&'\n') {
                    i += 2;
                    line += 1;
                    s.push(' ');
                    continue;
                }
                if b[i] == '"' {
                    s.push('"');
                    i += 1;
                    while i < b.len() && b[i] != '"' && b[i] != '\n' {
                        s.push(b[i]);
                        i += 1;
                    }
                    if b.get(i) == Some(&'"') {
                        s.push('"');
                        i += 1;
                    }
                    continue;
                }
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    i += 2;
                    loop {
                        if i + 1 >= b.len() {
                            return Err(HeaderError { line: at, why: "a comment that does not end".into() });
                        }
                        if b[i] == '*' && b[i + 1] == '/' {
                            i += 2;
                            break;
                        }
                        if b[i] == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    s.push(' ');
                    continue;
                }
                if b[i] == '/' && b.get(i + 1) == Some(&'/') {
                    while i < b.len() && b[i] != '\n' {
                        i += 1;
                    }
                    break;
                }
                s.push(b[i]);
                i += 1;
            }
            out.push((Tok::Directive(s.trim().to_string()), at));
            continue;
        }
        line_start = false;
        if c == '/' && b.get(i + 1) == Some(&'*') {
            let mut s = String::new();
            i += 2;
            loop {
                if i + 1 >= b.len() {
                    return Err(HeaderError { line: at, why: "a comment that does not end".into() });
                }
                if b[i] == '*' && b[i + 1] == '/' {
                    i += 2;
                    break;
                }
                if b[i] == '\n' {
                    line += 1;
                }
                s.push(b[i]);
                i += 1;
            }
            // One space for each run of white space, so a comment reads as one line.
            out.push((Tok::Comment(s.split_whitespace().collect::<Vec<_>>().join(" ")), at));
            continue;
        }
        if c == '/' && b.get(i + 1) == Some(&'/') {
            let mut s = String::new();
            i += 2;
            while i < b.len() && b[i] != '\n' {
                s.push(b[i]);
                i += 1;
            }
            out.push((Tok::Comment(s.trim().to_string()), at));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                s.push(b[i]);
                i += 1;
            }
            out.push((Tok::Ident(s), at));
            continue;
        }
        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                s.push(b[i]);
                i += 1;
            }
            out.push((Tok::Num(s), at));
            continue;
        }
        if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\n' {
                    return Err(HeaderError { line: at, why: "a string that does not end on its line".into() });
                }
                s.push(b[i]);
                i += 1;
            }
            i += 1;
            out.push((Tok::Str(s), at));
            continue;
        }
        if "{}()[];,*=-+<>|".contains(c) {
            out.push((Tok::Punct(c), at));
            i += 1;
            continue;
        }
        return Err(HeaderError { line: at, why: format!("'{c}' is not in the form") });
    }
    Ok(out)
}

/// A whole number as C writes it: decimal or 0x hex, with u/l suffixes.
/// An enum value's constant expression: `|` over `<<` over `+ -` over unary `-` and `( )`, in i64, overflow refused.
fn fold_or(p: &mut P, case: &str) -> Result<i64, HeaderError> {
    let mut v = fold_shift(p, case)?;
    while p.is_punct('|') {
        p.next_code();
        v |= fold_shift(p, case)?;
    }
    Ok(v)
}

fn fold_shift(p: &mut P, case: &str) -> Result<i64, HeaderError> {
    let mut v = fold_add(p, case)?;
    while p.is_punct('<') {
        p.next_code();
        if !p.is_punct('<') {
            return p.err(format!("{case}'s value: `<<` expected"));
        }
        p.next_code();
        let k = fold_add(p, case)?;
        v = match u32::try_from(k).ok().filter(|k| *k < 63).and_then(|k| v.checked_mul(1i64 << k)) {
            Some(x) => x,
            None => return p.err(format!("{case}'s value overflows")),
        };
    }
    Ok(v)
}

fn fold_add(p: &mut P, case: &str) -> Result<i64, HeaderError> {
    let mut v = fold_mul(p, case)?;
    loop {
        let minus = if p.is_punct('+') {
            false
        } else if p.is_punct('-') {
            true
        } else {
            return Ok(v);
        };
        p.next_code();
        let r = fold_mul(p, case)?;
        v = match if minus { v.checked_sub(r) } else { v.checked_add(r) } {
            Some(x) => x,
            None => return p.err(format!("{case}'s value overflows")),
        };
    }
}

/// `*` (a define's `(256ull * 1024ull * 1024ull)`).
fn fold_mul(p: &mut P, case: &str) -> Result<i64, HeaderError> {
    let mut v = fold_unary(p, case)?;
    while p.is_punct('*') {
        p.next_code();
        let r = fold_unary(p, case)?;
        v = match v.checked_mul(r) {
            Some(x) => x,
            None => return p.err(format!("{case}'s value overflows")),
        };
    }
    Ok(v)
}

/// A `#define`'s value folded as an enum value is (`(256ull * 1024ull * 1024ull)`, `(1u << 4) + 2`): None when it is
/// no whole constant expression of numbers alone, or below 0.
fn define_value(value: &str) -> Option<u64> {
    let toks = lex(value).ok()?;
    if toks.is_empty() {
        return None;
    }
    let mut p = P { toks, i: 0 };
    let v = fold_or(&mut p, "a define").ok()?;
    (p.peek_code().is_none()).then_some(v).and_then(|v| u64::try_from(v).ok())
}

/// An array's length: a whole number, or a `#define` of the header above it (`names[LUMA_MODLOG_ARGS]`).
fn length_of(t: &Tok, numbers: &BTreeMap<String, u64>) -> Option<u64> {
    match t {
        Tok::Num(n) => whole(n),
        Tok::Ident(n) => numbers.get(n).copied(),
        _ => None,
    }
}

fn fold_unary(p: &mut P, case: &str) -> Result<i64, HeaderError> {
    if p.is_punct('-') {
        p.next_code();
        return match fold_unary(p, case)?.checked_neg() {
            Some(x) => Ok(x),
            None => p.err(format!("{case}'s value overflows")),
        };
    }
    if p.is_punct('(') {
        p.next_code();
        let v = fold_or(p, case)?;
        p.punct(')')?;
        return Ok(v);
    }
    let Some(Tok::Num(n)) = p.next_code() else { return p.err(format!("{case}'s value is no whole number")) };
    match whole(&n).and_then(|v| i64::try_from(v).ok()) {
        Some(v) => Ok(v),
        None => p.err(format!("{case}'s value {n}")),
    }
}

fn whole(text: &str) -> Option<u64> {
    let t = text.trim_end_matches(['u', 'U', 'l', 'L']);
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => t.parse().ok(),
    }
}

struct P {
    toks: Vec<(Tok, u32)>,
    i: usize,
}

impl P {
    fn line(&self) -> u32 {
        self.toks.get(self.i).or(self.toks.last()).map_or(0, |t| t.1)
    }
    fn err<T>(&self, why: impl Into<String>) -> Result<T, HeaderError> {
        Err(HeaderError { line: self.line(), why: why.into() })
    }
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.0)
    }
    /// The next token that is no comment, its comments skipped.
    fn peek_code(&self) -> Option<&Tok> {
        self.toks[self.i..].iter().map(|t| &t.0).find(|t| !matches!(t, Tok::Comment(_)))
    }
    /// Comments here, joined, and moved past.
    fn comments(&mut self) -> String {
        let mut s: Vec<String> = Vec::new();
        while let Some(Tok::Comment(c)) = self.peek() {
            s.push(c.clone());
            self.i += 1;
        }
        s.join(" ")
    }
    /// The comments here that start on `line`, moved past (a field's own comment after it).
    fn trailing(&mut self, line: u32) -> String {
        let mut s: Vec<String> = Vec::new();
        while let Some((Tok::Comment(c), l)) = self.toks.get(self.i) {
            if *l != line {
                break;
            }
            s.push(c.clone());
            self.i += 1;
        }
        s.join(" ")
    }
    fn next_code(&mut self) -> Option<Tok> {
        self.comments();
        let t = self.toks.get(self.i).map(|t| t.0.clone());
        self.i += 1;
        t
    }
    fn punct(&mut self, c: char) -> Result<(), HeaderError> {
        match self.next_code() {
            Some(Tok::Punct(p)) if p == c => Ok(()),
            other => {
                self.i -= 1;
                self.err(format!("'{c}' expected, found {}", shown(other.as_ref())))
            }
        }
    }
    fn ident(&mut self) -> Result<String, HeaderError> {
        match self.next_code() {
            Some(Tok::Ident(s)) => Ok(s),
            other => {
                self.i -= 1;
                self.err(format!("a name expected, found {}", shown(other.as_ref())))
            }
        }
    }
    fn is_punct(&self, c: char) -> bool {
        matches!(self.peek_code(), Some(Tok::Punct(p)) if *p == c)
    }
}

fn shown(t: Option<&Tok>) -> String {
    match t {
        None => "the end".into(),
        Some(Tok::Ident(s) | Tok::Num(s)) => format!("\"{s}\""),
        Some(Tok::Str(s)) => format!("\"\\\"{s}\\\"\""),
        Some(Tok::Punct(c)) => format!("'{c}'"),
        Some(Tok::Comment(_)) => "a comment".into(),
        Some(Tok::Directive(d)) => format!("#{d}"),
    }
}

/// The words of a C type before a name: `const`, base words (`unsigned int`, `struct X`), `*`s.
fn ct_of(words: &[Tok]) -> Option<CT> {
    let (mut base, mut konst, mut ptrs) = (Vec::new(), false, 0u8);
    for w in words {
        match w {
            Tok::Ident(s) if s == "const" => {
                // `T* const` is a const pointer, the same to a caller; only a const before the first `*` is the pointee's.
                if ptrs == 0 {
                    konst = true;
                }
            }
            Tok::Ident(s) if s == "struct" || s == "enum" => {}
            Tok::Ident(s) if ptrs == 0 => base.push(s.clone()),
            Tok::Punct('*') => ptrs += 1,
            _ => return None,
        }
    }
    (!base.is_empty()).then(|| CT { base: base.join(" "), konst, ptrs })
}

impl Header {
    /// Reads a header in the one form.
    pub fn read(text: &str) -> Result<Header, HeaderError> {
        let mut p = P { toks: lex(text)?, i: 0 };
        let mut h = Header::default();
        let mut lead = Vec::new();
        while let Some((Tok::Comment(c), _)) = p.toks.get(p.i) {
            lead.push(c.clone());
            p.i += 1;
        }
        h.lead = lead.join(" ");
        loop {
            p.comments();
            let Some(t) = p.peek().cloned() else { break };
            match t {
                Tok::Directive(d) => {
                    p.i += 1;
                    let mut parts = d.splitn(3, char::is_whitespace);
                    let first = parts.next();
                    if first == Some("include") {
                        let rest = d["include".len()..].trim();
                        if let Some(name) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
                            h.includes.push(name.to_string());
                        }
                    }
                    if first == Some("define") {
                        if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                            let value = value.trim();
                            if let Some(n) = whole(value).or_else(|| define_value(value)) {
                                h.numbers.insert(name.to_string(), n);
                            } else if let Some(s) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                                h.texts.insert(name.to_string(), s.to_string());
                            }
                        }
                    }
                }
                Tok::Ident(w) if w == "extern" => {
                    // `extern "C" {`
                    p.i += 1;
                    if !matches!(p.next_code(), Some(Tok::Str(s)) if s == "C") {
                        return p.err("extern other than \"C\"");
                    }
                    p.punct('{')?;
                }
                Tok::Punct('}') => p.i += 1,
                Tok::Ident(w) if w == "enum" => {
                    p.i += 1;
                    let e = Self::enum_body(&mut p, None)?;
                    p.punct(';')?;
                    h.enums.push(e);
                }
                Tok::Ident(w) if w == "typedef" => {
                    p.i += 1;
                    Self::typedef(&mut p, &mut h)?;
                }
                other => return p.err(format!("{} is not in the form", shown(Some(&other)))),
            }
        }
        Ok(h)
    }

    fn typedef(p: &mut P, h: &mut Header) -> Result<(), HeaderError> {
        match p.peek_code() {
            Some(Tok::Ident(w)) if w == "enum" => {
                p.next_code();
                let tag = p.ident()?;
                let e = Self::enum_body(p, Some(tag.clone()))?;
                Self::closing_name(p, &tag)?;
                h.enums.push(e);
            }
            Some(Tok::Ident(_)) if Self::struct_ahead(p) => {
                p.next_code();
                let tag = p.ident()?;
                let members = Self::struct_body(p, &h.numbers)?;
                Self::closing_name(p, &tag)?;
                h.structs.push(CStruct { name: tag, members });
            }
            _ => {
                let line = p.line();
                let mut words = Vec::new();
                while !p.is_punct(';') && !p.is_punct('(') {
                    match p.next_code() {
                        Some(t) => words.push(t),
                        None => return p.err("a typedef that does not end"),
                    }
                }
                if p.is_punct('(') {
                    let f = Self::fn_ptr(p, words, String::new(), line, &h.numbers)?;
                    h.fn_types.push(f);
                } else {
                    let Some(Tok::Ident(name)) = words.pop() else { return p.err("a typedef with no name") };
                    let Some(ct) = ct_of(&words) else { return p.err(format!("the type of {name} is not in the form")) };
                    h.aliases.push((name, ct));
                }
                p.punct(';')?;
            }
        }
        Ok(())
    }

    /// `typedef struct X {` with comments between.
    fn struct_ahead(p: &P) -> bool {
        let code: Vec<&Tok> = p.toks[p.i..].iter().map(|t| &t.0).filter(|t| !matches!(t, Tok::Comment(_))).take(3).collect();
        matches!(code.as_slice(), [Tok::Ident(s), Tok::Ident(_), Tok::Punct('{')] if s == "struct")
    }

    fn closing_name(p: &mut P, tag: &str) -> Result<(), HeaderError> {
        let name = p.ident()?;
        if name != tag {
            return p.err(format!("typedef'd as {name}, not as its tag {tag}"));
        }
        p.punct(';')
    }

    /// `{ A = 1, /* c */ B, ... }`: a comment that starts on a case's line, after it, is that case's; one that starts on
    /// a line of its own leads the next case (`/* A player's device kind changed */ LUMA_INPUT_EVENT_DEVICE = 1,`), or,
    /// before the `}`, is the last case's.
    fn enum_body(p: &mut P, name: Option<String>) -> Result<CEnum, HeaderError> {
        p.punct('{')?;
        let mut cases: Vec<(String, i64, String)> = Vec::new();
        let mut next = 0i64;
        let join = |a: &mut String, c: &str| {
            if !c.is_empty() {
                *a = if a.is_empty() { c.to_string() } else { format!("{a} {c}") };
            }
        };
        // The line the last case ended on (its comma's), and the comments that lead the next case.
        let mut end_line = 0u32;
        let mut leading = String::new();
        loop {
            while let Some((Tok::Comment(c), l)) = p.toks.get(p.i) {
                match cases.last_mut() {
                    Some(last) if *l == end_line => join(&mut last.2, c),
                    _ => join(&mut leading, c),
                }
                p.i += 1;
            }
            if p.is_punct('}') {
                if let Some(last) = cases.last_mut() {
                    join(&mut last.2, &leading);
                }
                p.next_code();
                break;
            }
            let case = p.ident()?;
            if p.is_punct('=') {
                p.next_code();
                // Whole numbers with `( )`, unary `-`, `+ -`, `<<` and `|` as C reads them (J 2026-11-12:
                // `LUMA_INPUT_BUTTONS_ALL = (1u << 13) - 1u`).
                next = fold_or(p, &case)?;
            }
            let mut c = std::mem::take(&mut leading);
            join(&mut c, &p.comments());
            end_line = p.line();
            cases.push((case, next, c));
            next += 1;
            if p.is_punct(',') {
                p.i += 1;
            } else if !p.is_punct('}') {
                return p.err("',' or '}' expected after a case");
            }
        }
        Ok(CEnum { name, cases })
    }

    fn struct_body(p: &mut P, numbers: &BTreeMap<String, u64>) -> Result<Vec<Member>, HeaderError> {
        p.punct('{')?;
        let mut members = Vec::new();
        // A section's comment (`/* ---- 2-D (...) ---- ... */`) is every function's below it until the next section
        // (J 2026-11-12: draw.rect_outline's statuses are its section's).
        let mut section = String::new();
        loop {
            let mut lead = Vec::new();
            while let Some(Tok::Comment(c)) = p.peek() {
                let c = c.clone();
                p.i += 1;
                if c.starts_with("----") {
                    section = c;
                } else {
                    lead.push(c);
                }
            }
            let lead = lead.join(" ");
            if p.is_punct('}') {
                p.next_code();
                return Ok(members);
            }
            let line = p.line();
            let mut words = Vec::new();
            while !p.is_punct(';') && !p.is_punct('(') && !p.is_punct('[') && !p.is_punct(',') {
                match p.next_code() {
                    Some(t) => words.push(t),
                    None => return p.err("a struct that does not end"),
                }
            }
            if p.is_punct('(') {
                let comment = if section.is_empty() { lead } else if lead.is_empty() { section.clone() } else { format!("{lead} {section}") };
                let f = Self::fn_ptr(p, words, comment, line, numbers)?;
                let end = p.line();
                p.punct(';')?;
                p.trailing(end);
                members.push(Member::Fn(f));
                continue;
            }
            // `T a, b;` and `char x[8];`
            let Some(Tok::Ident(first)) = words.pop() else { return p.err("a field with no name") };
            let Some(ty) = ct_of(&words) else { return p.err(format!("the type of {first} is not in the form")) };
            let mut names = vec![(first, None)];
            loop {
                if p.is_punct('[') {
                    p.next_code();
                    let Some(len) = p.next_code().and_then(|t| length_of(&t, numbers)) else { return p.err("an array's length is no whole number nor a #define above it") };
                    p.punct(']')?;
                    names.last_mut().expect("one name at least").1 = Some(len);
                }
                if p.is_punct(',') {
                    p.next_code();
                    names.push((p.ident()?, None));
                    continue;
                }
                break;
            }
            let end = p.line();
            p.punct(';')?;
            let trail = p.trailing(end);
            let comment = if trail.is_empty() { lead } else { trail };
            for (name, array) in names {
                members.push(Member::Field(Decl { ty: ty.clone(), name, array, comment: comment.clone() }));
            }
        }
    }

    /// `R (*name)(params)` from R's words; `(` is next.
    fn fn_ptr(p: &mut P, words: Vec<Tok>, comment: String, line: u32, numbers: &BTreeMap<String, u64>) -> Result<FnPtr, HeaderError> {
        let Some(result) = ct_of(&words) else { return p.err("a function's result is not in the form") };
        p.punct('(')?;
        p.punct('*')?;
        let name = p.ident()?;
        p.punct(')')?;
        p.punct('(')?;
        let mut params = Vec::new();
        let mut words: Vec<Tok> = Vec::new();
        // A function-pointer parameter `R (*fn)(...)` (luma_harness.h's other_thread): read whole, its type
        // `R (*)(...)`, which no script value has, so only its own function is not callable.
        let mut fn_param: Option<Decl> = None;
        loop {
            match p.next_code() {
                Some(Tok::Punct('(')) if fn_param.is_none() => {
                    let Some(r) = ct_of(&words) else { return p.err(format!("{name}: a function parameter's result is not in the form")) };
                    p.punct('*')?;
                    let pname = p.ident()?;
                    p.punct(')')?;
                    p.punct('(')?;
                    let mut depth = 1u32;
                    while depth > 0 {
                        match p.next_code() {
                            Some(Tok::Punct('(')) => depth += 1,
                            Some(Tok::Punct(')')) => depth -= 1,
                            Some(_) => {}
                            None => return p.err(format!("{name}: {pname}'s parameters do not end")),
                        }
                    }
                    let base = format!("{}{} (*)()", if r.konst { "const " } else { "" }, r.base) + &"*".repeat(r.ptrs as usize);
                    fn_param = Some(Decl { ty: CT { base, konst: false, ptrs: 0 }, name: pname, array: None, comment: String::new() });
                    words.clear();
                }
                Some(Tok::Punct(c @ (',' | ')'))) if fn_param.is_some() => {
                    if !words.is_empty() {
                        return p.err(format!("{name}: words after a function parameter"));
                    }
                    params.push(fn_param.take().expect("a function parameter"));
                    if c == ')' {
                        break;
                    }
                }
                Some(Tok::Punct(c @ (',' | ')'))) => {
                    let only_void = matches!(words.as_slice(), [Tok::Ident(v)] if v == "void");
                    if !(c == ')' && params.is_empty() && (only_void || words.is_empty())) {
                        // `T name[N]`: an array parameter (C reads it as `T*`), its length kept; N a whole number or a
                        // `#define` above it.
                        let mut array = None;
                        if let [.., Tok::Punct('['), n, Tok::Punct(']')] = words.as_slice() {
                            let Some(len) = length_of(n, numbers) else { return p.err(format!("{name}: an array parameter's length is no whole number nor a #define above it")) };
                            array = Some(len);
                            words.truncate(words.len() - 3);
                        }
                        let Some(Tok::Ident(pname)) = words.pop() else { return p.err(format!("{name}: a parameter with no name")) };
                        let Some(ty) = ct_of(&words) else { return p.err(format!("{name}: the type of {pname} is not in the form")) };
                        params.push(Decl { ty, name: pname, array, comment: String::new() });
                    }
                    words.clear();
                    if c == ')' {
                        break;
                    }
                }
                Some(t) => words.push(t),
                None => return p.err(format!("{name}: parameters that do not end")),
            }
        }
        Ok(FnPtr { name, result, params, comment, line })
    }

    /// A struct by name.
    pub fn record(&self, name: &str) -> Option<&CStruct> {
        self.structs.iter().find(|s| s.name == name)
    }
}

/// `system.clipboard` -> `SystemClipboard`.
pub fn camel(service: &str) -> String {
    service.split(['.', '_']).map(|w| {
        let mut c = w.chars();
        c.next().map(|f| f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase()).unwrap_or_default()
    }).collect()
}

/// `system.clipboard` -> `SYSTEM_CLIPBOARD`.
pub fn upper(service: &str) -> String {
    service.replace('.', "_").to_ascii_uppercase()
}

/// A case's LumaScript name: `NOT_FOUND` -> `NotFound`, `D3D12` -> `D3d12`.
pub fn case_name(words: &str) -> String {
    camel(words)
}

/// A LumaScript type of a service's face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum STy {
    Bool,
    /// A C number type by its name (`uint32_t`, `float`).
    Number(String),
    Text,
    Bytes,
    /// `<service>.<T>`, a struct.
    Struct(String),
    /// `<service>.<T>`, a named enum.
    Enum(String),
    /// `<service>.<K>`, a handle kind; `<service>.Request` for a bare LumaHandle.
    Handle(String),
    /// `<owner>.<K>`: a handle kind of another service whose header this one includes (`content.model.Model` in
    /// content.fx's start_on_model), as (owner, kind).
    HandleOf(String, String),
    /// `float x[2..4]`: `vec2` to `vec4`.
    Vec(u8),
    /// A shared struct of the content headers (`LumaPlace`), by its C name.
    Shared(String),
    /// `LumaActor`: a view of a game actor (LiveActor or a class deriving from it).
    Actor,
    /// A name kind of references 5 (`LumaEffectName` -> `effect`): a content name the build checks.
    ContentName(String),
    /// A pointer parameter the comment says may be null (`` `joint` (null: ...) ``): `T?`.
    Opt(Box<STy>),
    /// `LumaColor`: `color`, passed as 0xRRGGBBAA.
    Color,
    /// `list<T>`: an out buffer of names each followed by a 0 byte (`mods.kit_names`), or an array with its count.
    List(Box<STy>),
    /// `svc.Answer`: a stream event's `status`, as `Opt(Answer)`.
    Answer,
    /// `<owner>.<T>`: a struct of another service of the same header (`input.Game` in input.inject's `set`), as
    /// (owner, name).
    StructOf(String, String),
    /// `const T x[N]` (N not a vec's 2 to 4 floats): a tuple of N values (draw's `tri` corners, `transform_push`).
    Fixed(Box<STy>, u8),
}

/// One case of a service's `<svc>.Event`, the events its streams give: the kind's number, its fields as the
/// members' `events:` lines list them, and where its `socket` and `data` come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamKind {
    pub number: i64,
    pub fields: Vec<(String, STy)>,
    /// `socket` is the event's value, not its handle (ACCEPTED: "value: the new socket's handle").
    pub socket_in_value: bool,
    /// The take that gives `data` (and `from`): the member the kind's comment names before "inside this call"
    /// (`` `take` inside this call ``); None: `data` is LumaEvent.data.
    pub take: Option<String>,
    /// The take's room when its comment gives no way to ask the length (no "null buf with size 0"): the header's
    /// `LUMA_..._<KIND>_MOST` (take_datagram: LUMA_NET_DATAGRAM_MOST). None: asked first with a null buffer.
    pub room: Option<u64>,
    /// The take gives one item an event's value counts (`` `take_datagram` inside this call, one datagram each ``):
    /// the closure runs once a datagram, up to the value, in the event's call (those not taken would wait a frame each).
    pub each: bool,
}

/// the kinds a member's `events:` line lists with their fields (`events: DATAGRAM(socket, data, from),
/// SENT(socket, value)`), kinds lower case; None for a member without the line.
pub fn events_line(comment: &str) -> Option<Vec<(String, Vec<String>)>> {
    let at = comment.find("events:")?;
    let mut rest = &comment[at + "events:".len()..];
    let mut out = Vec::new();
    loop {
        rest = rest.trim_start();
        let end = rest.find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')).unwrap_or(rest.len());
        if end == 0 {
            break;
        }
        let kind = &rest[..end];
        let Some(inner) = rest[end..].trim_start().strip_prefix('(') else { break };
        let Some(close) = inner.find(')') else { break };
        let fields = inner[..close].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        out.push((kind.to_ascii_lowercase(), fields));
        rest = inner[close + 1..].trim_start();
        match rest.strip_prefix(',') {
            Some(r) => rest = r,
            None => break,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Whether the out buffer at `ps[i]` (its size at `i + 1`) gives its length in a `uint64_t* length` after the size
/// (`mods.kit_names`, `kit_read`: an `int32_t` status result) rather than as an `int64_t` result. The face reading
/// and the back end both ask this, so they walk the same parameters.
pub fn length_out(ps: &[Decl], i: usize) -> bool {
    matches!(ps.get(i + 2), Some(Decl { ty: CT { base, konst: false, ptrs: 1 }, name, array: None, .. }) if base == "uint64_t" && name == "length")
        || length_out_32(ps, i)
}

/// The same out buffer with a `uint32_t` size (`size`, `<x>_size`) and a `uint32_t* length` or `needed` after it
/// (`input.text_peek`, `assets.entry`): read as the `uint64_t` one, its length asked first with a size of 0.
pub fn length_out_32(ps: &[Decl], i: usize) -> bool {
    size_32(ps.get(i + 1)) && is_u32(ps.get(i + 2), 1, &["length", "needed"])
}

/// A `uint32_t size` or `uint32_t <x>_size` (a buffer's room or length in 32 bits).
pub fn size_32(d: Option<&Decl>) -> bool {
    matches!(d, Some(Decl { ty: CT { base, konst: false, ptrs: 0 }, name, array: None, .. }) if base == "uint32_t" && (name == "size" || name.ends_with("_size")))
}

/// A buffer in with a `uint32_t` size after it (`input.dev.write`): bytes, their length checked to fit 32 bits.
pub fn sized_in_32(ps: &[Decl], i: usize) -> bool {
    matches!(ps.get(i), Some(Decl { ty: CT { base, konst: true, ptrs: 1 }, array: None, .. }) if base == "void" || base == "uint8_t")
        && size_32(ps.get(i + 1))
        && !matches!(ps.get(i + 2), Some(Decl { ty: CT { ptrs: 1, konst: false, .. }, .. }))
}

/// A `const float* <x>` whose function's comment says how many floats it points at (`` `rgba` (four floats ``):
/// a vec of 2 to 4.
pub fn floats_in(comment: &str, d: &Decl) -> Option<u8> {
    if !(d.array.is_none() && d.ty.base == "float" && d.ty.ptrs == 1 && d.ty.konst) {
        return None;
    }
    [("two", 2), ("three", 3), ("four", 4)].into_iter().find(|(w, _)| comment.contains(&format!("`{}` ({w} floats", d.name))).map(|(_, n)| n)
}

/// `const <N>* x, uint64_t <count>` at `ps[i]` (N a number type) where another member of the table says how many
/// numbers each of `count` is: `int32_t g(const LumaMod*, H, uint32_t* out)`, H the type of a parameter before `x`, its
/// comment saying "<f> reads `<count>` x that many" (audio's stream_channels: "stream_push reads `frames` x that many
/// floats"). The script gives the numbers alone; the count is their number over g's answer, never the script's. g's
/// name and the index of the parameter it takes. The face reading and the back end both ask this.
pub fn items_by(f: &str, ps: &[Decl], i: usize, members: &[FnPtr]) -> Option<(String, usize)> {
    let x = ps.get(i)?;
    let count = ps.get(i + 1)?;
    if !(x.array.is_none() && x.ty.ptrs == 1 && x.ty.konst && NUMBERS.contains(&x.ty.base.as_str()) && x.ty.base != "uint8_t") {
        return None;
    }
    if !(count.array.is_none() && count.ty.base == "uint64_t" && count.ty.ptrs == 0) {
        return None;
    }
    let says = format!("{f} reads `{}` x that many", count.name);
    members.iter().filter(|g| g.comment.contains(&says)).find_map(|g| {
        let gp = &g.params;
        let fits = g.result.base == "int32_t" && g.result.ptrs == 0 && gp.len() == 3 && gp[1].ty.ptrs == 0 && gp[1].array.is_none() && is_u32(gp.get(2), 1, &["out"]);
        if !fits {
            return None;
        }
        let h = ps[..i].iter().position(|p| p.array.is_none() && p.ty == gp[1].ty)?;
        Some((g.name.clone(), h))
    })
}

/// Bytes the service lends: `const void** data, uint64_t* bytes` (or `const uint8_t**`) at `ps[i]` (assets.bytes):
/// `bytes`, copied into the script's own.
pub fn lent_out(ps: &[Decl], i: usize) -> bool {
    matches!(ps.get(i), Some(Decl { ty: CT { base, konst: true, ptrs: 2 }, array: None, .. }) if base == "void" || base == "uint8_t")
        && matches!(ps.get(i + 1), Some(Decl { ty: CT { base, konst: false, ptrs: 1 }, array: None, .. }) if base == "uint64_t")
}

/// The words an out buffer's comment uses for names each ended by a 0 byte: the buffer is `list<text>`.
pub const NAMES_PHRASE: &str = "each followed by a 0 byte";

fn is_u32(d: Option<&Decl>, ptrs: u8, names: &[&str]) -> bool {
    matches!(d, Some(Decl { ty: CT { base, konst: false, ptrs: p }, name, array: None, .. }) if base == "uint32_t" && *p == ptrs && names.contains(&name.as_str()))
}

/// Pixels in (`draw.texture_from_pixels`): `uint32_t w, uint32_t h, const void* rgba8` at `ps[i - 2]..=ps[i]`, the
/// buffer's length w x h x 4 by its own parameters rather than a size after it: the script's `bytes`, its length
/// checked at the call. The face reading and the back end both ask this.
pub fn pixels_in(ps: &[Decl], i: usize) -> bool {
    i >= 2
        && matches!(ps.get(i), Some(Decl { ty: CT { base, konst: true, ptrs: 1 }, name, array: None, .. }) if (base == "void" || base == "uint8_t") && name == "rgba8")
        && is_u32(ps.get(i - 2), 0, &["w"])
        && is_u32(ps.get(i - 1), 0, &["h"])
}

/// An array in (C1c): `const T* x, uint32_t count` at `ps[i]` (`ui.menu_rows`, `draw.post.params`): one `list<T>`
/// argument. The face reading and the back end both ask this.
pub fn array_in(ps: &[Decl], i: usize) -> bool {
    matches!(ps.get(i), Some(Decl { ty: CT { konst: true, ptrs: 1, .. }, array: None, .. })) && is_u32(ps.get(i + 1), 0, &["count"])
}

/// An array out (C1c): `T* out, uint32_t capacity|count, uint32_t* count|total|found` at `ps[i]` (`mods.needing`,
/// `content.model.player_parts`, `input.dev.list`): a `list<T>` out; the call asks the number with a capacity of 0
/// first. The face reading and the back end both ask this.
pub fn array_out(ps: &[Decl], i: usize) -> bool {
    matches!(ps.get(i), Some(Decl { ty: CT { konst: false, ptrs: 1, .. }, array: None, .. }))
        && is_u32(ps.get(i + 1), 0, &["capacity", "count"])
        && is_u32(ps.get(i + 2), 1, &["count", "total", "found"])
}

/// A parameter as the script writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SParam {
    /// A value (a struct by value included).
    Value { name: String, ty: STy },
    /// The closure for a request's or a stream's events (`LumaEventFn fn, void *user`); `optional`: a stream's whose
    /// comment takes a null fn (`` `fn` null: none ``, `a null fn stops the events`), `fn(..)?` to the script.
    Closure { optional: bool },
    /// A function the service calls back, `X fn, void* user` with `typedef void (*X)(void* user, args...)` of the
    /// header: the script gives a `fn(args)` value. `c_fn` is the typedef, `c_args` its C parameters after `user`.
    Callback { name: String, c_fn: String, args: Vec<STy>, c_args: Vec<CT> },
}

/// What a call gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gives {
    /// Nothing, or the outs (one: that type, more: a tuple).
    Outs(Vec<STy>),
    /// The outs as `T?`, or `bool` with none: the function has answers.
    Answered(Vec<STy>),
    /// A number that is no status.
    Number(String),
    /// A request: its handle (`<service>.<K>?`); waitable when it closes after its one event; `kind` the event kind
    /// its comment says follows (`a TIMER event follows`, lower case), whose record the closure takes.
    Request { handle: STy, waitable: bool, kind: String },
    /// A stream (an `events:` line): its handle (`<service>.<K>?`); the closure takes `<service>.Event`. A
    /// stream with no handle out (a watch: `input.watch_device`) gives what its answers give (`bool`, or nothing).
    Stream { handle: Option<STy> },
}

/// One function of a face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Callable { params: Vec<SParam>, gives: Gives },
    /// A take function (`inside the <KIND> event's call`): the compiler's, in the event's call.
    Take { kind: String, data: STy },
    /// Set by `on <x>` (S0108).
    SetBy(String),
    /// Not callable from LumaScript, and why (S0060's ServiceParam, or a status list missing).
    NotCallable(Unfit),
}

/// Why a function of a face is not callable from LumaScript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unfit {
    /// A parameter of no LumaScript type (S0134): a C-only function, as set_image's pixels are.
    Param { param: String, why: ParamWhy, c_type: String },
    /// Not the registry's one form (S0135): LumaLink's own mirror test fails on it, so a modder never meets it from a
    /// LumaLink that passed its tests. The English detail is for that test.
    Form(String),
    /// A service whose header asks for permission `native` (draw.native): full access outside every guard, its
    /// functions run where no script runs (the render thread) with the device's own objects. For C mods only, by design.
    Native,
}

/// Why a parameter takes no LumaScript value (S0134's phrases `Script.Why.SvcBuffer` and the rest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamWhy {
    /// A buffer with no `uint64_t` size after it.
    Buffer,
    /// A pointer to something that is no struct of the service, or a const pointer to a number.
    Pointer,
    /// A type LumaScript does not take.
    Type,
}

impl ParamWhy {
    /// The phrase S0134 names.
    pub fn phrase(self) -> &'static str {
        match self {
            ParamWhy::Buffer => "Script.Why.SvcBuffer",
            ParamWhy::Pointer => "Script.Why.SvcPointer",
            ParamWhy::Type => "Script.Why.SvcType",
        }
    }
}

impl fmt::Display for Unfit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unfit::Param { param, why: ParamWhy::Buffer, .. } => write!(f, "{param}: a buffer with no uint64_t size after it"),
            Unfit::Param { param, why: ParamWhy::Pointer, c_type } => write!(f, "{param}: a pointer to {c_type}"),
            Unfit::Param { param, why: ParamWhy::Type, c_type } => write!(f, "{param}: {c_type} is no type LumaScript takes"),
            Unfit::Form(why) => f.write_str(why),
            Unfit::Native => f.write_str("a service with full access outside every guard (permission native), for C mods only"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SFn {
    pub name: String,
    pub shape: Shape,
    /// The statuses its comment names.
    pub statuses: Vec<String>,
    /// The answers among them: LUMA_E_* but the mod's mistakes and CANCELLED, as `svc.Answer` cases.
    pub answers: Vec<String>,
    pub any_thread: bool,
    /// `<param>: <a> to <b>` ranges a literal is checked against (S0111).
    pub ranges: Vec<(String, i64, i64)>,
    pub comment: String,
    /// The table member as the header declares it: the back end calls it with these parameters.
    pub c: FnPtr,
}

/// One catalogued service's LumaScript face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub name: String,
    pub functions: Vec<SFn>,
    /// Struct name (`Now`) -> its fields as the script reads them (`size` and `reserved*` hidden; a char array is text).
    pub structs: BTreeMap<String, Vec<(String, STy)>>,
    /// Enum name (`Clock`) -> its cases (`Frames`, value).
    pub enums: BTreeMap<String, Vec<(String, i64)>>,
    /// Handle kinds (`Timer`).
    pub handles: Vec<String>,
    /// Event kinds, lower case (`timer`) -> (value, the comment).
    pub events: BTreeMap<String, (i64, String)>,
    /// The cases of `<service>.Event`, lower case (`datagram`), from every stream member's `events:` line.
    pub stream: BTreeMap<String, StreamKind>,
    /// `#define LUMA_<NAME>_<C>` whole numbers: `C` -> value.
    pub constants: BTreeMap<String, u64>,
    /// Struct fields a script cannot read, `T.field` and why (an array of no char, a type of no service value).
    pub unreadable: Vec<(String, String)>,
    /// Structs and typedefs of the shared headers (luma_content.h's `LumaPlace`, `LumaActor`, `LumaEffectName`).
    pub shared: Vec<String>,
    pub shared_aliases: Vec<String>,
    /// The shared structs' fields, by C name (`LumaPlace` -> offset vec3, turn vec3); a script names them
    /// `content.<T>`.
    pub shared_structs: BTreeMap<String, Vec<(String, STy)>>,
    /// The header that declares it (`luma_ui.h`), which the back end includes, and the version its import asks
    /// (`LUMA_<NAME>_MAJOR`.`MINOR`, `"1.0"`; the kind 5 row, LUMA_SERVICE).
    pub header: String,
    pub version: String,
    /// A struct's C name by its script name (`Rect` -> `LumaDrawRect`).
    pub struct_c: BTreeMap<String, String>,
    /// The structs (C names, this face's and the shared headers') whose first field is `uint32_t size`: the
    /// compiler sets it to sizeof.
    pub sized: Vec<String>,
    /// Callback typedefs (`LumaDrawFn`) -> their parameters after `void* user`, as the script reads them.
    pub callbacks: BTreeMap<String, (Vec<STy>, Vec<CT>)>,
    /// Every handle typedef of the SDK by its C name -> (its service, its kind) ([`handle_kinds`]): another service's
    /// handle is a value of this face too.
    pub foreign: BTreeMap<String, (String, String)>,
    /// The header's structs of another service by C name -> (that service, the struct's name there): a const pointer
    /// to one takes that service's value (`const LumaInputGame*` in input.inject is an `input.Game`).
    pub foreign_structs: BTreeMap<String, (String, String)>,
    /// A struct's fields that are pointers ('s parameter rules on a field): struct name -> field -> how it is
    /// held ([`FieldPtr`]). A field not here is held in the struct itself.
    pub field_ptrs: BTreeMap<String, BTreeMap<String, FieldPtr>>,
}

/// How a struct's pointer field is held (on fields): the script sees text, bytes or a list; the C struct holds a
/// pointer to it and, for bytes and a list, its count in the field right after (hidden from the script). Given to a
/// service, the pointers are into the script's value for the call (a request's struct: "copied before it returns");
/// read from one, each is copied into the script's own value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldPtr {
    /// `const char *f`: text, NUL-terminated (null read as empty).
    Text,
    /// `const void *f` (or `const uint8_t *f`), then `uint64_t <len>`: bytes.
    Bytes { len: String },
    /// `const <T> *f`, then `uint32_t <count>`: a list of the struct `elem` (this face's name of `<T>`).
    List { count: String, elem: String, elem_c: String },
    /// `uint32_t <x>_count` then `const <N> *<x>s`, N a number type: a list of numbers (`anim.How.marks`).
    Numbers { count: String, elem_c: String },
    /// `const uint8_t *f` (or `const void *f`) in a struct with `uint32_t stride` and `uint32_t height`: bytes, stride x
    /// height of them (`capture.camera.Frame.pixels`); written to C, the script's bytes must be that long.
    Rows { stride: String, height: String },
}

/// The part of the C name `c` after `prefix` when it is a word of its own (`LumaInputInjectStep` after
/// `LumaInputInject`; `LumaInputInjection` is not): empty, or starting with no lower-case letter.
fn after_prefix<'c>(c: &'c str, prefix: &str) -> Option<&'c str> {
    c.strip_prefix(prefix).filter(|rest| !rest.starts_with(|ch: char| ch.is_ascii_lowercase()))
}

/// The handle kind of `typedef LumaHandle <name>;` for the service whose prefix it starts with: the rest of the name,
/// or, for a name that is the prefix itself (`LumaMovie`, `LumaContentModel`), the service's last word
/// (`movie.Movie`, `content.model.Model`).
fn handle_kind(service: &str, rest: &str) -> String {
    if rest.is_empty() {
        camel(service.rsplit('.').next().unwrap_or(service))
    } else {
        rest.to_string()
    }
}

/// Every `typedef LumaHandle X;` of `headers` -> (the service with the longest prefix `X` starts with, the kind), over
/// the services the headers name (`#define LUMA_<X>_NAME "<name>"`).
pub fn handle_kinds<'h>(headers: impl Iterator<Item = &'h Header> + Clone) -> BTreeMap<String, (String, String)> {
    let names: Vec<String> = headers
        .clone()
        .flat_map(|h| h.texts.iter().filter(|(d, _)| d.ends_with("_NAME")).map(|(_, v)| v.clone()))
        .collect();
    let mut out = BTreeMap::new();
    for h in headers {
        for (alias, ct) in &h.aliases {
            if ct.base != "LumaHandle" || ct.ptrs != 0 {
                continue;
            }
            let owner = names.iter().filter(|s| after_prefix(alias, &format!("Luma{}", camel(s))).is_some()).max_by_key(|s| camel(s).len());
            if let Some(s) = owner {
                let rest = &alias[format!("Luma{}", camel(s)).len()..];
                out.insert(alias.clone(), (s.clone(), handle_kind(s, rest)));
            }
        }
    }
    out
}

/// Every catalogued service's face and the language-wide `svc.Answer`, for one game version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Services {
    /// By service name (`system.clipboard`).
    pub faces: BTreeMap<String, Face>,
    /// `svc.Answer`'s cases (`Busy`) with their status numbers: luma_services.h's LUMA_E_* but the mod's mistakes and
    /// CANCELLED.
    pub answers: Vec<(String, i64)>,
}

/// Why a service set cannot be read: the header and why. A LumaLink whose SDK headers give one fails its own tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicesError(pub String);

impl fmt::Display for ServicesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Services {
    /// The faces of `catalog` (the self-description's `services` rows) from the SDK's headers, by file name. A service
    /// `a.b.c` is in the longest of `luma_a_b_c.h`, `luma_a_b.h`, `luma_a.h` there is (`content.fx` in
    /// luma_content_fx.h, `system.clipboard` in luma_system.h); its shared headers are luma_services.h and the headers
    /// it includes that hold no table of their own.
    /// Every service an SDK's headers declare, for a build that has no self-description's catalog: a header's
    /// `#define LUMA_<NAME>_NAME "<name>"` with its `LUMA_<NAME>_MAJOR` beside it names one (docs/modding/services/registry.md, section 2). Headers
    /// not in the one form (luma.h, luma_hook.h) are no service's and are left out; each service is read on its own,
    /// and one whose face cannot be read is given back with why, the others kept.
    pub fn of_sdk(headers: &BTreeMap<String, String>) -> (Services, Vec<ServicesError>) {
        let mut kept = BTreeMap::new();
        let mut catalog = Vec::new();
        let mut errors = Vec::new();
        for (file, text) in headers {
            // A header that names no service (luma.h, luma_hook.h, luma_types.h: the kernel's and C++ helpers, not in
            // the one form) is read only as a shared header, and left out silently when it does not read. A header
            // that names one and does not read is said: its services are then not callable. A service's name is text
            // (`#define LUMA_TIME_NAME "time"`); a `_NAME` define of no text is no service's (luma.h's LUMA_ZERO_NAME,
            // the C library's zeroing function).
            let names_one = text.lines().any(|l| {
                let mut words = l.split_whitespace();
                words.next() == Some("#define")
                    && words.next().is_some_and(|d| d.starts_with("LUMA_") && d.ends_with("_NAME"))
                    && words.next().is_some_and(|v| v.starts_with('"'))
            });
            let h = match Header::read(text) {
                Ok(h) => h,
                Err(_) if !names_one => continue,
                Err(e) => {
                    errors.push(ServicesError(format!("{file}: {e}")));
                    continue;
                }
            };
            // A name with its MAJOR names a service (docs/modding/services/registry.md, section 2). One whose header has no table that starts with
            // `LumaServiceTable base` is no script's (the proof harness's `harness`, a C table of its own); a name
            // whose table is not `Luma<Name>Service` is an error of Services::read, said below.
            let has_table = h.structs.iter().any(|s| {
                s.name.ends_with("Service")
                    && matches!(s.members.first(), Some(Member::Field(Decl { ty: CT { base, ptrs: 0, .. }, name, .. })) if base == "LumaServiceTable" && name == "base")
            });
            for (define, name) in &h.texts {
                let Some(up) = define.strip_prefix("LUMA_").and_then(|d| d.strip_suffix("_NAME")) else { continue };
                if has_table && h.numbers.contains_key(&format!("LUMA_{up}_MAJOR")) {
                    catalog.push(name.clone());
                }
            }
            kept.insert(file.clone(), text.clone());
        }
        catalog.sort();
        catalog.dedup();
        let mut out = Services::default();
        for name in &catalog {
            match Services::read(&[name.as_str()], &kept) {
                Ok(one) => {
                    out.answers = one.answers;
                    out.faces.extend(one.faces);
                }
                Err(e) => errors.push(e),
            }
        }
        (out, errors)
    }

    pub fn read(catalog: &[&str], headers: &BTreeMap<String, String>) -> Result<Services, ServicesError> {
        let mut read: BTreeMap<&str, Header> = BTreeMap::new();
        for (name, text) in headers {
            let h = Header::read(text).map_err(|e| ServicesError(format!("{name}: {e}")))?;
            read.insert(name.as_str(), h);
        }
        let common = read.get("luma_services.h").ok_or_else(|| ServicesError("no luma_services.h".into()))?;
        let mut out = Services { faces: BTreeMap::new(), answers: answers(common) };
        let foreign = handle_kinds(read.values());
        for service in catalog {
            let words: Vec<&str> = service.split('.').collect();
            // The header whose `#define LUMA_<X>_NAME "<service>"` names it (luma_capture.h holds audio.microphone),
            // else the longest file name that fits.
            let named = read.iter().find(|(_, h)| h.texts.iter().any(|(d, v)| d.ends_with("_NAME") && v == service)).map(|(f, _)| f.to_string());
            let file = named
                .or_else(|| (1..=words.len()).rev().map(|n| format!("luma_{}.h", words[..n].join("_"))).find(|f| read.contains_key(f.as_str())))
                .ok_or_else(|| ServicesError(format!("{service}: no luma_{}.h", words.join("_"))))?;
            let h = &read[file.as_str()];
            let mut commons = vec![common];
            // The structs of another service's header this one includes (luma_draw.h's LumaDrawVec3 in draw.view):
            // a const pointer to one takes that service's value, as a struct of the same header does.
            let mut included = BTreeMap::new();
            for inc in &h.includes {
                if inc == "luma_services.h" {
                    continue;
                }
                if let Some(shared) = read.get(inc.as_str()) {
                    if !shared.structs.iter().any(|s| s.name.ends_with("Service")) {
                        commons.push(shared);
                        continue;
                    }
                    let owners: Vec<&str> = shared.texts.iter().filter(|(d, _)| d.ends_with("_NAME")).map(|(_, v)| v.as_str()).collect();
                    for st in shared.structs.iter().filter(|st| !st.name.ends_with("Service")) {
                        let owner = owners.iter().filter(|o| after_prefix(&st.name, &format!("Luma{}", camel(o))).is_some()).max_by_key(|o| camel(o).len());
                        if let Some(o) = owner {
                            let rest = &st.name[format!("Luma{}", camel(o)).len()..];
                            included.insert(st.name.clone(), (o.to_string(), handle_kind(o, rest)));
                        }
                    }
                }
            }
            let mut face = Face::of_full(service, h, &commons, catalog, &foreign, &included).map_err(|e| ServicesError(format!("{service} ({file}): {e}")))?;
            face.header = file.clone();
            let up = upper(service);
            if let (Some(major), Some(minor)) = (h.numbers.get(&format!("LUMA_{up}_MAJOR")), h.numbers.get(&format!("LUMA_{up}_MINOR"))) {
                face.version = format!("{major}.{minor}");
            }
            out.faces.insert(service.to_string(), face);
        }
        Ok(out)
    }
}

impl Services {
    /// Every function of every service and every struct field scripts cannot read, with whether scripts reach it, as
    /// `lumalink services` prints it for the services' pages: (`function` or `field`, `<service>.<name>`, `script`,
    /// `native` for a service of permission native, by design C's alone, or `c` for a part no reading rule covers).
    /// Sorted by name.
    pub fn reach(&self) -> Vec<(&'static str, String, &'static str)> {
        let mut out = Vec::new();
        for (name, face) in &self.faces {
            let native = face.functions.iter().any(|f| matches!(f.shape, Shape::NotCallable(Unfit::Native)));
            for f in &face.functions {
                let reach = match &f.shape {
                    Shape::NotCallable(Unfit::Native) => "native",
                    Shape::NotCallable(_) => "c",
                    _ => "script",
                };
                out.push(("function", format!("{name}.{}", f.name), reach));
            }
            for (field, _) in &face.unreadable {
                out.push(("field", format!("{name}.{field}"), if native { "native" } else { "c" }));
            }
        }
        out.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
        out
    }

    /// Every part of the services a script cannot reach: each function that is not callable and each struct field a
    /// script cannot read, as (`service.function` or `service.Struct.field`, why), sorted. What the services page
    /// and `lumalink services --unreachable` list.
    pub fn unreachable(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (name, face) in &self.faces {
            for f in &face.functions {
                if let Shape::NotCallable(why) = &f.shape {
                    out.push((format!("{name}.{}", f.name), why.to_string()));
                }
            }
            for (field, why) in &face.unreadable {
                out.push((format!("{name}.{field}"), why.clone()));
            }
        }
        out.sort();
        out
    }
}

/// `svc.Answer` from luma_services.h's statuses: every LUMA_E_* but the mistakes and CANCELLED, CamelCase.
pub fn answers(common: &Header) -> Vec<(String, i64)> {
    common
        .enums
        .iter()
        .filter(|e| e.name.is_none())
        .flat_map(|e| e.cases.iter())
        .filter(|(c, _, _)| c.starts_with("LUMA_E_") && !NOT_ANSWERS.contains(&c.as_str()))
        .map(|(c, v, _)| (case_name(&c["LUMA_E_".len()..]), *v))
        .collect()
}

/// Why a face cannot be made from a header at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceError(pub String);

impl fmt::Display for FaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const NUMBERS: [&str; 10] = ["int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "float", "double"];
/// The mod's mistakes, reported by the service: never a script's value. CANCELLED never reaches running code.
/// A stream member's comment taking a null fn: its closure is `fn(..)?` (`none`: no events, or the watch stopped).
const OPTIONAL_FN: [&str; 5] = ["`fn` null", "fn (null", "With `fn`", "a null fn stops", "a null fn removes"];

const NOT_ANSWERS: [&str; 5] =["LUMA_E_ARGUMENT", "LUMA_E_THREAD", "LUMA_E_HANDLE", "LUMA_E_LIMIT", "LUMA_E_CANCELLED"];

/// The statuses (`LUMA_OK`, `LUMA_PENDING`, `LUMA_E_*`) a comment names, in order, each once.
fn statuses_in(comment: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in comment.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if (w == "LUMA_OK" || w == "LUMA_PENDING" || w.starts_with("LUMA_E_")) && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// `<param>: <a> to <b>` for one of the parameters.
fn ranges_in(comment: &str, params: &[Decl]) -> Vec<(String, i64, i64)> {
    let mut out = Vec::new();
    for p in params {
        let key = format!("{}: ", p.name);
        let mut rest = comment;
        while let Some(at) = rest.find(&key) {
            // A whole word: the key not inside another name.
            let before_ok = rest[..at].chars().last().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '`'));
            let after = &rest[at + key.len()..];
            let words: Vec<&str> = after.split_whitespace().take(3).collect();
            if before_ok {
                if let [a, "to", b] = words.as_slice() {
                    let b = b.trim_end_matches(|c: char| !c.is_ascii_digit());
                    if let (Ok(lo), Ok(hi)) = (a.parse::<i64>(), b.parse::<i64>()) {
                        out.push((p.name.clone(), lo, hi));
                        break;
                    }
                }
            }
            rest = &rest[at + key.len()..];
        }
    }
    out
}

/// A pointer parameter whose comment says `` `name` (null: ``: `T?`.
fn nullable(comment: &str, name: &str, ty: STy) -> STy {
    // `name` (null: ...), or the parenthesis saying it among other things: `src` (texels; null: the whole).
    let lead = format!("`{name}` (");
    let says = comment.match_indices(&lead).any(|(at, _)| {
        let group = &comment[at + lead.len()..];
        let group = &group[..group.find(')').unwrap_or(group.len())];
        group.starts_with("null") || group.contains("; null")
    });
    if says {
        STy::Opt(Box::new(ty))
    } else {
        ty
    }
}

/// A struct whose first field is `uint32_t size` (the header's versioning, docs/modding/language/compiled-form.md).
fn sized(s: &CStruct) -> bool {
    matches!(s.members.first(), Some(Member::Field(Decl { ty: CT { base, ptrs: 0, .. }, name, array: None, .. })) if base == "uint32_t" && name == "size")
}

/// Every word that follows `phrase` in the comment, in order (the phrase at a word's start).
fn phrase_words<'c>(comment: &'c str, phrase: &str) -> Vec<&'c str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = comment[from..].find(phrase) {
        let at = from + i;
        let starts_word = at == 0 || !comment.as_bytes()[at - 1].is_ascii_alphanumeric();
        let rest = &comment[at + phrase.len()..];
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
        if starts_word && end > 0 {
            out.push(&rest[..end]);
        }
        from = at + phrase.len();
    }
    out
}

/// `word` in `text` as a whole word (no letter, digit or `_` either side).
fn has_word(text: &str, word: &str) -> bool {
    let inner = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| !text[..at].ends_with(inner) && !text[at + word.len()..].starts_with(inner))
}

/// The comment of the table member at `i` (`table.members[i]`) as the reader takes it. A member without a comment
/// takes the nearest one above when that comment names it (`send`, `take` and `close` under "As net.local's send,
/// take and close"). A comment saying `As <ref>` is followed by <ref>'s comment, `depth` references deep: `<ref>` a
/// member of this table (`As read`, ``As `game` ``), another service's (`As fs.read`), or the member of the same name
/// in another service of the header (`As net.local's`, `As net.local's tls_connect` on tls_connect).
fn member_comment(header: &Header, table: &CStruct, i: usize, depth: u32) -> String {
    fn fns(t: &CStruct, j: usize) -> Option<&FnPtr> {
        match t.members.get(j) {
            Some(Member::Fn(f)) => Some(f),
            _ => None,
        }
    }
    let Some(f) = fns(table, i) else { return String::new() };
    let mut own = f.comment.clone();
    if own.trim().is_empty() {
        if let Some(above) = (1..i).rev().filter_map(|j| fns(table, j)).find(|g| !g.comment.trim().is_empty()) {
            if has_word(&above.comment, &f.name) {
                own = above.comment.clone();
            }
        }
    }
    if depth == 0 {
        return own;
    }
    let table_of = |svc: &str| -> Option<&CStruct> {
        let ok = !svc.is_empty() && svc.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
        if ok {
            header.record(&format!("Luma{}Service", camel(svc)))
        } else {
            None
        }
    };
    for (at, _) in own.match_indices("As ") {
        if own[..at].ends_with(|c: char| c.is_ascii_alphanumeric()) {
            continue;
        }
        let rest = own[at + 3..].trim_start_matches('`');
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).unwrap_or(rest.len());
        let token = rest[..end].trim_end_matches('.');
        let possessive = rest[end..].starts_with("'s");
        let target = match (table_of(token), token.rfind('.')) {
            (Some(t), _) => Some((t, f.name.as_str())),
            (None, _) if possessive => None,
            (None, Some(dot)) => table_of(&token[..dot]).map(|t| (t, &token[dot + 1..])),
            (None, None) => Some((table, token)),
        };
        let Some((t, name)) = target else { continue };
        let found = t.members.iter().enumerate().skip(1).find(|(j, m)| matches!(m, Member::Fn(g) if g.name == name) && !(std::ptr::eq(t, table) && *j == i));
        if let Some((j, _)) = found {
            return format!("{own} {}", member_comment(header, t, j, depth - 1));
        }
    }
    own
}

/// The word after a fixed phrase: `set by on frame` -> `frame`.
fn after_phrase<'c>(comment: &'c str, phrase: &str) -> Option<&'c str> {
    let at = comment.find(phrase)?;
    comment[at + phrase.len()..].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).find(|w| !w.is_empty())
}

impl Face {
    /// The face of `service` (`time`, `system.clipboard`) from its header (`header`) and the services header
    /// (`common`, luma_services.h: the statuses). `catalog` holds every catalogued service's name, so a longer
    /// service's names (`LumaSystemClipboard...` beside `LumaSystem...`) are never taken for this one's.
    pub fn of(service: &str, header: &Header, common: &Header, catalog: &[&str]) -> Result<Face, FaceError> {
        Self::of_with(service, header, &[common], catalog)
    }

    /// As [`Face::of`], with every shared header the service's includes (luma_services.h first, then luma_content.h
    /// and the like): their struct names and typedefs are known to this face.
    pub fn of_with(service: &str, header: &Header, commons: &[&Header], catalog: &[&str]) -> Result<Face, FaceError> {
        Self::of_all(service, header, commons, catalog, &BTreeMap::new())
    }

    /// As [`Face::of_with`], knowing every handle typedef of the SDK (`foreign`, [`handle_kinds`]).
    pub fn of_all(service: &str, header: &Header, commons: &[&Header], catalog: &[&str], foreign: &BTreeMap<String, (String, String)>) -> Result<Face, FaceError> {
        Self::of_full(service, header, commons, catalog, foreign, &BTreeMap::new())
    }

    /// As [`Face::of_all`], knowing the structs of the other services' headers this one includes by C name ->
    /// (service, name there).
    pub fn of_full(
        service: &str,
        header: &Header,
        commons: &[&Header],
        catalog: &[&str],
        foreign: &BTreeMap<String, (String, String)>,
        included: &BTreeMap<String, (String, String)>,
    ) -> Result<Face, FaceError> {
        let common = commons.first().ok_or_else(|| FaceError("no luma_services.h".into()))?;
        let prefix = format!("Luma{}", camel(service));
        // Dotted services under this one: the catalog's, and those the header names itself (`LUMA_MODS_IDS_NAME "mods.ids"`).
        let named = header.texts.iter().filter(|(k, _)| k.ends_with("_NAME")).map(|(_, v)| v.as_str());
        let mut longer_names: Vec<&str> = catalog
            .iter()
            .copied()
            .chain(named)
            .filter(|c| c.len() > service.len() && c.starts_with(service) && c.as_bytes()[service.len()] == b'.')
            .collect();
        longer_names.sort_unstable();
        longer_names.dedup();
        let longer: Vec<String> = longer_names.iter().map(|c| format!("Luma{}", camel(c))).collect();
        // The part of a C name after this service's prefix, when it is this service's; a name that is the prefix itself
        // (`typedef LumaHandle LumaMovie;`) is the service's last word (`movie.Movie`, C1g).
        let own = |c: &str| -> Option<String> {
            let rest = after_prefix(c, &prefix)?;
            if longer.iter().any(|l| after_prefix(c, l).is_some()) {
                return None;
            }
            Some(handle_kind(service, rest))
        };
        let known: Vec<String> = common.enums.iter().flat_map(|e| e.cases.iter().map(|c| c.0.clone())).collect();
        let table_name = format!("{prefix}Service");
        let table = header.record(&table_name).ok_or_else(|| FaceError(format!("no table {table_name}")))?;
        if !matches!(table.members.first(), Some(Member::Field(Decl { ty: CT { base, ptrs: 0, .. }, name, .. })) if base == "LumaServiceTable" && name == "base") {
            return Err(FaceError(format!("{table_name} does not start with LumaServiceTable base")));
        }
        let mut face = Face { name: service.to_string(), functions: Vec::new(), structs: BTreeMap::new(), enums: BTreeMap::new(), handles: Vec::new(), events: BTreeMap::new(), stream: BTreeMap::new(), constants: BTreeMap::new(), unreadable: Vec::new(), shared: Vec::new(), shared_aliases: Vec::new(), shared_structs: BTreeMap::new(), header: String::new(), version: String::new(), struct_c: BTreeMap::new(), sized: Vec::new(), callbacks: BTreeMap::new(), foreign: foreign.clone(), foreign_structs: included.clone(), field_ptrs: BTreeMap::new() };
        // The header's structs of the other services it declares (the one with the longest prefix owns each).
        let services: Vec<&str> = catalog.iter().copied().chain(header.texts.iter().filter(|(k, _)| k.ends_with("_NAME")).map(|(_, v)| v.as_str())).collect();
        for s in &header.structs {
            if s.name.ends_with("Service") || own(&s.name).is_some() {
                continue;
            }
            let owner = services.iter().filter(|o| after_prefix(&s.name, &format!("Luma{}", camel(o))).is_some()).max_by_key(|o| camel(o).len());
            if let Some(o) = owner.filter(|o| **o != service) {
                let rest = &s.name[format!("Luma{}", camel(o)).len()..];
                face.foreign_structs.insert(s.name.clone(), (o.to_string(), handle_kind(o, rest)));
            }
        }
        // A struct of the header shared by the services under one name, no service of that name owning it
        // (`LumaNetHttpRequest`, `LumaNetHeader` beside net.local and net.internet): the rest after `Luma<First>`, as
        // record_of names a take's (`Peer`); never `Request` or `Event`, every face's own.
        let first = format!("Luma{}", camel(service.split('.').next().unwrap_or(service)));
        let group = |c: &str| -> Option<String> {
            if own(c).is_some() || services.iter().any(|o| after_prefix(c, &format!("Luma{}", camel(o))).is_some()) {
                return None;
            }
            after_prefix(c, &first).filter(|r| !r.is_empty() && *r != "Request" && *r != "Event").map(str::to_string)
        };
        // A struct of this header no service's name prefixes (`LumaAssetInfo` in luma_assets.h, `LumaPlayerPart` in
        // luma_content_model.h): this face's, by the rest after `Luma`.
        let local = |c: &str| -> Option<String> {
            if services.iter().any(|o| after_prefix(c, &format!("Luma{}", camel(o))).is_some()) || !header.structs.iter().any(|s| s.name == c) {
                return None;
            }
            c.strip_prefix("Luma").filter(|r| r.starts_with(|ch: char| ch.is_ascii_uppercase()) && *r != "Request" && *r != "Event").map(str::to_string)
        };
        let named = |c: &str| own(c).or_else(|| group(c)).or_else(|| local(c));
        for h in &commons[1..] {
            face.shared.extend(h.structs.iter().map(|s| s.name.clone()));
            face.shared_aliases.extend(h.aliases.iter().map(|a| a.0.clone()));
        }
        // Enums, the event enum among them: `Luma<Svc>Event`, or an enum without a name whose cases are all
        // `LUMA_<SVC>_EVENT_<KIND>` (anim's, movie's); a table with event functions and no events of its own takes
        // the nearest service's above it (fs.exchange: fs's LumaFsEvent).
        let event_cases = |svc: &str| -> Option<Vec<(String, i64, String)>> {
            let named = format!("Luma{}Event", camel(svc));
            let lead = format!("LUMA_{}_EVENT_", upper(svc));
            let e = header.enums.iter().find(|e| match e.name.as_deref() {
                Some(n) => n == named,
                None => !e.cases.is_empty() && e.cases.iter().all(|c| c.0.starts_with(&lead)),
            })?;
            Some(e.cases.iter().filter_map(|(c, v, comment)| c.strip_prefix(&lead).map(|k| (k.to_ascii_lowercase(), *v, comment.clone()))).collect())
        };
        let takes_events = table.members.iter().any(|m| matches!(m, Member::Fn(f) if f.params.iter().any(|d| d.ty.base == "LumaEventFn")));
        let mut above = Some(service);
        while let Some(svc) = above {
            if let Some(cases) = event_cases(svc) {
                face.events.extend(cases.into_iter().map(|(k, v, c)| (k, (v, c))));
                break;
            }
            if !takes_events {
                break;
            }
            above = svc.rfind('.').map(|dot| &svc[..dot]);
        }
        for e in &header.enums {
            let Some(rest) = e.name.as_deref().and_then(own) else { continue };
            let common_prefix = case_prefix(&e.cases);
            if rest == "Event" {
                let lead = format!("LUMA_{}_EVENT_", upper(service));
                // Read above (event_cases); a case outside the lead is the header's mistake.
                if let Some((c, _, _)) = e.cases.iter().find(|c| !c.0.starts_with(&lead)) {
                    return Err(FaceError(format!("{c} is not {lead}<KIND>")));
                }
            } else {
                face.enums.insert(rest, e.cases.iter().map(|(c, v, _)| (case_name(&c[common_prefix.len()..]), *v)).collect());
            }
        }
        for (name, ct) in &header.aliases {
            if let Some(rest) = own(name) {
                if ct.base == "LumaHandle" && ct.ptrs == 0 {
                    face.handles.push(rest);
                }
            }
        }
        let lead = format!("LUMA_{}_", upper(service));
        let longer_lead: Vec<String> = longer_names.iter().map(|c| format!("LUMA_{}_", upper(c))).collect();
        // A whole-number `#define LUMA_<SVC>_<C>` and a case of an unnamed `enum { LUMA_<SVC>_<C> = .. }` are both the
        // constant `<svc>.<C>`: a field or parameter of uint32_t that takes one (LumaInputTextRequest.kind's
        // LUMA_INPUT_TEXT_*, a flags word's bits) is written with its name. The event enum's cases are the stream's
        // (read above), not constants; a value below 0 is no constant of a whole number.
        let unnamed = header
            .enums
            .iter()
            .filter(|e| e.name.is_none())
            .flat_map(|e| e.cases.iter())
            .filter_map(|(c, v, _)| u64::try_from(*v).ok().map(|v| (c, v)))
            .filter(|(c, _)| !c.strip_prefix(&lead).is_some_and(|k| k.starts_with("EVENT_")));
        for (name, v) in header.numbers.iter().map(|(n, v)| (n, *v)).chain(unnamed) {
            if let Some(c) = name.strip_prefix(&lead) {
                if !longer_lead.iter().any(|l| name.starts_with(l.as_str())) && c != "MAJOR" && c != "MINOR" {
                    face.constants.insert(c.to_string(), v);
                }
            }
        }
        for s in &header.structs {
            if s.name == table_name || s.name.ends_with("Service") {
                continue;
            }
            let Some(rest) = named(&s.name) else { continue };
            let fields = face.fields(s, &rest, &named);
            face.structs.insert(rest.clone(), fields);
            face.struct_c.insert(rest, s.name.clone());
            if sized(s) {
                face.sized.push(s.name.clone());
            }
        }
        let none = |_: &str| -> Option<String> { None };
        for h in &commons[1..] {
            for s in &h.structs {
                let fields = face.fields(s, &s.name, &none);
                face.shared_structs.insert(s.name.clone(), fields);
                if sized(s) {
                    face.sized.push(s.name.clone());
                }
            }
        }
        // Callback typedefs: `typedef void (*X)(void* user, ...)`, each later parameter a value or a const struct
        // pointer the script reads (LumaEventFn is the requests' own).
        for t in &header.fn_types {
            if t.name == "LumaEventFn" || !(t.result.base == "void" && t.result.ptrs == 0) {
                continue;
            }
            match t.params.first() {
                Some(Decl { ty: CT { base, konst: false, ptrs: 1 }, array: None, .. }) if base == "void" => {}
                _ => continue,
            }
            let mut args = Vec::new();
            let mut ok = true;
            for d in &t.params[1..] {
                let ty = if d.array.is_some() {
                    None
                } else if d.ty.ptrs == 1 && d.ty.konst {
                    own(&d.ty.base).filter(|r| face.structs.contains_key(r)).map(STy::Struct)
                } else {
                    face.value_ty(&d.ty, &own)
                };
                match ty {
                    Some(ty) => args.push(ty),
                    None => ok = false,
                }
            }
            if ok {
                let c_args = t.params[1..].iter().map(|d| d.ty.clone()).collect();
                face.callbacks.insert(t.name.clone(), (args, c_args));
            }
        }
        let mut members = Vec::new();
        for (i, m) in table.members.iter().enumerate().skip(1) {
            let Member::Fn(f) = m else { return Err(FaceError(format!("{table_name}: a member that is no function"))) };
            members.push(FnPtr { comment: member_comment(header, table, i, 4), ..f.clone() });
        }
        for f in &members {
            let made = face.make_fn(f, &named, &known, &members);
            face.functions.push(made);
        }
        face.read_streams(header, service, &own);
        // A service the header says needs permission `native` (draw.native) is C's alone, every function of it.
        if header.lead.contains("permission `native`") {
            for f in &mut face.functions {
                f.shape = Shape::NotCallable(Unfit::Native);
            }
        }
        Ok(face)
    }

    /// the cases of `<service>.Event` from every stream member's `events:` line. A kind lists the same fields,
    /// of the same types, on every line (else the member is not callable: the header fails LumaLink's mirror test);
    /// its number is the header's `..._EVENT_<KIND>` (net's `LumaNetEvent`, shared by net.local and net.internet).
    /// Each take a kind names is the compiler's, not the script's (as's takes).
    fn read_streams(&mut self, header: &Header, service: &str, own: &dyn Fn(&str) -> Option<String>) {
        let mut wrong: Vec<(usize, String)> = Vec::new();
        let mut takes: Vec<(String, String, STy)> = Vec::new();
        for i in 0..self.functions.len() {
            let Shape::Callable { gives: Gives::Stream { handle }, .. } = &self.functions[i].shape else { continue };
            let handle = handle.clone();
            let Some(line) = events_line(&self.functions[i].comment) else { continue };
            for (kind, names) in line {
                match self.stream_kind(header, service, &kind, &names, handle.as_ref(), own) {
                    Ok(k) => {
                        if let (Some(t), Some((_, data))) = (&k.take, k.fields.iter().find(|(n, _)| n == "data")) {
                            takes.push((t.clone(), kind.clone(), data.clone()));
                        }
                        match self.stream.get(&kind) {
                            Some(old) if *old != k => wrong.push((i, format!("events: {} differs from another member's", kind.to_ascii_uppercase()))),
                            Some(_) => {}
                            None => {
                                self.stream.insert(kind, k);
                            }
                        }
                    }
                    Err(why) => wrong.push((i, why)),
                }
            }
        }
        for (i, why) in wrong {
            self.functions[i].shape = Shape::NotCallable(Unfit::Form(why));
        }
        for (name, kind, data) in takes {
            if let Some(f) = self.functions.iter_mut().find(|f| f.name == name) {
                if !matches!(f.shape, Shape::Take { .. }) {
                    f.shape = Shape::Take { kind, data };
                }
            }
        }
    }

    /// One kind of an `events:` line: its number and comment from the header, its fields' types, its take.
    fn stream_kind(&mut self, header: &Header, service: &str, kind: &str, names: &[String], handle: Option<&STy>, own: &dyn Fn(&str) -> Option<String>) -> Result<StreamKind, String> {
        let up = kind.to_ascii_uppercase();
        let (number, comment) = match self.events.get(kind) {
            Some((n, c)) => (*n, c.clone()),
            None => header
                .enums
                .iter()
                .flat_map(|e| e.cases.iter())
                .find(|(c, _, _)| c.starts_with("LUMA_") && c.ends_with(&format!("_EVENT_{up}")))
                .map(|(_, n, c)| (*n, c.clone()))
                .ok_or_else(|| format!("events: {up} is no event kind of the header"))?,
        };
        // The take: `` `name` inside this call ``.
        let take = comment.find("` inside this call").and_then(|end| comment[..end].rfind('`').map(|start| comment[start + 1..end].to_string()));
        let take_fn = match &take {
            Some(t) => {
                let f = self.functions.iter().find(|f| f.name == *t).ok_or_else(|| format!("events: {up}'s take {t} is no member"))?;
                let ps = &f.c.params;
                let fits = f.c.result.base == "int64_t" && f.c.result.ptrs == 0 && (ps.len() == 4 || ps.len() == 5) && ps[2].ty.ptrs == 1 && ps[3].ty.base == "uint64_t";
                if !fits {
                    return Err(format!("events: {up}'s take {t} is not (mod, handle, buf, size[, out])"));
                }
                Some(f.clone())
            }
            None => None,
        };
        let room = match &take_fn {
            Some(f) if !f.comment.contains("null buf with size 0") => {
                let most = format!("_{up}_MOST");
                let v = header.numbers.iter().find(|(n, _)| n.starts_with("LUMA_") && n.ends_with(&most)).map(|(_, v)| *v);
                Some(v.ok_or_else(|| format!("events: {up}'s take gives no way to ask the length, and the header no LUMA_..{most}"))?)
            }
            _ => None,
        };
        let mut fields = Vec::new();
        for n in names {
            let ty = match n.as_str() {
                "socket" => handle.cloned().ok_or_else(|| format!("events: {up}'s socket, and the member gives no handle"))?,
                "data" if comment.contains("UTF-8 text") => STy::Text,
                // A struct of the header (`data, a LumaCaptureCameraFrame (valid during the call)`): the script's copy.
                "data" => match comment.find("data, a Luma").map(|at| &comment[at + "data, a ".len()..]) {
                    Some(rest) => {
                        let c = &rest[..rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).unwrap_or(rest.len())];
                        own(c).filter(|n| self.structs.contains_key(n)).map(STy::Struct).ok_or_else(|| format!("events: {up}'s data is a {c}, no struct of the service"))?
                    }
                    None => STy::Bytes,
                },
                "status" => STy::Opt(Box::new(STy::Answer)),
                "value" => STy::Number("uint64_t".into()),
                "flags" => STy::Number("uint32_t".into()),
                "from" => {
                    let Some(out) = take_fn.as_ref().and_then(|f| f.c.params.get(4)) else { return Err(format!("events: {up}'s from has no take out")) };
                    self.record_of(header, service, &out.ty, own).ok_or_else(|| format!("events: {up}'s from is no struct a script reads"))?
                }
                other => return Err(format!("events: {up}'s {other} is not socket, data, status, value, flags or from")),
            };
            fields.push((n.clone(), ty));
        }
        let socket_in_value = names.iter().any(|n| n == "socket") && comment.contains("value: the new") && comment.contains("handle");
        let each = take.is_some() && comment.contains("` inside this call, one ") && has_word(&comment, "each");
        Ok(StreamKind { number, fields, socket_in_value, take, room, each })
    }

    /// The struct a take's out points at as this face's (`LumaNetPeer *from` -> `Peer`): its own name, or, for a
    /// struct of the header shared by the services under one name (`LumaNet...` beside `net.local`, `net.internet`),
    /// the rest after `Luma<First>`. Added to the face's structs.
    fn record_of(&mut self, header: &Header, service: &str, t: &CT, own: &dyn Fn(&str) -> Option<String>) -> Option<STy> {
        if t.ptrs != 1 || t.konst {
            return None;
        }
        let first = format!("Luma{}", camel(service.split('.').next().unwrap_or(service)));
        let name = own(&t.base).or_else(|| t.base.strip_prefix(&first).filter(|r| !r.is_empty()).map(str::to_string))?;
        if !self.structs.contains_key(&name) {
            if self.enums.contains_key(&name) || self.handles.contains(&name) || name == "Request" || name == "Event" {
                return None;
            }
            let s = header.record(&t.base)?;
            let fields = self.fields(s, &name, own);
            self.structs.insert(name.clone(), fields);
            self.struct_c.insert(name.clone(), t.base.clone());
        }
        Some(STy::Struct(name))
    }

    /// A struct's fields as a script reads them: `size` and `reserved*` hidden, a char array text, `float[2..4]` a vector;
    /// a pointer field by D12.3's parameter rules ([`FieldPtr`]: `const char *` text, `const void *` then its
    /// `uint64_t` size bytes, `const <T> *` of this face's struct then its `uint32_t` count a list, `const <N> *<x>s`
    /// of numbers after its `uint32_t <x>_count` a list of numbers, the size or count hidden; `const uint8_t *` in a
    /// struct with `stride` and `height` bytes of stride x height); any other field is put in `unreadable` as
    /// `<name>.<field>` with why.
    fn fields(&mut self, s: &CStruct, name: &str, own: &dyn Fn(&str) -> Option<String>) -> Vec<(String, STy)> {
        let mut fields = Vec::new();
        let decls: Vec<&Decl> = s.members.iter().filter_map(|m| if let Member::Field(d) = m { Some(d) } else { None }).collect();
        let mut i = 0;
        while i < decls.len() {
            let d = decls[i];
            i += 1;
            if d.name == "size" || d.name.starts_with("reserved") {
                continue;
            }
            if d.array.is_none() && d.ty.ptrs == 1 && d.ty.konst {
                let next = decls.get(i).filter(|n| n.array.is_none() && n.ty.ptrs == 0).map(|n| (n.ty.base.as_str(), n.name.clone()));
                // A count before the pointer it counts (`uint32_t mark_count; const float* marks;`), already read as a
                // number field: taken out again, the list holds it.
                let before = (i >= 2)
                    .then(|| decls[i - 2])
                    .filter(|p| p.array.is_none() && p.ty.base == "uint32_t" && p.ty.ptrs == 0)
                    .filter(|p| p.name.strip_suffix("_count").is_some_and(|x| !x.is_empty() && d.name == format!("{x}s")))
                    .map(|p| p.name.clone());
                let number = NUMBERS.contains(&d.ty.base.as_str()) && d.ty.base != "uint8_t";
                let has = |n: &str| decls.iter().any(|x| x.name == n && x.array.is_none() && x.ty.ptrs == 0 && x.ty.base == "uint32_t");
                let held = match (d.ty.base.as_str(), next) {
                    ("char", _) => Some((STy::Text, FieldPtr::Text, false)),
                    ("void" | "uint8_t", Some(("uint64_t", len))) => Some((STy::Bytes, FieldPtr::Bytes { len }, true)),
                    ("void" | "uint8_t", _) if has("stride") && has("height") => {
                        Some((STy::Bytes, FieldPtr::Rows { stride: "stride".into(), height: "height".into() }, false))
                    }
                    (base, _) if number && before.is_some() => {
                        let count = before.clone().unwrap_or_default();
                        if fields.last().is_some_and(|(f, _)| *f == count) {
                            fields.pop();
                        }
                        Some((STy::List(Box::new(STy::Number(base.to_string()))), FieldPtr::Numbers { count, elem_c: base.to_string() }, false))
                    }
                    (base, Some(("uint32_t", count))) => own(base)
                        .filter(|e| self.structs.contains_key(e) && !self.unreadable.iter().any(|(f, _)| f.starts_with(&format!("{e}."))))
                        .map(|elem| (STy::List(Box::new(STy::Struct(elem.clone()))), FieldPtr::List { count, elem, elem_c: base.to_string() }, true)),
                    _ => None,
                };
                if let Some((ty, how, skip)) = held {
                    self.field_ptrs.entry(name.to_string()).or_default().insert(d.name.clone(), how);
                    fields.push((d.name.clone(), ty));
                    if skip {
                        i += 1;
                    }
                    continue;
                }
            }
            let ty = match (&d.ty, d.array) {
                (CT { base, ptrs: 0, .. }, Some(_)) if base == "char" => STy::Text,
                (CT { base, ptrs: 0, .. }, Some(n @ 2..=4)) if base == "float" => STy::Vec(n as u8),
                (t, Some(n)) => {
                    self.unreadable.push((format!("{name}.{}", d.name), format!("an array of {n} {}", t.base)));
                    continue;
                }
                (t, None) => match self.value_ty(t, own) {
                    Some(t) => t,
                    None => {
                        self.unreadable.push((format!("{name}.{}", d.name), format!("{}{}", t.base, "*".repeat(t.ptrs as usize))));
                        continue;
                    }
                },
            };
            fields.push((d.name.clone(), ty));
        }
        fields
    }

    /// A value type (no pointer): a number, bool, this service's enum, struct or handle kind, or LumaHandle.
    fn value_ty(&self, t: &CT, own: &dyn Fn(&str) -> Option<String>) -> Option<STy> {
        if t.ptrs != 0 {
            return None;
        }
        if t.base == "bool" {
            return Some(STy::Bool);
        }
        if NUMBERS.contains(&t.base.as_str()) {
            return Some(STy::Number(t.base.clone()));
        }
        if t.base == "LumaHandle" {
            return Some(STy::Handle("Request".into()));
        }
        if t.base == "LumaColor" {
            return Some(STy::Color);
        }
        if t.base == "LumaActor" && self.shared_aliases.iter().any(|a| a == "LumaActor") {
            return Some(STy::Actor);
        }
        // A name kind (docs/modding/language/references.md, section 5): `Luma<Kind>Name`, a typedef of const char* in a shared header.
        if let Some(kind) = t.base.strip_prefix("Luma").and_then(|k| k.strip_suffix("Name")) {
            if !kind.is_empty() && self.shared_aliases.contains(&t.base) {
                return Some(STy::ContentName(kind.to_ascii_lowercase()));
            }
        }
        if self.shared_structs.contains_key(&t.base) {
            return Some(STy::Shared(t.base.clone()));
        }
        if let Some(rest) = own(&t.base) {
            if self.enums.contains_key(&rest) {
                return Some(STy::Enum(rest));
            } else if self.handles.contains(&rest) {
                return Some(STy::Handle(rest));
            } else if self.structs.contains_key(&rest) {
                return Some(STy::Struct(rest));
            }
        }
        // Another service's handle (C1g): content.fx's start_on_model takes a `content.model.Model`.
        self.foreign.get(&t.base).filter(|(s, _)| *s != self.name).map(|(s, k)| STy::HandleOf(s.clone(), k.clone()))
    }

    fn make_fn(&self, f: &FnPtr, own: &dyn Fn(&str) -> Option<String>, known: &[String], members: &[FnPtr]) -> SFn {
        let statuses = statuses_in(&f.comment);
        let answers: Vec<String> = statuses
            .iter()
            .filter(|s| s.starts_with("LUMA_E_") && !NOT_ANSWERS.contains(&s.as_str()))
            .map(|s| case_name(&s["LUMA_E_".len()..]))
            .collect();
        let mut sfn = SFn {
            name: f.name.clone(),
            shape: Shape::NotCallable(Unfit::Form(String::new())),
            statuses: statuses.clone(),
            answers,
            any_thread: f.comment.contains("Any thread") || f.comment.contains("any thread"),
            ranges: ranges_in(&f.comment, &f.params),
            comment: f.comment.clone(),
            c: f.clone(),
        };
        let not = |why: String| Shape::NotCallable(Unfit::Form(why));
        let unfit = |param: &str, why: ParamWhy, c_type: String| Shape::NotCallable(Unfit::Param { param: param.to_string(), why, c_type });
        sfn.shape = (|| {
            if let Some(unknown) = statuses.iter().find(|s| !known.contains(s)) {
                return not(format!("its comment names {unknown}, which is no status"));
            }
            if let Some(x) = after_phrase(&f.comment, "set by on ") {
                return Shape::SetBy(x.to_string());
            }
            if let Some(kind) = after_phrase(&f.comment, "inside the ") {
                if f.comment.contains(&format!("inside the {kind} event's call")) {
                    // Its buffer says what the event's `data` is: `char*` text, else bytes.
                    let text = f.params.iter().any(|d| d.ty.base == "char" && d.ty.ptrs == 1 && !d.ty.konst);
                    return Shape::Take { kind: kind.to_ascii_lowercase(), data: if text { STy::Text } else { STy::Bytes } };
                }
            }
            match f.params.first() {
                Some(Decl { ty: CT { base, konst: true, ptrs: 1 }, .. }) if base == "LumaMod" => {}
                _ => return not("its first parameter is not const LumaMod* mod".into()),
            }
            let (mut params, mut outs, mut closure, mut handle_out) = (Vec::new(), Vec::new(), false, None);
            let ps = &f.params[1..];
            let mut i = 0;
            while i < ps.len() {
                let d = &ps[i];
                // A const array of a fixed length: 2 to 4 floats are a vec, others a tuple of their values
                // (`const LumaColor colours[3]`). An array the function writes, or of a struct the script cannot write
                // whole, is not callable (said as any unfit parameter); the table's other functions are.
                if let Some(n) = d.array {
                    let t = &d.ty;
                    let ty = if !t.konst || t.ptrs != 0 || n == 0 || n > 16 {
                        None
                    } else if t.base == "float" && (2..=4).contains(&n) {
                        Some(STy::Vec(n as u8))
                    } else {
                        own(&t.base)
                            .filter(|r| self.structs.contains_key(r))
                            .map(STy::Struct)
                            .or_else(|| self.shared.contains(&t.base).then(|| STy::Shared(t.base.clone())))
                            .or_else(|| self.value_ty(&CT { base: t.base.clone(), konst: false, ptrs: 0 }, own))
                            .filter(|e| match e {
                                STy::Struct(r) | STy::Shared(r) => !self.unreadable.iter().any(|(f, _)| f.starts_with(&format!("{r}."))),
                                _ => true,
                            })
                            .map(|e| STy::Fixed(Box::new(e), n as u8))
                    };
                    let Some(ty) = ty else { return unfit(&d.name, ParamWhy::Pointer, format!("{}[{n}]", t.base)) };
                    params.push(SParam::Value { name: d.name.clone(), ty });
                    i += 1;
                    continue;
                }
                let next_is_size = matches!(ps.get(i + 1), Some(Decl { ty: CT { base, ptrs: 0, .. }, .. }) if base == "uint64_t");
                let t = &d.ty;
                let buffer = matches!(t.base.as_str(), "char" | "void" | "uint8_t") && t.ptrs == 1;
                if let (0, Some((args, c_args))) = (t.ptrs, self.callbacks.get(&t.base)) {
                    match ps.get(i + 1) {
                        Some(Decl { ty: CT { base, konst: false, ptrs: 1 }, .. }) if base == "void" => {}
                        _ => return not(format!("{}: a {} not followed by void* user", d.name, t.base)),
                    }
                    params.push(SParam::Callback { name: d.name.clone(), c_fn: t.base.clone(), args: args.clone(), c_args: c_args.clone() });
                    i += 2;
                    continue;
                }
                if t.base == "LumaEventFn" && t.ptrs == 0 {
                    match ps.get(i + 1) {
                        Some(Decl { ty: CT { base, konst: false, ptrs: 1 }, .. }) if base == "void" => {}
                        _ => return not(format!("{}: a LumaEventFn not followed by void* user", d.name)),
                    }
                    closure = true;
                    params.push(SParam::Closure { optional: false });
                    i += 2;
                    continue;
                }
                if buffer && t.konst {
                    if t.base == "char" && !next_is_size {
                        params.push(SParam::Value { name: d.name.clone(), ty: nullable(&f.comment, &d.name, STy::Text) });
                        i += 1;
                        continue;
                    }
                    if pixels_in(ps, i) {
                        params.push(SParam::Value { name: d.name.clone(), ty: STy::Bytes });
                        i += 1;
                        continue;
                    }
                    if sized_in_32(ps, i) {
                        params.push(SParam::Value { name: d.name.clone(), ty: STy::Bytes });
                        i += 2;
                        continue;
                    }
                    if !next_is_size {
                        return unfit(&d.name, ParamWhy::Buffer, format!("{}*", t.base));
                    }
                    params.push(SParam::Value { name: d.name.clone(), ty: if t.base == "char" { STy::Text } else { STy::Bytes } });
                    i += 2;
                    continue;
                }
                if buffer {
                    if !next_is_size && !length_out_32(ps, i) {
                        return unfit(&d.name, ParamWhy::Buffer, format!("{}*", t.base));
                    }
                    // Its length comes back as the int64_t result, or in the `uint64_t* length` after the size, which
                    // is then no out of its own (C1b). Names each followed by a 0 byte are a list of texts.
                    let ty = match t.base.as_str() {
                        "char" if f.comment.contains(NAMES_PHRASE) => STy::List(Box::new(STy::Text)),
                        "char" => STy::Text,
                        _ => STy::Bytes,
                    };
                    outs.push(ty);
                    i += if length_out(ps, i) { 3 } else { 2 };
                    continue;
                }
                // Floats the comment counts (`` `rgba` (four floats ``): a vec of that many.
                if let Some(n) = floats_in(&f.comment, d) {
                    params.push(SParam::Value { name: d.name.clone(), ty: STy::Vec(n) });
                    i += 1;
                    continue;
                }
                // Numbers whose count another member gives per item (stream_push's `frames` x stream_channels): a list,
                // its count worked out at the call.
                if items_by(&f.name, ps, i, members).is_some() {
                    params.push(SParam::Value { name: d.name.clone(), ty: STy::List(Box::new(STy::Number(t.base.clone()))) });
                    i += 2;
                    continue;
                }
                // Bytes the service lends (assets.bytes' `const void** data, uint64_t* bytes`): copied as the script's.
                if lent_out(ps, i) {
                    outs.push(STy::Bytes);
                    i += 2;
                    continue;
                }
                // An array with its count (C1c): a list of a number, enum, handle or struct the script can write whole.
                if t.ptrs == 1 && (array_in(ps, i) || array_out(ps, i)) {
                    let pointee = CT { base: t.base.clone(), konst: false, ptrs: 0 };
                    let elem = own(&t.base)
                        .filter(|r| self.structs.contains_key(r))
                        .map(STy::Struct)
                        .or_else(|| self.shared.contains(&t.base).then(|| STy::Shared(t.base.clone())))
                        // Another service's struct of the same header (`input.DeviceInfo` in input.dev's list).
                        .or_else(|| self.foreign_structs.get(&t.base).map(|(o, n)| STy::StructOf(o.clone(), n.clone())))
                        .or_else(|| self.value_ty(&pointee, own));
                    let Some(elem) = elem else { return unfit(&d.name, ParamWhy::Pointer, t.base.clone()) };
                    // A struct with a field the script cannot read (a `const char*`, a pointer) would cross with that
                    // field zero: the function is not callable (assets.provide's files, player_parts' names).
                    let record = match &elem {
                        STy::Struct(n) | STy::Shared(n) => Some(format!("{n}.")),
                        _ => None,
                    };
                    if record.is_some_and(|r| self.unreadable.iter().any(|(f, _)| f.starts_with(&r))) {
                        return unfit(&d.name, ParamWhy::Pointer, t.base.clone());
                    }
                    if t.konst {
                        params.push(SParam::Value { name: d.name.clone(), ty: STy::List(Box::new(elem)) });
                        i += 2;
                    } else {
                        outs.push(STy::List(Box::new(elem)));
                        i += 3;
                    }
                    continue;
                }
                if t.ptrs == 1 {
                    let pointee = CT { base: t.base.clone(), konst: false, ptrs: 0 };
                    let struct_ty = own(&t.base)
                        .filter(|r| self.structs.contains_key(r))
                        .map(STy::Struct)
                        .or_else(|| self.shared.contains(&t.base).then(|| STy::Shared(t.base.clone())))
                        .or_else(|| self.foreign_structs.get(&t.base).filter(|_| t.konst).map(|(o, n)| STy::StructOf(o.clone(), n.clone())));
                    let ty = struct_ty.or_else(|| self.value_ty(&pointee, own));
                    let Some(ty) = ty else { return unfit(&d.name, ParamWhy::Pointer, t.base.clone()) };
                    if t.konst {
                        if !matches!(ty, STy::Struct(_) | STy::Shared(_) | STy::StructOf(..)) {
                            return unfit(&d.name, ParamWhy::Pointer, format!("const {}", t.base));
                        }
                        params.push(SParam::Value { name: d.name.clone(), ty: nullable(&f.comment, &d.name, ty) });
                    } else if matches!(ty, STy::Handle(_)) {
                        // In its place among the outs; a request takes it out again.
                        handle_out = Some((outs.len(), ty.clone()));
                        outs.push(ty);
                    } else {
                        outs.push(ty);
                    }
                    i += 1;
                    continue;
                }
                let Some(ty) = self.value_ty(t, own) else { return unfit(&d.name, ParamWhy::Type, t.base.clone()) };
                params.push(SParam::Value { name: d.name.clone(), ty });
                i += 1;
            }
            // a stream, whose `events:` line lists its kinds; LUMA_OK at once or LUMA_PENDING.
            if closure && events_line(&f.comment).is_some() {
                if outs.len() > usize::from(handle_out.is_some()) {
                    return not("a stream with outs beside its handle".into());
                }
                if !(f.result.base == "int32_t" && f.result.ptrs == 0) {
                    return not("a stream whose result is no status".into());
                }
                if statuses.is_empty() {
                    return not("its comment names no status".into());
                }
                if OPTIONAL_FN.iter().any(|p| f.comment.contains(p)) {
                    for p in &mut params {
                        if let SParam::Closure { optional } = p {
                            *optional = true;
                        }
                    }
                }
                return Shape::Callable { params, gives: Gives::Stream { handle: handle_out.map(|(_, h)| h) } };
            }
            let pending = statuses.iter().any(|s| s == "LUMA_PENDING");
            if pending {
                let (true, Some((_, handle))) = (closure, handle_out) else { return not("LUMA_PENDING without a LumaEventFn and a handle out".into()) };
                if outs.len() > 1 {
                    return not("a request with outs beside its handle".into());
                }
                // `a <KIND> event follows` (`an` before a vowel), a kind of this table's event enum.
                let kind = phrase_words(&f.comment, "a ")
                    .into_iter()
                    .chain(phrase_words(&f.comment, "an "))
                    .find(|k| f.comment.contains(&format!(" {k} event follows")))
                    .map(|k| k.to_ascii_lowercase());
                let Some(kind) = kind else { return not("a request whose comment says no `a <KIND> event follows`".into()) };
                if !self.events.contains_key(&kind) {
                    return not(format!("a {} event follows, and the table has no such event", kind.to_ascii_uppercase()));
                }
                return Shape::Callable { params, gives: Gives::Request { handle, waitable: f.comment.contains("closed after its event"), kind } };
            }
            if closure {
                return not("a LumaEventFn without LUMA_PENDING".into());
            }
            // A handle the call opens at once (content.fx's start) stays an out like any other.
            let r = &f.result;
            if r.ptrs != 0 {
                return not("a pointer result".into());
            }
            let gives = match r.base.as_str() {
                "void" => Gives::Outs(outs),
                "int32_t" if statuses.is_empty() => return not("its comment names no status".into()),
                "int32_t" if !sfn.answers.is_empty() => Gives::Answered(outs),
                "int32_t" => Gives::Outs(outs),
                // A length the out buffer's: its answers make the buffer's `T?`.
                "int64_t" if !outs.is_empty() && !sfn.answers.is_empty() => Gives::Answered(outs),
                "int64_t" if !outs.is_empty() => Gives::Outs(outs),
                n @ ("int64_t" | "uint32_t" | "uint64_t") if outs.is_empty() => Gives::Number(n.to_string()),
                other => return not(format!("a result of {other}")),
            };
            Shape::Callable { params, gives }
        })();
        sfn
    }

    /// A request event's `data`: its take's (`take_read`), or `LumaEvent.data` where the kind's comment says
    /// the event carries it (input's TEXT: "with `data` the UTF-8 text"; never "data: none"); text where it says UTF-8
    /// text. The take, when it has one.
    pub fn event_data(&self, kind: &str) -> Option<(STy, Option<&SFn>)> {
        let take = self.functions.iter().find(|f| matches!(&f.shape, Shape::Take { kind: k, .. } if k == kind));
        if let Some(t) = take {
            let Shape::Take { data, .. } = &t.shape else { return None };
            return Some((data.clone(), Some(t)));
        }
        let comment = &self.events.get(kind)?.1;
        let says = comment.contains("`data`") || comment.contains("data the ") || comment.contains("data its ") || (comment.contains("data: ") && !comment.contains("data: none"));
        says.then(|| (if comment.contains("UTF-8 text") { STy::Text } else { STy::Bytes }, None))
    }

    pub fn function(&self, name: &str) -> Option<&SFn> {
        self.functions.iter().find(|f| f.name == name)
    }
}

/// The longest `XXX_` all of an enum's cases start with (`LUMA_TIME_`), cut at an `_`.
fn case_prefix(cases: &[(String, i64, String)]) -> String {
    let Some(first) = cases.first().map(|c| c.0.as_str()) else { return String::new() };
    let mut len = first.len();
    for (c, _, _) in cases {
        len = len.min(first.bytes().zip(c.bytes()).take_while(|(a, b)| a == b).count());
    }
    // Back to the last `_` inside the common part, and never the whole of a case.
    let cut = first[..len].rfind('_').map_or(0, |u| u + 1);
    if cases.len() == 1 {
        // One case: its words but the last are the prefix.
        return first.rfind('_').map_or(String::new(), |u| first[..=u].to_string());
    }
    first[..cut].to_string()
}
