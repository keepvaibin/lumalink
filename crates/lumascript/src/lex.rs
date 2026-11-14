//! Tokens (docs/modding/language/language.md, section 2; appendix G.1). A lexical error is said and the scan goes on, so one
//! run lists every error (9.1).

use crate::ast::Pos;
use crate::diag::{code, Arg, Diagnostic};
use std::ops::Range;
use std::rc::Rc;

/// Reserved words: never a name of the mod's own. After `.` or `::` any word is a name.
pub const KEYWORDS: &[&str] = &[
    "use", "pub", "let", "var", "fn", "return", "if", "else", "while", "for", "in", "break", "continue", "match", "struct", "enum",
    "class", "override", "hook", "wrap", "replace", "proceed", "skip", "state", "states", "go", "wait", "task", "stop", "on",
    "option", "true", "false", "none", "self", "result", "as", "and", "or", "not", "extern", "export", "defer", "test", "expect",
    "guard", "start", "log", "ref",
];

/// The content words: a content literal only when a string follows.
pub const CONTENT: &[&str] = &["model", "effect", "sound", "layout", "text", "font", "movie", "data", "nerve"];

/// Operators, longest first.
pub(crate) const OPERATORS: &[&str] = &[
    "<<=", ">>=", "..=", "??", "?.", "==", "!=", "<=", ">=", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "->", "=>", "..",
    "::", "+", "-", "*", "/", "%", "&", "|", "^", "<", ">", "=", "!", "?", ".", ",", ":", ";", "(", ")", "[", "]", "{", "}", "~",
];

/// A line end after one of these is no end. Not `>` and `>>`, which also end a type (`items: list<int>`,
/// `map<text, list<int>>`) where a line end ends the statement: after them as operators the parser skips it.
pub(crate) const CONTINUES: &[&str] = &[
    "??", "==", "!=", "<=", ">=", "<<", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "->", "=>", "..", "..=", "+", "-",
    "*", "/", "%", "&", "|", "^", "<", "=", ",", ":", "(", "[", "{", "and", "or", "not", "?.",
];

const SUFFIXES: &[&str] = &["f32", "f64", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A line end that ends a statement.
    Nl,
    Eof,
    Op,
    Kw,
    Id,
    /// A backquoted game name; `text` without the backquotes.
    Game,
    Num,
    Color,
    Str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tok {
    pub kind: Kind,
    /// The token as written; empty for a string, whose place in the file is [`StrTok::whole`].
    pub text: String,
    pub at: Pos,
    /// A string's pieces.
    pub string: Option<Box<StrTok>>,
}

/// A string: its pieces, and where it and they stand in the file's characters, which every string of the file shares,
/// so strings inside strings cost no copies (a file's memory is a few times its size, however deep it nests).
#[derive(Clone, Debug, PartialEq)]
pub struct StrTok {
    pub prefix: crate::ast::StrPrefix,
    pub parts: Vec<RawPart>,
    pub source: Rc<[char]>,
    /// The whole string, quotes included.
    pub whole: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RawPart {
    /// Text, escapes as written; `{{` already one brace.
    Text(String),
    /// An interpolated expression, its characters in [`StrTok::source`] and its place. The parser reads it
    /// ([`sub_tokens`]) when it reads the string, so a value nested in strings counts toward the one depth limit
    /// (S0022) and nothing here recurses.
    Expr { span: Range<usize>, at: Pos },
}

impl Tok {
    fn new(kind: Kind, text: impl Into<String>, at: Pos) -> Tok {
        Tok { kind, text: text.into(), at, string: None }
    }

    pub fn is(&self, value: &str) -> bool {
        matches!(self.kind, Kind::Op | Kind::Kw) && self.text == value
    }

    /// How a diagnostic names it.
    pub fn found(&self) -> Arg {
        match self.kind {
            Kind::Nl => Arg::Phrase("Script.Found.LineEnd"),
            Kind::Eof => Arg::Phrase("Script.Found.FileEnd"),
            Kind::Str => match &self.string {
                Some(s) => Arg::Text(format!("`{}`", s.source[s.whole.clone()].iter().collect::<String>())),
                None => Arg::Text(String::from("``")),
            },
            _ => Arg::Text(format!("`{}`", self.text)),
        }
    }
}

pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The brackets open at a point, the lexer's and the parser's recovery alike. A closer closes back to its own opener
/// (a `}` after an unclosed `(` still ends the block); one with no opener changes nothing. Each bracket goes on and off
/// once, so a file is followed in time linear in its length, however its brackets nest or fail to.
#[derive(Default)]
pub(crate) struct Brackets {
    stack: Vec<char>,
    /// How many of `(`, `[`, `{` are open.
    count: [usize; 3],
}

impl Brackets {
    fn slot(open: char) -> usize {
        match open {
            '(' => 0,
            '[' => 1,
            _ => 2,
        }
    }

    /// Follows one token's text: an opener or a closer; anything else changes nothing.
    pub(crate) fn take(&mut self, text: &str) {
        let open = match text {
            "(" | "[" | "{" => {
                let open = text.chars().next().unwrap_or('(');
                self.count[Self::slot(open)] += 1;
                return self.stack.push(open);
            }
            ")" => '(',
            "]" => '[',
            "}" => '{',
            _ => return,
        };
        if self.count[Self::slot(open)] == 0 {
            return;
        }
        while let Some(b) = self.stack.pop() {
            self.count[Self::slot(b)] -= 1;
            if b == open {
                break;
            }
        }
    }

    pub(crate) fn innermost(&self) -> Option<char> {
        self.stack.last().copied()
    }

    pub(crate) fn in_braces(&self) -> bool {
        self.count[2] > 0
    }
}

struct Lexer<'a> {
    /// The file's characters, shared with every string token.
    shared: &'a Rc<[char]>,
    /// What this lexer reads: the file up to the end of its part (an interpolation's or the whole), from `i` on.
    src: &'a [char],
    i: usize,
    line: u32,
    col: u32,
    toks: Vec<Tok>,
    diags: &'a mut Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn here(&self) -> Pos {
        Pos { line: self.line, col: self.col }
    }

    fn at(&self, k: usize) -> Option<char> {
        self.src.get(self.i + k).copied()
    }

    fn starts(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, c)| self.at(k) == Some(c))
    }

    fn advance(&mut self, k: usize) {
        for _ in 0..k {
            if self.i >= self.src.len() {
                return;
            }
            if self.src[self.i] == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            self.i += 1;
        }
    }

    fn text(&self, from: usize, to: usize) -> String {
        self.src[from..to].iter().collect()
    }

    fn error(&mut self, code: u16, at: Pos) {
        self.diags.push(Diagnostic::new(code, at));
    }

    fn run(&mut self) {
        let mut open = Brackets::default();
        while let Some(c) = self.at(0) {
            if c == '\n' {
                let at = self.here();
                self.advance(1);
                if open.innermost().is_none_or(|b| b == '{') {
                    self.toks.push(Tok::new(Kind::Nl, "\n", at));
                }
                continue;
            }
            if c == ' ' || c == '\t' {
                self.advance(1);
                continue;
            }
            if self.starts("//") {
                while self.at(0).is_some_and(|c| c != '\n') {
                    self.advance(1);
                }
                continue;
            }
            let at = self.here();
            if self.starts("/*") {
                let mut depth = 0u32;
                loop {
                    if self.at(0).is_none() {
                        self.error(code::NO_COMMENT_END, at);
                        break;
                    }
                    if self.starts("/*") {
                        depth += 1;
                        self.advance(2);
                    } else if self.starts("*/") {
                        depth -= 1;
                        self.advance(2);
                        if depth == 0 {
                            break;
                        }
                    } else {
                        self.advance(1);
                    }
                }
                continue;
            }
            if c == '`' {
                let start = self.i + 1;
                let mut j = start;
                while j < self.src.len() && self.src[j] != '`' && self.src[j] != '\n' {
                    j += 1;
                }
                let name = self.text(start, j);
                if j >= self.src.len() || self.src[j] == '\n' {
                    self.error(code::NO_CLOSING_BACKQUOTE, at);
                    self.advance(j - self.i);
                } else {
                    self.advance(j + 1 - self.i);
                }
                self.toks.push(Tok::new(Kind::Game, name, at));
                continue;
            }
            if c == '#' {
                let mut h = 0;
                while self.at(1 + h).is_some_and(|d| d.is_ascii_hexdigit()) {
                    h += 1;
                }
                if (h == 6 || h == 8) && !self.at(1 + h).is_some_and(is_word) {
                    let text = self.text(self.i, self.i + 1 + h);
                    self.advance(1 + h);
                    self.toks.push(Tok::new(Kind::Color, text, at));
                    continue;
                }
            }
            if c.is_ascii_digit() {
                let n = self.number_length();
                let text = self.text(self.i, self.i + n);
                self.advance(n);
                self.toks.push(Tok::new(Kind::Num, text, at));
                continue;
            }
            if c == '"' || ((c == 'r' || c == 'b') && self.at(1) == Some('"')) {
                self.string(c, at);
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let mut n = 1;
                while self.at(n).is_some_and(is_word) {
                    n += 1;
                }
                let word = self.text(self.i, self.i + n);
                self.advance(n);
                if !word.is_ascii() {
                    // Names are ASCII (section 2): said once for the whole word, which reads on as a name.
                    self.diags.push(Diagnostic::new(code::NON_ASCII_NAME, at).with("found", Arg::Text(format!("`{word}`"))));
                    self.toks.push(Tok::new(Kind::Id, word, at));
                    continue;
                }
                let kind = if KEYWORDS.contains(&word.as_str()) { Kind::Kw } else { Kind::Id };
                self.toks.push(Tok::new(kind, word, at));
                continue;
            }
            if let Some(op) = OPERATORS.iter().find(|op| self.starts(op)) {
                self.advance(op.chars().count());
                open.take(op);
                self.toks.push(Tok::new(Kind::Op, *op, at));
                continue;
            }
            self.diags.push(Diagnostic::new(code::NOT_A_TOKEN, at).with("found", Arg::Text(format!("`{c}`"))));
            self.advance(1);
        }
        let end = self.here();
        self.toks.push(Tok::new(Kind::Nl, "\n", end));
        self.toks.push(Tok::new(Kind::Eof, "", end));
    }

    /// The length of the number at `i` (G.1): `0x..`, `0b..`, digits with a fraction, an exponent and a suffix; only
    /// digits right after `.` (a tuple's field, `p.0`).
    fn number_length(&self) -> usize {
        let after_dot = self.toks.last().is_some_and(|t| t.kind == Kind::Op && t.text == ".");
        let digit = |k: usize| self.at(k).is_some_and(|c| c.is_ascii_digit());
        if after_dot {
            let mut n = 0;
            while digit(n) {
                n += 1;
            }
            return n;
        }
        for (lead, ok) in [("0x", (|c: char| c.is_ascii_hexdigit() || c == '_') as fn(char) -> bool), ("0b", |c: char| c == '0' || c == '1' || c == '_')] {
            if self.starts(lead) && self.at(2).is_some_and(ok) {
                let mut n = 3;
                while self.at(n).is_some_and(ok) {
                    n += 1;
                }
                return n;
            }
        }
        let run = |mut n: usize| {
            while self.at(n).is_some_and(|c| c.is_ascii_digit() || c == '_') {
                n += 1;
            }
            n
        };
        let mut n = run(1);
        if self.at(n) == Some('.') && digit(n + 1) {
            n = run(n + 2);
        }
        if matches!(self.at(n), Some('e' | 'E')) {
            let sign = usize::from(matches!(self.at(n + 1), Some('+' | '-')));
            if digit(n + 1 + sign) {
                n += 1 + sign;
                while digit(n) {
                    n += 1;
                }
            }
        }
        // The first suffix that matches, as the grammar's alternation takes it.
        if let Some(s) = SUFFIXES.iter().find(|s| s.chars().enumerate().all(|(k, c)| self.at(n + k) == Some(c))) {
            n += s.len();
        }
        n
    }

    fn string(&mut self, c: char, at: Pos) {
        use crate::ast::StrPrefix;
        let prefix = match c {
            'r' => StrPrefix::Raw,
            'b' => StrPrefix::Bytes,
            _ => StrPrefix::None,
        };
        let open = self.i;
        let start = self.i + if prefix == StrPrefix::None { 1 } else { 2 };
        let mut j = start;
        let mut parts = Vec::new();
        let mut buf = String::new();
        let mut closed = false;
        let mut said = false;
        while j < self.src.len() && self.src[j] != '\n' {
            let ch = self.src[j];
            if ch == '"' {
                closed = true;
                break;
            }
            if ch == '\\' && prefix != StrPrefix::Raw {
                buf.push(ch);
                if let Some(&next) = self.src.get(j + 1).filter(|&&n| n != '\n') {
                    buf.push(next);
                    j += 2;
                    // `\u{...}`: its braces are the escape's, not an interpolation; the checker reads the
                    // digits (BAD_ESCAPE). At most nine characters to the `}`, on this line.
                    if next == 'u' && self.src.get(j) == Some(&'{') {
                        let close = (j..self.src.len().min(j + 10)).take_while(|&k| self.src[k] != '\n').find(|&k| self.src[k] == '}');
                        if let Some(close) = close {
                            buf.extend(&self.src[j..=close]);
                            j = close + 1;
                        }
                    }
                } else {
                    j += 1;
                }
                continue;
            }
            if ch == '{' && prefix == StrPrefix::None {
                if self.src.get(j + 1) == Some(&'{') {
                    buf.push('{');
                    j += 2;
                    continue;
                }
                // The expression runs to its matching brace on this line (G.1: a string is one line); a string
                // inside it is skipped whole, also on this line.
                let (mut depth, mut k) = (1u32, j + 1);
                while k < self.src.len() && self.src[k] != '\n' && depth > 0 {
                    match self.src[k] {
                        '"' => {
                            k += 1;
                            while k < self.src.len() && self.src[k] != '"' && self.src[k] != '\n' {
                                k += if self.src[k] == '\\' && self.src.get(k + 1).is_some_and(|&n| n != '\n') { 2 } else { 1 };
                            }
                            if self.src.get(k) != Some(&'"') {
                                break;
                            }
                        }
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    k += 1;
                }
                if depth > 0 {
                    self.error(code::NO_INTERPOLATION_END, self.pos_of(open, at, j));
                    said = true;
                    // Nothing more of this line reads as the string.
                    while j < self.src.len() && self.src[j] != '\n' {
                        j += 1;
                    }
                    break;
                }
                parts.push(RawPart::Text(std::mem::take(&mut buf)));
                let inner_at = self.pos_of(open, at, j + 1);
                parts.push(RawPart::Expr { span: j + 1..k - 1, at: inner_at });
                j = k;
                continue;
            }
            buf.push(ch);
            j += 1;
        }
        parts.push(RawPart::Text(buf));
        if !closed && !said {
            self.error(code::NO_CLOSING_QUOTE, at);
        }
        let end = (if closed { j + 1 } else { j }).min(self.src.len());
        self.advance(end - self.i);
        let string = StrTok { prefix, parts, source: Rc::clone(self.shared), whole: open..end };
        self.toks.push(Tok { kind: Kind::Str, text: String::new(), at, string: Some(Box::new(string)) });
    }

    /// The place of character `k` of a token that starts at character `open`, which is at `at`.
    fn pos_of(&self, open: usize, at: Pos, k: usize) -> Pos {
        let mut p = at;
        for &ch in &self.src[open..k.min(self.src.len())] {
            if ch == '\n' {
                p.line += 1;
                p.col = 1;
            } else {
                p.col += 1;
            }
        }
        p
    }
}

/// An interpolated expression's tokens (characters `span` of `source`, at `at`): no line ends, then Eof.
pub fn sub_tokens(source: &Rc<[char]>, span: Range<usize>, at: Pos, diags: &mut Vec<Diagnostic>) -> Vec<Tok> {
    let mut toks: Vec<Tok> = raw(source, span, at, diags).into_iter().filter(|t| t.kind != Kind::Nl).collect();
    if toks.last().is_none_or(|t| t.kind != Kind::Eof) {
        toks.push(Tok::new(Kind::Eof, "", at));
    }
    toks
}

fn raw(shared: &Rc<[char]>, span: Range<usize>, at: Pos, diags: &mut Vec<Diagnostic>) -> Vec<Tok> {
    let mut lx = Lexer { shared, src: &shared[..span.end], i: span.start, line: at.line, col: at.col, toks: Vec::new(), diags };
    lx.run();
    lx.toks
}

/// The tokens of a file, with the line ends that end statements: none after an operator but `>` and `>>`, nor
/// after `,`, `:`, `->`, `=>` or an open bracket, none before `else` (but before `else =>`), and none twice. One
/// leading byte-order mark is skipped; CR LF and a lone CR each end a line.
pub fn lex(src: &str, diags: &mut Vec<Diagnostic>) -> Vec<Tok> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src).replace("\r\n", "\n").replace('\r', "\n");
    let chars: Rc<[char]> = src.chars().collect();
    let toks = raw(&chars, 0..chars.len(), Pos { line: 1, col: 1 }, diags);
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    for (k, t) in toks.iter().enumerate() {
        if t.kind == Kind::Nl {
            if let Some(prev) = out.last() {
                if matches!(prev.kind, Kind::Op | Kind::Kw) && CONTINUES.contains(&prev.text.as_str()) {
                    continue;
                }
            }
            if let Some(n) = (k + 1..toks.len()).find(|&n| toks[n].kind != Kind::Nl) {
                if toks[n].kind == Kind::Kw && toks[n].text == "else" && toks.get(n + 1).is_none_or(|a| a.text != "=>") {
                    continue;
                }
            }
            if out.last().is_some_and(|p| p.kind == Kind::Nl) {
                continue;
            }
        }
        out.push(t.clone());
    }
    out
}
