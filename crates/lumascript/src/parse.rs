//! The parser (language appendix G; `ls_parse.py` was its first executable form and this follows it rule for rule).
//! An error is said with its place and what was expected; the parser then skips to the end of that statement or item
//! and goes on, so one run lists every error (9.1).

use crate::ast::*;
use crate::diag::{code, Arg as Say, Diagnostic};
use crate::lex::{sub_tokens, Brackets, Kind, RawPart, Tok, CONTENT};
use crate::MAX_DEPTH;

type R<T> = Result<T, Diagnostic>;

const ASSIGN: &[(&str, AssignOp)] = &[
    ("=", AssignOp::Set),
    ("+=", AssignOp::Add),
    ("-=", AssignOp::Sub),
    ("*=", AssignOp::Mul),
    ("/=", AssignOp::Div),
    ("%=", AssignOp::Rem),
    ("&=", AssignOp::And),
    ("|=", AssignOp::Or),
    ("^=", AssignOp::Xor),
    ("<<=", AssignOp::Shl),
    (">>=", AssignOp::Shr),
];

/// Binary operators by level, loosest first (G.4); None is `not`'s level.
const LEVELS: &[Option<&[(&str, BinOp)]>] = &[
    Some(&[("??", BinOp::Coalesce)]),
    Some(&[("or", BinOp::Or)]),
    Some(&[("and", BinOp::And)]),
    None,
    Some(&[("==", BinOp::Eq), ("!=", BinOp::Ne), ("<", BinOp::Lt), ("<=", BinOp::Le), (">", BinOp::Gt), (">=", BinOp::Ge)]),
    Some(&[("..", BinOp::Range), ("..=", BinOp::RangeIncl)]),
    Some(&[("|", BinOp::BitOr)]),
    Some(&[("^", BinOp::BitXor)]),
    Some(&[("&", BinOp::BitAnd)]),
    Some(&[("<<", BinOp::Shl), (">>", BinOp::Shr)]),
    Some(&[("+", BinOp::Add), ("-", BinOp::Sub)]),
    Some(&[("*", BinOp::Mul), ("/", BinOp::Div), ("%", BinOp::Rem)]),
];
const NOT_LEVEL: usize = 3;
const COMPARE_LEVEL: usize = 4;

const BLOCK_ONLY_AT_TOP: &[&str] = &[
    "hook", "wrap", "replace", "on", "states", "struct", "enum", "class", "task", "use", "option", "pub", "test", "extern", "export",
];

fn want(phrase: &'static str) -> Say {
    Say::Phrase(phrase)
}

fn token(text: &str) -> Say {
    Say::Text(format!("`{text}`"))
}

fn upper(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

pub struct Parser<'a> {
    t: Vec<Tok>,
    i: usize,
    /// Tokens changed while parsing (`>>` read as two `>`), to put back when a guess is undone.
    undo: Vec<(usize, Tok)>,
    /// Where the last block's `}` was taken (the token after it): a statement ending there needs no line end.
    after_block: usize,
    /// How deep the tree being built is here (crate::MAX_DEPTH): each counted place adds one and puts it back.
    depth: u32,
    /// The next expression id (ast::ExprId): unique in the module, carried into interpolations' parsers.
    ids: u32,
    diags: &'a mut Vec<Diagnostic>,
    /// Reading a library's interface file (`<mod id>.lumi`, docs/modding/language/compiled-form.md): a function is its head
    /// alone, with no body.
    pub interface: bool,
}

impl<'a> Parser<'a> {
    fn new_id(&mut self) -> ExprId {
        let id = self.ids;
        self.ids += 1;
        id
    }

    pub fn new(t: Vec<Tok>, diags: &'a mut Vec<Diagnostic>) -> Self {
        Parser { t, i: 0, undo: Vec::new(), after_block: usize::MAX, depth: 0, ids: 0, diags, interface: false }
    }

    // ------------------------------------------------------------ depth (S0022)

    /// `f` one level deeper; past [`MAX_DEPTH`] it is S0022 at `at` instead. The depth is as it was afterwards, error
    /// or not.
    fn nested<T>(&mut self, at: Pos, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        if self.depth >= MAX_DEPTH {
            return Err(Diagnostic::too_deep(at));
        }
        let base = self.depth;
        self.depth += 1;
        let out = f(self);
        self.depth = base;
        out
    }

    /// `f`, with the depth put back afterwards, error or not: for a loop that wraps each node around the one before.
    fn keeping_depth<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let base = self.depth;
        let out = f(self);
        self.depth = base;
        out
    }

    /// One level deeper for a node a loop wraps around the one before (`a + b + c`, `f()()`, `x as A as B`): the
    /// tree's depth, which each later pass walks by recursion, is held to the limit as the parser's own stack is.
    /// Only inside [`Self::keeping_depth`].
    fn wrap_deeper(&mut self, at: Pos) -> R<()> {
        if self.depth >= MAX_DEPTH {
            return Err(Diagnostic::too_deep(at));
        }
        self.depth += 1;
        Ok(())
    }

    // ------------------------------------------------------------ helpers

    fn peek(&self, k: usize) -> &Tok {
        let last = self.t.len() - 1;
        &self.t[(self.i + k).min(last)]
    }

    fn at(&self, value: &str) -> bool {
        self.peek(0).is(value)
    }

    fn at_any(&self, values: &[&str]) -> bool {
        values.iter().any(|v| self.at(v))
    }

    /// A word, reserved or a name (the contextual words).
    fn at_word(&self, value: &str) -> bool {
        let t = self.peek(0);
        matches!(t.kind, Kind::Id | Kind::Kw) && t.text == value
    }

    fn eof(&self) -> bool {
        self.peek(0).kind == Kind::Eof
    }

    /// Inside a list that `close` ends: neither at it nor at the end of the file.
    fn open_until(&self, close: &str) -> bool {
        !self.at(close) && !self.eof()
    }

    fn take(&mut self) -> Tok {
        let t = self.peek(0).clone();
        if self.i < self.t.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn fail(&self, expected: Say) -> Diagnostic {
        let t = self.peek(0);
        Diagnostic::new(code::UNEXPECTED, t.at).with("found", t.found()).with("expected", expected)
    }

    fn expect(&mut self, value: &str) -> R<Tok> {
        if !self.at(value) {
            return Err(self.fail(token(value)));
        }
        Ok(self.take())
    }

    fn expect_word(&mut self, value: &str) -> R<Tok> {
        if !self.at_word(value) {
            return Err(self.fail(token(value)));
        }
        Ok(self.take())
    }

    fn ident(&mut self, what: &'static str) -> R<Ident> {
        let t = self.peek(0);
        if t.kind != Kind::Id {
            return Err(self.fail(want(what)));
        }
        if t.text == "_" {
            return Err(Diagnostic::new(code::UNDERSCORE_NAME, t.at));
        }
        let t = self.take();
        Ok(Ident { name: t.text, at: t.at })
    }

    /// A part of a `use` path: any word, reserved or not (`use kit.player.state`), never `_`.
    fn path_word(&mut self, what: &'static str) -> R<Ident> {
        let t = self.peek(0);
        if t.kind == Kind::Id && t.text == "_" {
            return Err(Diagnostic::new(code::UNDERSCORE_NAME, t.at));
        }
        let t = self.any_word(what)?;
        Ok(Ident { name: t.text, at: t.at })
    }

    /// Any word, reserved or not (after `::`).
    fn any_word(&mut self, what: &'static str) -> R<Tok> {
        if !matches!(self.peek(0).kind, Kind::Id | Kind::Kw) {
            return Err(self.fail(want(what)));
        }
        Ok(self.take())
    }

    fn nls(&mut self) {
        while self.peek(0).kind == Kind::Nl || self.at(";") {
            self.take();
        }
    }

    /// The end of a statement or item: a line end or `;`, or nothing after a block's `}` (D4.2: a block's, not a
    /// literal's or a declaration's).
    fn end(&mut self) -> R<()> {
        if self.peek(0).kind == Kind::Nl || self.at(";") {
            self.nls();
            Ok(())
        } else if self.after_block == self.i {
            Ok(())
        } else if !self.at("}") && !self.eof() {
            Err(self.fail(want("Script.Want.LineEnd")))
        } else {
            Ok(())
        }
    }

    /// Between entries of a brace list: `,` or a line end, any number; whether one was there.
    fn sep(&mut self) -> bool {
        let mut took = false;
        while self.at(",") || self.at(";") || self.peek(0).kind == Kind::Nl {
            self.take();
            took = true;
        }
        took
    }

    /// After an error in what started at `start`: skip to the end of it (its line end or `;` once its own braces are
    /// closed), stopping before a `}` that closes the block around it. A closer goes back to its own opener, so an
    /// unclosed `(` does not take the block's `}` with it.
    fn recover(&mut self, start: usize) {
        let mut open = Brackets::default();
        for t in &self.t[start..self.i] {
            if t.kind == Kind::Op {
                open.take(&t.text);
            }
        }
        loop {
            let t = self.peek(0);
            if t.kind == Kind::Eof {
                break;
            }
            let in_braces = open.in_braces();
            if !in_braces && (t.kind == Kind::Nl || t.is(";")) {
                self.nls();
                break;
            }
            if t.is("}") && !in_braces {
                break;
            }
            if t.kind == Kind::Op {
                open.take(&t.text);
            }
            self.take();
        }
        if self.i == start && !self.eof() {
            self.take();
        }
    }

    fn record(&mut self, d: Diagnostic, start: usize) {
        self.diags.push(d);
        self.recover(start);
    }

    // ------------------------------------------------------------ module (G.2)

    pub fn module(&mut self) -> Module {
        let mut items = Vec::new();
        self.nls();
        while !self.eof() {
            let start = self.i;
            match self.item() {
                Ok(item) => {
                    items.push(item);
                    if let Err(d) = self.end() {
                        self.record(d, start);
                    }
                }
                Err(d) => self.record(d, start),
            }
        }
        Module { items }
    }

    /// A file of statements (a program written as a function body).
    pub fn statements(&mut self) -> Vec<Stmt> {
        let mut out = Vec::new();
        self.nls();
        while !self.eof() {
            self.statement_into(&mut out);
        }
        out
    }

    fn statement_into(&mut self, out: &mut Vec<Stmt>) {
        let start = self.i;
        match self.statement() {
            Ok(s) => {
                out.push(s);
                if let Err(d) = self.end() {
                    self.record(d, start);
                }
            }
            Err(d) => self.record(d, start),
        }
    }

    fn item(&mut self) -> R<Item> {
        let at = self.peek(0).at;
        let mut public = false;
        if self.at("use") {
            return Ok(Item { at, public, kind: ItemKind::Use(self.use_()?) });
        }
        if self.at("option") {
            self.take();
            let key = self.ident("Script.Want.Key")?;
            self.expect(":")?;
            let ty = self.ty()?;
            return Ok(Item { at, public, kind: ItemKind::Option { key, ty } });
        }
        if self.at("pub") {
            self.take();
            public = true;
            let c_struct = self.at_word("c") && self.peek(1).is("struct");
            if !c_struct && !self.at_any(&["let", "var", "fn", "struct", "enum", "class", "task"]) {
                return Err(self.fail(want("Script.Want.AfterPub")));
            }
        }
        let kind = if self.at("let") || self.at("var") {
            ItemKind::Binding(Box::new(self.binding()?))
        } else if self.at("fn") {
            ItemKind::Fn(self.fn_decl(false)?)
        } else if self.at_word("c") && self.peek(1).is("struct") {
            self.take();
            ItemKind::Struct(self.struct_decl(true)?)
        } else if self.at("struct") {
            ItemKind::Struct(self.struct_decl(false)?)
        } else if self.at("enum") {
            ItemKind::Enum(self.enum_decl()?)
        } else if self.at("class") {
            ItemKind::Class(self.class_decl()?)
        } else if self.at("task") {
            self.take();
            let name = self.ident("Script.Want.Name")?;
            let params = self.params(false, false)?;
            let ret = if self.at("->") {
                self.take();
                Some(self.ty()?)
            } else {
                None
            };
            // An interface's task has no body, as its functions.
            let body = if self.interface && !self.at("{") {
                let at = name.at;
                Block { at, stmts: Vec::new(), end: at }
            } else {
                self.block()?
            };
            ItemKind::Task { name, params, ret, body }
        } else if self.at_any(&["hook", "wrap", "replace"]) {
            ItemKind::Hook(self.hook()?)
        } else if self.at("on") {
            ItemKind::On(self.on()?)
        } else if self.at("states") {
            ItemKind::States(self.states()?)
        } else if self.at("test") {
            self.take();
            let game = self.at_word("game");
            if game {
                self.take();
            }
            let name = self.string()?;
            let body = self.block()?;
            ItemKind::Test { game, name, body }
        } else if self.at("extern") || self.at("export") {
            let export = self.take().text == "export";
            self.expect_word("c")?;
            self.expect("fn")?;
            let name = self.ident("Script.Want.Name")?;
            let params = self.params(false, false)?;
            let ret = if self.at("->") {
                self.take();
                Some(self.ty()?)
            } else {
                None
            };
            if export {
                let body = self.block()?;
                ItemKind::Export { name, params, ret, body }
            } else {
                ItemKind::Extern { name, params, ret }
            }
        } else {
            return Err(self.fail(want("Script.Want.Item")));
        };
        Ok(Item { at, public, kind })
    }

    fn use_(&mut self) -> R<Use> {
        self.take();
        if self.at_word("c") && self.peek(1).kind == Kind::Str {
            self.take();
            return Ok(Use::C(self.string()?));
        }
        let optional = self.at_word("optional");
        if optional {
            self.take();
        }
        // ( "." "/" IDENT { "/" IDENT } | IDENT { "." IDENT } ) [ "." "{" IDENT { "," IDENT } "}" ] [ "as" IDENT ]
        let local = self.at(".") && self.peek(1).is("/");
        let mut path = Vec::new();
        if local {
            self.take();
            self.take();
            path.push(self.path_word("Script.Want.Module")?);
            while self.at("/") {
                self.take();
                path.push(self.path_word("Script.Want.Module")?);
            }
        } else {
            path.push(self.path_word("Script.Want.Module")?);
            while self.at(".") && !self.peek(1).is("{") {
                self.take();
                path.push(self.path_word("Script.Want.Module")?);
            }
        }
        let mut names = Vec::new();
        if self.at(".") {
            self.take();
            self.expect("{")?;
            names.push(self.ident("Script.Want.Name")?);
            while self.at(",") {
                self.take();
                names.push(self.ident("Script.Want.Name")?);
            }
            self.expect("}")?;
        }
        let alias = if self.at("as") {
            self.take();
            Some(self.ident("Script.Want.Name")?)
        } else {
            None
        };
        Ok(Use::Module { optional, local, path, names, alias })
    }

    fn binding(&mut self) -> R<Binding> {
        let t = self.take();
        let var = t.text == "var";
        let pattern = if var {
            let name = self.ident("Script.Want.Name")?;
            Pattern { at: name.at, kind: PatternKind::Path { path: vec![name], args: None, fields: None } }
        } else {
            self.pattern(false)?
        };
        let ty = if self.at(":") {
            self.take();
            Some(self.ty()?)
        } else {
            None
        };
        let value = if self.at("=") {
            self.take();
            Some(self.expr(false)?)
        } else if !var || ty.is_none() {
            return Err(self.fail(want("Script.Want.Value")));
        } else {
            None
        };
        Ok(Binding { at: t.at, var, pattern, ty, value })
    }

    fn fn_decl(&mut self, method: bool) -> R<FnDecl> {
        self.take();
        let name = self.ident("Script.Want.Name")?;
        let params = self.params(method, false)?;
        let ret = if self.at("->") {
            self.take();
            Some(self.ty()?)
        } else {
            None
        };
        // An interface's function has no body: an empty one stands for it (nothing reads it; the library's build
        // checked the real one).
        if self.interface && !self.at("{") {
            let at = name.at;
            return Ok(FnDecl { name, params, ret, body: Block { at, stmts: Vec::new(), end: at } });
        }
        let body = self.block()?;
        Ok(FnDecl { name, params, ret, body })
    }

    /// `params` (G.2), or with `loose` a hook's or an override's `lparams`: names, each with its type or leaving it to
    /// the game's, and nothing more.
    fn params(&mut self, method: bool, loose: bool) -> R<Vec<Param>> {
        let mut n = 0;
        self.list("(", ")", |p| {
            n += 1;
            p.param(method && n == 1, loose)
        })
    }

    fn param(&mut self, may_be_self: bool, loose: bool) -> R<Param> {
        let at = self.peek(0).at;
        let var = !loose && self.at("var");
        if var {
            self.take();
        }
        if self.at("self") {
            if !may_be_self {
                return Err(Diagnostic::new(code::SELF_NOT_FIRST, at));
            }
            self.take();
            return Ok(Param { at, var, is_self: true, name: None, ty: None, default: None });
        }
        // A hook's parameter it does not read (`(id, _)`): no name, no type.
        if loose && !var && self.peek(0).kind == Kind::Id && self.peek(0).text == "_" {
            self.take();
            return Ok(Param { at, var: false, is_self: false, name: None, ty: None, default: None });
        }
        let name = self.ident("Script.Want.Param")?;
        let ty = if self.at(":") {
            self.take();
            Some(self.ty()?)
        } else if !loose {
            return Err(self.fail(want("Script.Want.ParamType")));
        } else {
            None
        };
        let default = if !loose && self.at("=") {
            self.take();
            Some(self.expr(false)?)
        } else {
            None
        };
        Ok(Param { at, var, is_self: false, name: Some(name), ty, default })
    }

    /// `open item { "," item } close`, or `open close`; no comma before the closer (G.3, G.4).
    fn list<T>(&mut self, open: &str, close: &str, mut item: impl FnMut(&mut Self) -> R<T>) -> R<Vec<T>> {
        self.expect(open)?;
        let mut out = Vec::new();
        if !self.at(close) {
            loop {
                out.push(item(self)?);
                if !self.at(",") {
                    break;
                }
                self.take();
            }
        }
        self.expect(close)?;
        Ok(out)
    }

    fn struct_decl(&mut self, c: bool) -> R<StructDecl> {
        self.take();
        let name = self.ident("Script.Want.Name")?;
        self.expect("{")?;
        self.sep();
        let (mut fields, mut fns) = (Vec::new(), Vec::new());
        while self.open_until("}") {
            if self.at("fn") {
                fns.push(self.fn_decl(true)?);
            } else {
                fields.push(self.field()?);
            }
            if !self.sep() && !self.at("}") {
                return Err(self.fail(want("Script.Want.FieldSep")));
            }
        }
        self.expect("}")?;
        Ok(StructDecl { c, name, fields, fns })
    }

    fn field(&mut self) -> R<Field> {
        let name = self.ident("Script.Want.Field")?;
        self.expect(":")?;
        let ty = self.ty()?;
        let default = if self.at("=") {
            self.take();
            Some(self.expr(false)?)
        } else {
            None
        };
        Ok(Field { name, ty, default })
    }

    fn enum_decl(&mut self) -> R<EnumDecl> {
        self.take();
        let name = self.ident("Script.Want.Name")?;
        self.expect("{")?;
        self.sep();
        let mut cases = Vec::new();
        while self.open_until("}") {
            let case = self.ident("Script.Want.Case")?;
            let mut fields = Vec::new();
            let mut braces = false;
            if self.at("(") || self.at("{") {
                braces = self.take().text == "{";
                let close = if braces { "}" } else { ")" };
                self.sep();
                while self.open_until(close) {
                    let name = self.ident("Script.Want.Field")?;
                    self.expect(":")?;
                    let ty = self.ty()?;
                    fields.push(Field { name, ty, default: None });
                    if !self.sep() && !self.at(close) {
                        return Err(self.fail(want("Script.Want.FieldSep")));
                    }
                }
                self.expect(close)?;
            }
            cases.push(Case { name: case, braces, fields });
            if !self.sep() && !self.at("}") {
                return Err(self.fail(want("Script.Want.CaseSep")));
            }
        }
        self.expect("}")?;
        Ok(EnumDecl { name, cases })
    }

    fn class_decl(&mut self) -> R<ClassDecl> {
        self.take();
        let name = self.ident("Script.Want.Name")?;
        self.expect(":")?;
        let base = self.game_name()?;
        // Contextual words, in this order only: `made by <row>`, then `replaces <name>`.
        let maker = if self.at_word("made") {
            self.take();
            self.expect_word("by")?;
            Some(self.game_name()?)
        } else {
            None
        };
        let replaces = if self.at_word("replaces") {
            self.take();
            Some(self.game_name()?)
        } else {
            None
        };
        if replaces.is_some() && self.at_word("made") {
            return Err(Diagnostic::new(code::MADE_BY_AFTER_REPLACES, self.peek(0).at));
        }
        self.expect("{")?;
        self.nls();
        let mut members = Vec::new();
        while self.open_until("}") {
            let start = self.i;
            match self.class_member() {
                Ok(m) => {
                    members.push(m);
                    if let Err(d) = self.end() {
                        self.record(d, start);
                    }
                }
                Err(d) => self.record(d, start),
            }
        }
        self.expect("}")?;
        Ok(ClassDecl { name, base, maker, replaces, members })
    }

    fn class_member(&mut self) -> R<ClassMember> {
        if self.at("var") || self.at("let") {
            Ok(ClassMember::Binding(Box::new(self.binding()?)))
        } else if self.at_word("override") {
            self.take();
            self.expect("fn")?;
            let name = self.ident("Script.Want.Name")?;
            let params = self.params(false, true)?;
            let ret = if self.at("->") {
                self.take();
                Some(self.ty()?)
            } else {
                None
            };
            let body = self.block()?;
            Ok(ClassMember::Override(FnDecl { name, params, ret, body }))
        } else if self.at("fn") {
            Ok(ClassMember::Fn(self.fn_decl(true)?))
        } else if self.at_any(&["hook", "wrap", "replace"]) {
            Ok(ClassMember::Hook(self.hook()?))
        } else {
            Err(self.fail(want("Script.Want.ClassMember")))
        }
    }

    fn game_name(&mut self) -> R<GameName> {
        let t = self.peek(0).clone();
        if t.kind == Kind::Game {
            self.take();
            return Ok(GameName { at: t.at, text: t.text, quoted: true });
        }
        let first = self.ident("Script.Want.GameName")?;
        let mut text = first.name;
        while self.at("::") {
            self.take();
            text.push_str("::");
            if self.at("~") {
                self.take();
                text.push('~');
            }
            text.push_str(&self.any_word("Script.Want.GameName")?.text);
        }
        Ok(GameName { at: first.at, text, quoted: false })
    }

    fn hook(&mut self) -> R<Hook> {
        let t = self.take();
        let kind = match t.text.as_str() {
            "wrap" => HookKind::Wrap,
            "replace" => HookKind::Replace,
            _ if self.at_word("before") => {
                self.take();
                HookKind::Before
            }
            _ if self.at_word("after") => {
                self.take();
                HookKind::After
            }
            _ => return Err(self.fail(want("Script.Want.BeforeAfter"))),
        };
        let every_override = self.at_word("every");
        if every_override {
            self.take();
            self.expect_word("override")?;
            self.expect_word("of")?;
        }
        let target = self.game_name()?;
        let params = if self.at("(") { Some(self.params(false, true)?) } else { None };
        let ret = if self.at("->") {
            self.take();
            // `-> IDENT` names an after's result (G.2); a type goes on (`<`, `?`, `.`) or is a before's.
            if kind == HookKind::After && self.peek(0).kind == Kind::Id && self.peek(1).is("{") {
                Some(HookRet::Name(self.ident("Script.Want.Name")?))
            } else {
                Some(HookRet::Type(self.ty()?))
            }
        } else {
            None
        };
        let body = self.block()?;
        Ok(Hook { at: t.at, kind, every_override, target, params, ret, body })
    }

    fn on(&mut self) -> R<On> {
        let at = self.take().at;
        // A word followed by `.` starts an event's path (`on stage.entered`), whatever the word.
        let word = |p: &Self, w: &str| p.at_word(w) && !p.peek(1).is(".");
        let event = if word(self, "start") || word(self, "stop") || word(self, "frame") || word(self, "load") {
            match self.take().text.as_str() {
                "start" => OnEvent::Start,
                "stop" => OnEvent::Stop,
                "frame" => OnEvent::Frame,
                _ => OnEvent::Load,
            }
        } else if word(self, "step") {
            self.take();
            // `on step(now: time.Now)`: one parameter, named and typed.
            let now = if self.at("(") {
                self.take();
                let at = self.peek(0).at;
                let name = self.ident("Script.Want.Param")?;
                self.expect(":")?;
                let ty = self.ty()?;
                self.expect(")")?;
                Some(Param { at, var: false, is_self: false, name: Some(name), ty: Some(ty), default: None })
            } else {
                None
            };
            OnEvent::Step(now)
        } else if self.at("option") {
            self.take();
            OnEvent::Option(self.ident("Script.Want.Key")?)
        } else if word(self, "stage") {
            self.take();
            OnEvent::Stage(self.string()?)
        } else if self.at_word("draw") && self.peek(1).kind == Kind::Id {
            // `on draw <point>`; `on draw.<event>` is an event of the module draw.
            self.take();
            OnEvent::Draw(self.dotted()?)
        } else if word(self, "layers") {
            self.take();
            self.expect("->")?;
            OnEvent::Layers(self.ty()?)
        } else if self.peek(0).kind == Kind::Id {
            let path = self.dotted()?;
            // Names, the first may be typed (`on mods.stopped(e: mods.StoppedEvent)`); a type on another is refused
            // (not as the event's).
            let mut ty = None;
            let mut first = true;
            let names = if self.at("(") {
                Some(self.list("(", ")", |p| {
                    let n = p.ident("Script.Want.Name")?;
                    if p.at(":") {
                        if !first {
                            return Err(p.fail(token(",")));
                        }
                        p.take();
                        ty = Some(p.ty()?);
                    }
                    first = false;
                    Ok(n)
                })?)
            } else {
                None
            };
            OnEvent::Event { path, names, ty }
        } else {
            return Err(self.fail(want("Script.Want.OnWhat")));
        };
        let body = self.block()?;
        Ok(On { at, event, body })
    }

    fn dotted(&mut self) -> R<Vec<Ident>> {
        let mut out = vec![self.ident("Script.Want.Name")?];
        while self.at(".") {
            self.take();
            out.push(self.ident("Script.Want.Name")?);
        }
        Ok(out)
    }

    fn states(&mut self) -> R<States> {
        self.take();
        let name = self.ident("Script.Want.Name")?;
        self.expect_word("for")?;
        let class = self.ident("Script.Want.Name")?;
        self.expect("{")?;
        self.nls();
        let (mut states, mut start) = (Vec::new(), None);
        while self.open_until("}") {
            if self.at("state") {
                self.take();
                let name = self.ident("Script.Want.State")?;
                self.expect("{")?;
                self.nls();
                let mut parts = Vec::new();
                while self.open_until("}") {
                    let begin = self.i;
                    let part = if self.at_word("enter") || self.at_word("update") || self.at_word("exit") {
                        if self.peek(1).is("{") {
                            let word = self.take().text;
                            self.block().map(|b| match word.as_str() {
                                "enter" => StatePart::Enter(b),
                                "update" => StatePart::Update(b),
                                _ => StatePart::Exit(b),
                            })
                        } else {
                            self.statement().map(|s| StatePart::Stmt(Box::new(s)))
                        }
                    } else {
                        self.statement().map(|s| StatePart::Stmt(Box::new(s)))
                    };
                    match part {
                        Ok(p) => {
                            parts.push(p);
                            if let Err(d) = self.end() {
                                self.record(d, begin);
                            }
                        }
                        Err(d) => self.record(d, begin),
                    }
                }
                self.expect("}")?;
                states.push(State { name, parts });
            } else if self.at("start") {
                let at = self.take().at;
                if start.is_some() {
                    return Err(Diagnostic::new(code::ONE_START_STATE, at));
                }
                start = Some(self.ident("Script.Want.State")?);
            } else {
                return Err(self.fail(want("Script.Want.StateOrStart")));
            }
            self.end()?;
        }
        let Some(start) = start else {
            return Err(Diagnostic::new(code::ONE_START_STATE, self.peek(0).at));
        };
        self.expect("}")?;
        Ok(States { name, class, states, start })
    }

    // ------------------------------------------------------------ types (G.3)

    fn ty(&mut self) -> R<Type> {
        let at = self.peek(0).at;
        self.nested(at, Self::ty_here)
    }

    fn ty_here(&mut self) -> R<Type> {
        let at = self.peek(0).at;
        let kind = if self.at("(") {
            self.take();
            let mut items = vec![self.ty()?];
            while self.at(",") {
                self.take();
                items.push(self.ty()?);
            }
            self.expect(")")?;
            // `(T)` is T, grouped: `(fn(A) -> bool)?` is an optional function (FB, FC 2026-11-12).
            if items.len() == 1 {
                let mut inner = items.pop().expect("one type");
                while self.at("?") {
                    self.take();
                    inner.optional += 1;
                }
                inner.at = at;
                return Ok(inner);
            }
            TypeKind::Tuple(items)
        } else if self.at("fn") {
            self.take();
            let params = self.list("(", ")", |p| p.ty())?;
            let ret = if self.at("->") {
                self.take();
                Some(Box::new(self.ty()?))
            } else {
                None
            };
            TypeKind::Fn { params, ret }
        } else if self.at("ref") {
            self.take();
            TypeKind::Ref(self.type_args()?)
        } else {
            let t = self.peek(0).clone();
            let (mut parts, mut seps, quoted) = (Vec::new(), Vec::new(), t.kind == Kind::Game);
            if quoted {
                self.take();
                parts.push(Ident { name: t.text, at: t.at });
            } else {
                parts.push(self.ident("Script.Want.Type")?);
                while self.at(".") || self.at("::") {
                    seps.push(if self.take().text == "." { Sep::Dot } else { Sep::Colons });
                    parts.push(self.ident("Script.Want.Type")?);
                }
            }
            let args = if self.at("<") { self.type_args()? } else { Vec::new() };
            TypeKind::Named { parts, seps, quoted, args }
        };
        let mut optional = 0;
        while self.at("?") {
            self.take();
            optional += 1;
        }
        Ok(Type { at, kind, optional })
    }

    fn type_args(&mut self) -> R<Vec<Type>> {
        self.expect("<")?;
        let mut out = vec![self.ty()?];
        while self.at(",") {
            self.take();
            out.push(self.ty()?);
        }
        let t = self.peek(0);
        if t.kind == Kind::Op && t.text.len() > 1 && t.text.starts_with('>') {
            // `map<text, list<int>>`, `list<int>= []`: the lexer read `>>`, `>=` or `>>=`; this list takes the first
            // `>` and leaves the rest (`>`, `=`, `>=`) where it stood, to be put back if a guess is undone.
            let old = t.clone();
            let mut rest = old.clone();
            rest.text = old.text[1..].to_string();
            rest.at.col += 1;
            self.undo.push((self.i, old));
            self.t[self.i] = rest;
            return Ok(out);
        }
        self.expect(">")?;
        Ok(out)
    }

    fn string(&mut self) -> R<StrLit> {
        let t = self.peek(0).clone();
        let Some(s) = t.string.as_ref().filter(|_| t.kind == Kind::Str) else {
            return Err(self.fail(want("Script.Want.String")));
        };
        self.take();
        let mut parts = Vec::new();
        for part in &s.parts {
            match part {
                RawPart::Text(text) => parts.push(StrPart::Text(text.clone())),
                RawPart::Expr { span, at } => {
                    let toks = sub_tokens(&s.source, span.clone(), *at, self.diags);
                    let mut p = Parser::new(toks, &mut *self.diags);
                    p.depth = self.depth;
                    p.ids = self.ids;
                    let e = p.expr(false);
                    self.ids = p.ids;
                    let e = e?;
                    if !p.eof() {
                        return Err(p.fail(want("Script.Want.InterpolationEnd")));
                    }
                    parts.push(StrPart::Expr(e));
                }
            }
        }
        Ok(StrLit { at: t.at, prefix: s.prefix, parts })
    }

    // ------------------------------------------------------------ statements (G.5)

    fn block(&mut self) -> R<Block> {
        let at = self.peek(0).at;
        self.nested(at, Self::block_here)
    }

    fn block_here(&mut self) -> R<Block> {
        let at = self.expect("{")?.at;
        self.nls();
        let mut stmts = Vec::new();
        while self.open_until("}") {
            self.statement_into(&mut stmts);
        }
        let end = self.expect("}")?.at;
        self.after_block = self.i;
        Ok(Block { at, stmts, end })
    }

    fn statement(&mut self) -> R<Stmt> {
        let t = self.peek(0).clone();
        let at = t.at;
        let kind = if self.at("let") || self.at("var") {
            StmtKind::Binding(Box::new(self.binding()?))
        } else if self.at("return") {
            self.take();
            StmtKind::Return(if self.ends() { None } else { Some(self.expr(false)?) })
        } else if self.at("break") {
            self.take();
            StmtKind::Break
        } else if self.at("continue") {
            self.take();
            StmtKind::Continue
        } else if self.at("stop") {
            self.take();
            StmtKind::Stop
        } else if self.at("skip") {
            self.take();
            StmtKind::Skip(if self.ends() { None } else { Some(self.expr(false)?) })
        } else if self.at("go") {
            self.take();
            StmtKind::Go(self.dotted()?)
        } else if self.at("defer") {
            self.take();
            StmtKind::Defer(Box::new(self.nested(at, Self::statement)?))
        } else if self.at("log") {
            self.take();
            // `log fact <name> <value>`: `fact` is the word when a value starts right after it; `log fact + 1` and
            // `log fact.n` log a variable named fact.
            let next = self.peek(1);
            let value_next = matches!(next.kind, Kind::Str | Kind::Num | Kind::Id | Kind::Color | Kind::Game)
                || ["true", "false", "none", "self", "result"].iter().any(|w| next.is(w));
            let fact = if self.at_word("fact") && value_next {
                self.take();
                Some(self.expr(false)?)
            } else {
                None
            };
            StmtKind::Log { fact, value: self.expr(false)? }
        } else if self.at_word("fail") && self.peek(1).kind == Kind::Str {
            // `fail "<text>"`: a word here only, before a text.
            self.take();
            StmtKind::Fail(self.expr(false)?)
        } else if self.at("expect") {
            self.take();
            StmtKind::Expect(self.expr(false)?)
        } else if self.at("guard") {
            self.take();
            let conds = self.conds()?;
            self.expect("else")?;
            StmtKind::Guard { conds, otherwise: self.block()? }
        } else if self.at("while") {
            self.take();
            let cond = self.expr(true)?;
            StmtKind::While { cond, body: self.block()? }
        } else if self.at("for") {
            self.take();
            let pattern = self.pattern(false)?;
            self.expect("in")?;
            let iter = self.expr(true)?;
            StmtKind::For { pattern, iter, body: self.block()? }
        } else if self.at("fn") {
            StmtKind::Fn(self.fn_decl(false)?)
        } else if t.kind == Kind::Kw && BLOCK_ONLY_AT_TOP.contains(&t.text.as_str()) {
            return Err(Diagnostic::new(code::ITEM_IN_BLOCK, at).with("found", token(&t.text)));
        } else {
            let e = self.expr(false)?;
            let op = ASSIGN.iter().find(|(text, _)| self.peek(0).kind == Kind::Op && self.peek(0).text == *text).map(|(_, op)| *op);
            match op {
                Some(op) => {
                    self.take();
                    StmtKind::Assign { target: e, op, value: self.expr(false)? }
                }
                None => StmtKind::Expr(e),
            }
        };
        Ok(Stmt { at, kind })
    }

    fn ends(&self) -> bool {
        matches!(self.peek(0).kind, Kind::Nl | Kind::Eof) || self.at(";") || self.at("}")
    }

    fn conds(&mut self) -> R<Vec<Cond>> {
        let mut out = Vec::new();
        loop {
            let pattern = if self.at("let") {
                self.take();
                let p = self.pattern(false)?;
                self.expect("=")?;
                Some(p)
            } else {
                None
            };
            out.push(Cond { pattern, value: self.expr(true)? });
            if !self.at(",") {
                return Ok(out);
            }
            self.take();
        }
    }

    /// `arm`: a match arm's pattern (or an element of its tuple), where a name must be an enum case: a bare
    /// name there would match every value. Inside a case's fields a bare name binds the field.
    fn pattern(&mut self, arm: bool) -> R<Pattern> {
        let at = self.peek(0).at;
        self.nested(at, |p| p.pattern_here(arm))
    }

    fn pattern_here(&mut self, arm: bool) -> R<Pattern> {
        let t = self.peek(0).clone();
        let at = t.at;
        if self.at("(") {
            self.take();
            let mut items = vec![self.pattern(arm)?];
            while self.at(",") {
                self.take();
                items.push(self.pattern(arm)?);
            }
            self.expect(")")?;
            return Ok(Pattern { at, kind: PatternKind::Tuple(items) });
        }
        if matches!(t.kind, Kind::Num | Kind::Str | Kind::Color) || self.at("true") || self.at("false") || self.at("none") {
            let e = self.primary(false)?;
            return Ok(Pattern { at, kind: PatternKind::Literal(Box::new(e)) });
        }
        if self.at("-") && self.peek(1).kind == Kind::Num {
            self.take();
            let n = self.take();
            let e = Expr { id: self.new_id(), at, kind: ExprKind::Neg(Box::new(Expr { id: self.new_id(), at: n.at, kind: ExprKind::Number(n.text) })) };
            return Ok(Pattern { at, kind: PatternKind::Literal(Box::new(e)) });
        }
        if t.kind == Kind::Id {
            if arm && t.text == "_" {
                return Err(Diagnostic::new(code::UNDERSCORE_PATTERN, at));
            }
            let path = self.dotted()?;
            if path.len() == 1 && arm {
                return Err(Diagnostic::new(code::BARE_ARM_NAME, at));
            }
            let (mut args, mut fields) = (None, None);
            if self.at("(") {
                args = Some(self.list("(", ")", |p| p.pattern(false))?);
            } else if self.at("{") && path.last().is_some_and(|c| upper(&c.name)) {
                // A case's fields by name (`Shape.Box { w, h }`); the case is capitalized, as a struct literal's is.
                fields = Some(self.list("{", "}", |p| p.ident("Script.Want.Field"))?);
            }
            return Ok(Pattern { at, kind: PatternKind::Path { path, args, fields } });
        }
        Err(self.fail(want("Script.Want.Pattern")))
    }

    // ------------------------------------------------------------ expressions (G.4)

    /// `nostruct`: in a condition, where a `{` after a capitalized name opens the block.
    pub fn expr(&mut self, nostruct: bool) -> R<Expr> {
        let at = self.peek(0).at;
        self.nested(at, |p| p.binary(0, nostruct))
    }

    /// The binary operator at the token and its level (G.4), if one is there.
    fn binary_op(&self) -> Option<(usize, &'static str, BinOp)> {
        let t = self.peek(0);
        if !matches!(t.kind, Kind::Op | Kind::Kw) {
            return None;
        }
        LEVELS.iter().enumerate().find_map(|(level, ops)| {
            ops.and_then(|ops| ops.iter().find(|(text, _)| t.text == *text)).map(|&(text, op)| (level, text, op))
        })
    }

    /// The operators of level `min` and tighter, by precedence climbing: one call per operand rather than one per
    /// level, so a bracket costs the stack a few calls.
    fn binary(&mut self, min: usize, nostruct: bool) -> R<Expr> {
        self.keeping_depth(|p| p.binary_here(min, nostruct))
    }

    fn binary_here(&mut self, min: usize, nostruct: bool) -> R<Expr> {
        // `not` is a level of its own, looser than the comparisons: `not a == b` is `not (a == b)`.
        let mut left = if min <= NOT_LEVEL && self.at("not") {
            let at = self.take().at;
            let e = self.nested(at, |p| p.binary(NOT_LEVEL, nostruct))?;
            Expr { id: self.new_id(), at, kind: ExprKind::Not(Box::new(e)) }
        } else {
            self.unary(nostruct)?
        };
        while let Some((level, text, op)) = self.binary_op().filter(|&(level, ..)| level >= min) {
            let first = self.take().at;
            if (text == "&" || text == "|") && self.at(text) {
                return Err(Diagnostic::new(code::DOUBLED_OPERATOR, first)
                    .with("found", token(&format!("{text}{text}")))
                    .with("expected", token(if text == "&" { "and" } else { "or" })));
            }
            // A line end after an operator is no end; the lexer leaves the one after `>` and `>>`, which may
            // end a type instead.
            while self.peek(0).kind == Kind::Nl {
                self.take();
            }
            // `??` groups to the right (`a ?? b ?? c` is `a ?? (b ?? c)`), the others to the left.
            let right = if op == BinOp::Coalesce {
                self.nested(first, |p| p.binary(level, nostruct))?
            } else {
                self.binary(level + 1, nostruct)?
            };
            self.wrap_deeper(first)?;
            let at = left.at;
            left = Expr { id: self.new_id(), at, kind: ExprKind::Binary { op, left: Box::new(left), right: Box::new(right) } };
            if level == COMPARE_LEVEL && self.binary_op().is_some_and(|(next, ..)| next == COMPARE_LEVEL) {
                return Err(Diagnostic::new(code::CHAINED_COMPARISON, self.peek(0).at));
            }
        }
        Ok(left)
    }

    fn unary(&mut self, nostruct: bool) -> R<Expr> {
        self.keeping_depth(|p| p.unary_here(nostruct))
    }

    fn unary_here(&mut self, nostruct: bool) -> R<Expr> {
        let at = self.peek(0).at;
        if self.at("!") {
            return Err(Diagnostic::new(code::PREFIX_BANG, at));
        }
        if self.at("-") {
            self.take();
            let e = self.nested(at, |p| p.unary(nostruct))?;
            return Ok(Expr { id: self.new_id(), at, kind: ExprKind::Neg(Box::new(e)) });
        }
        if self.at("wait") {
            self.take();
            if self.at_word("until") {
                self.take();
                let e = self.expr(nostruct)?;
                return Ok(Expr { id: self.new_id(), at, kind: ExprKind::WaitUntil(Box::new(e)) });
            }
            let amount = self.nested(at, |p| p.unary(nostruct))?;
            let unit = if self.at_word("frames") {
                self.take();
                Some(WaitUnit::Frames)
            } else if self.at_word("seconds") {
                self.take();
                Some(WaitUnit::Seconds)
            } else {
                None
            };
            return Ok(Expr { id: self.new_id(), at, kind: ExprKind::Wait { amount: Box::new(amount), unit } });
        }
        if self.at("start") {
            self.take();
            let e = self.postfix(nostruct)?;
            return Ok(Expr { id: self.new_id(), at, kind: ExprKind::Start(Box::new(e)) });
        }
        let mut e = self.postfix(nostruct)?;
        while self.at("as") {
            let word = self.take().at;
            // `as?`: the `?` right after `as` makes it the narrowing of a game object.
            let narrow = if self.at("?") {
                self.take();
                true
            } else {
                false
            };
            self.wrap_deeper(word)?;
            let ty = self.ty()?;
            e = Expr { id: self.new_id(), at, kind: ExprKind::As { value: Box::new(e), ty: Box::new(ty), narrow } };
        }
        Ok(e)
    }

    /// `arg = ["var"] expr | IDENT ":" ["var"] expr`, the positional ones first (a `var` argument by name).
    fn args(&mut self) -> R<Vec<Arg>> {
        let mut named = false;
        self.list("(", ")", |p| {
            let at = p.peek(0).at;
            let (mut name, mut var) = (None, false);
            if p.peek(0).kind == Kind::Id && p.peek(1).kind == Kind::Op && p.peek(1).text == ":" {
                name = Some(p.ident("Script.Want.Name")?);
                p.take();
                named = true;
                if p.at("var") {
                    p.take();
                    var = true;
                }
            } else if named {
                return Err(Diagnostic::new(code::POSITIONAL_AFTER_NAMED, at));
            } else if p.at("var") {
                p.take();
                var = true;
            }
            let value = p.expr(false)?;
            Ok(Arg { at, name, var, value })
        })
    }

    fn postfix(&mut self, nostruct: bool) -> R<Expr> {
        self.keeping_depth(|p| p.postfix_here(nostruct))
    }

    fn postfix_here(&mut self, nostruct: bool) -> R<Expr> {
        let mut e = self.primary(nostruct)?;
        loop {
            let at = e.at;
            if self.at_any(&["(", "[", ".", "?.", "!"]) {
                let here = self.peek(0).at;
                self.wrap_deeper(here)?;
            }
            if self.at("(") {
                let args = self.args()?;
                e = Expr { id: self.new_id(), at, kind: ExprKind::Call { callee: Box::new(e), args } };
            } else if self.at("[") {
                self.take();
                let index = self.expr(false)?;
                self.expect("]")?;
                e = Expr { id: self.new_id(), at, kind: ExprKind::Index { base: Box::new(e), index: Box::new(index) } };
            } else if self.at(".") || self.at("?.") {
                let optional = self.take().text == "?.";
                let t = self.peek(0);
                if !matches!(t.kind, Kind::Id | Kind::Num | Kind::Kw) {
                    return Err(self.fail(want("Script.Want.Member")));
                }
                let t = self.take();
                let name = Ident { name: t.text, at: t.at };
                let targs = self.maybe_type_args()?;
                e = Expr { id: self.new_id(), at, kind: ExprKind::Member { base: Box::new(e), name, optional, targs } };
            } else if self.at("!") {
                self.take();
                e = Expr { id: self.new_id(), at, kind: ExprKind::Unwrap(Box::new(e)) };
            } else {
                return Ok(e);
            }
        }
    }

    /// `json.decode<map<text, Best>>(data)`: type arguments right before a call's `(` (G.4: only then); anything else
    /// is a comparison, and the guess is undone. Types nested past the limit are said as such, not guessed again.
    fn maybe_type_args(&mut self) -> R<Vec<Type>> {
        if !self.at("<") {
            return Ok(Vec::new());
        }
        let (i, undo) = (self.i, self.undo.len());
        match self.type_args() {
            Ok(args) if self.at("(") => return Ok(args),
            // `json.decode<list<int>>` / `(data)`: the line ends the statement, so as written this is no call
            // and its comparison reading is a chained one. Said as what it is, and read on as the call it means.
            Ok(args) if self.peek(0).kind == Kind::Nl && self.peek(1).is("(") => {
                self.diags.push(Diagnostic::new(code::TYPE_ARGS_LINE_END, self.peek(1).at));
                self.take();
                return Ok(args);
            }
            Err(d) if d.code == code::TOO_DEEP => return Err(d),
            _ => {}
        }
        while self.undo.len() > undo {
            if let Some((k, tok)) = self.undo.pop() {
                self.t[k] = tok;
            }
        }
        self.i = i;
        Ok(Vec::new())
    }

    fn primary(&mut self, nostruct: bool) -> R<Expr> {
        let t = self.peek(0).clone();
        let at = t.at;
        let id = self.new_id();
        let simple = |kind| Ok(Expr { id, at, kind });
        match t.kind {
            Kind::Num => {
                self.take();
                return simple(ExprKind::Number(t.text));
            }
            Kind::Color => {
                self.take();
                return simple(ExprKind::Color(t.text));
            }
            Kind::Str => return simple(ExprKind::Str(self.string()?)),
            Kind::Game => {
                self.take();
                return simple(ExprKind::Game(GameName { at, text: t.text, quoted: true }));
            }
            _ => {}
        }
        for (word, kind) in [("true", ExprKind::True), ("false", ExprKind::False), ("none", ExprKind::None), ("self", ExprKind::SelfValue), ("result", ExprKind::Result)] {
            if self.at(word) {
                self.take();
                return simple(kind);
            }
        }
        // A reserved word right before `.` is a module's name (`state.enter(...)` after `use kit.player.state`).
        if t.kind == Kind::Kw && self.peek(1).is(".") {
            self.take();
            return simple(ExprKind::Name { name: Ident { name: t.text, at }, targs: Vec::new() });
        }
        if self.at("proceed") {
            self.take();
            self.expect("(")?;
            self.expect(")")?;
            return simple(ExprKind::Proceed);
        }
        if self.at("ref") {
            self.take();
            return simple(ExprKind::Ref(self.args()?));
        }
        if self.at("if") {
            self.take();
            let mut arms = vec![(self.conds()?, self.block()?)];
            let mut otherwise = None;
            while self.at("else") {
                self.take();
                if self.at("if") {
                    self.take();
                    arms.push((self.conds()?, self.block()?));
                } else {
                    otherwise = Some(self.block()?);
                    break;
                }
            }
            return simple(ExprKind::If { arms, otherwise });
        }
        if self.at("match") {
            return self.match_();
        }
        if self.at("fn") {
            self.take();
            let params = self.params(false, false)?;
            let ret = if self.at("->") {
                self.take();
                Some(self.ty()?)
            } else {
                None
            };
            return simple(ExprKind::Closure { params, ret: ret.map(Box::new), body: self.block()? });
        }
        if self.at("(") {
            self.take();
            let first = self.expr(false)?;
            if !self.at(",") {
                self.expect(")")?;
                return simple(ExprKind::Paren(Box::new(first)));
            }
            let mut items = vec![first];
            while self.at(",") {
                self.take();
                items.push(self.expr(false)?);
            }
            self.expect(")")?;
            return simple(ExprKind::Tuple(items));
        }
        if self.at("[") {
            return simple(ExprKind::List(self.list("[", "]", |p| p.expr(false))?));
        }
        if self.at("{") {
            return self.brace_literal();
        }
        if t.kind == Kind::Id && CONTENT.contains(&t.text.as_str()) && self.peek(1).kind == Kind::Str {
            self.take();
            let path = self.string()?;
            if path.interpolates() {
                return Err(Diagnostic::new(code::CONTENT_INTERPOLATES, path.at).with("found", token(&t.text)));
            }
            let args = if t.text == "text" && self.at("(") { Some(self.args()?) } else { None };
            return simple(ExprKind::Content { word: t.text, path, args });
        }
        if t.kind == Kind::Id {
            let name = self.ident("Script.Want.Name")?;
            if self.at("::") {
                let mut text = name.name;
                while self.at("::") {
                    self.take();
                    text.push_str("::");
                    if self.at("~") {
                        self.take();
                        text.push('~');
                    }
                    text.push_str(&self.any_word("Script.Want.GameName")?.text);
                }
                return simple(ExprKind::Game(GameName { at, text, quoted: false }));
            }
            let targs = self.maybe_type_args()?;
            let capital = upper(&name.name);
            if nostruct && capital {
                self.no_struct_here(self.i)?;
            }
            if self.at("{") && !nostruct && capital {
                return self.struct_literal(vec![name]);
            }
            if self.at(".") && self.peek(1).kind == Kind::Id && upper(&self.peek(1).text) {
                // module.Struct { ... }: a literal when a brace follows the last, capitalized name.
                let mut j = self.i;
                while self.t[j].is(".") && self.t.get(j + 1).is_some_and(|n| n.kind == Kind::Id) {
                    j += 2;
                }
                let last_upper = upper(&self.t[j - 1].text);
                if nostruct {
                    if last_upper {
                        self.no_struct_here(j)?;
                    }
                } else if self.t[j].is("{") && last_upper {
                    let mut path = vec![name];
                    while self.i < j {
                        self.take();
                        let part = self.take();
                        path.push(Ident { name: part.text, at: part.at });
                    }
                    return self.struct_literal(path);
                }
            }
            return simple(ExprKind::Name { name, targs });
        }
        Err(self.fail(want("Script.Want.Expression")))
    }

    /// In a condition a `{` after a capitalized name opens the block; when `name:` follows it, it was meant as a
    /// struct literal (no statement starts `name:`): say the rule.
    fn no_struct_here(&self, j: usize) -> R<()> {
        if !self.t[j].is("{") {
            return Ok(());
        }
        let mut k = j + 1;
        while self.t.get(k).is_some_and(|t| t.kind == Kind::Nl) {
            k += 1;
        }
        if self.t.get(k).is_some_and(|t| t.kind == Kind::Id) && self.t.get(k + 1).is_some_and(|t| t.kind == Kind::Op && t.text == ":") {
            return Err(Diagnostic::new(code::STRUCT_IN_CONDITION, self.t[j].at));
        }
        Ok(())
    }

    fn struct_literal(&mut self, path: Vec<Ident>) -> R<Expr> {
        let at = path[0].at;
        self.expect("{")?;
        self.sep();
        let mut fields = Vec::new();
        while self.open_until("}") {
            // A field a reserved word names (a service struct's `start`, read as `how.start`) before its `:`.
            let name = if self.peek(0).kind == Kind::Kw && self.peek(1).is(":") {
                self.path_word("Script.Want.Field")?
            } else {
                self.ident("Script.Want.Field")?
            };
            let value = if self.at(":") {
                self.take();
                Some(self.expr(false)?)
            } else {
                None
            };
            fields.push((name, value));
            if !self.sep() && !self.at("}") {
                return Err(self.fail(want("Script.Want.FieldSep")));
            }
        }
        self.expect("}")?;
        Ok(Expr { id: self.new_id(), at, kind: ExprKind::StructLit { path, fields } })
    }

    /// `{}` (an empty map or set, by its place), `{k: v, ...}` a map, `{a, b}` a set.
    fn brace_literal(&mut self) -> R<Expr> {
        let at = self.expect("{")?.at;
        self.sep();
        let mut entries = Vec::new();
        while self.open_until("}") {
            let key = self.expr(false)?;
            let value = if self.at(":") {
                self.take();
                Some(self.expr(false)?)
            } else {
                None
            };
            entries.push((key, value));
            if !self.sep() && !self.at("}") {
                return Err(self.fail(want("Script.Want.EntrySep")));
            }
        }
        self.expect("}")?;
        Ok(Expr { id: self.new_id(), at, kind: ExprKind::Braces(entries) })
    }

    fn match_(&mut self) -> R<Expr> {
        let at = self.take().at;
        let subject = self.expr(true)?;
        self.expect("{")?;
        self.sep();
        let mut arms = Vec::new();
        while self.open_until("}") {
            let pattern = if self.at("else") {
                self.take();
                None
            } else {
                Some(self.pattern(true)?)
            };
            self.expect("=>")?;
            let body = if self.at("{") { ArmBody::Block(self.block()?) } else { ArmBody::Stmt(Box::new(self.statement()?)) };
            arms.push(Arm { pattern, body });
            if !self.sep() && !self.at("}") {
                return Err(self.fail(want("Script.Want.ArmSep")));
            }
        }
        self.expect("}")?;
        Ok(Expr { id: self.new_id(), at, kind: ExprKind::Match { subject: Box::new(subject), arms } })
    }
}
