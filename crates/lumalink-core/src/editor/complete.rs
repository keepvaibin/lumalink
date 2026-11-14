//! What may be written at a place (completion) and the call being written (signature help). The place's kind comes
//! from the compiler's own tokens (nothing is offered in a comment or a string); what is offered comes from the scopes
//! the checker made, the types it gave, the needed mods' interfaces, the services' faces and the game's
//! self-description. After `value.` the members are those of the type the checker gives `value`, read with the
//! `.member` being written taken out, so a half-written line still has its type.

use super::query::{binding_name, params_text, ret_text};
use super::*;
use lumascript::check::{ModRef, ModTarget, Name};
use lumascript::lex::{self, Kind as TokKind};

/// The most game names one answer gives inside backquotes; past it the answer says it is incomplete, and the editor
/// asks again as more is typed.
pub const MOST_GAME_NAMES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: Kind,
    /// Code: a signature, a type, a module's path.
    pub detail: String,
    /// The documentation written in the source above the name.
    pub doc: String,
    /// The text it replaces (start, end), when that is more than the word before the place.
    pub replace: Option<(Pos, Pos)>,
    /// Its group, first shown first: locals, the module's names, standard names, words of the language, the game's.
    pub group: u8,
}

/// The answer: the items, and whether more would match (the editor asks again as more is typed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completions {
    pub items: Vec<Completion>,
    pub incomplete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    /// Each parameter's text, as it is in `label`.
    pub params: Vec<String>,
    pub doc: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signatures {
    pub list: Vec<Signature>,
    pub active: usize,
    pub param: u32,
}

/// What kind of text a place is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spot {
    Code,
    Comment,
    Str,
    /// Inside backquotes; the game name starts at `from`.
    Game { from: Pos },
}

/// A member name no value has, written after a value's dot so the value reads in its place while its member is being
/// written.
const NO_MEMBER: &str = "lumalink_no_member";

/// A text's characters and the character offset each line starts at.
struct Chars {
    chars: Vec<char>,
    starts: Vec<usize>,
}

impl Chars {
    fn new(text: &str) -> Chars {
        let chars: Vec<char> = text.chars().collect();
        let mut starts = vec![0];
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                starts.push(i + 1);
            }
        }
        Chars { chars, starts }
    }

    fn offset(&self, at: Pos) -> usize {
        self.starts.get(at.line.saturating_sub(1) as usize).map_or(self.chars.len(), |s| (s + at.col.saturating_sub(1) as usize).min(self.chars.len()))
    }

    fn pos(&self, offset: usize) -> Pos {
        let line = self.starts.partition_point(|s| *s <= offset).max(1);
        Pos { line: line as u32, col: (offset - self.starts[line - 1]) as u32 + 1 }
    }

    fn text(&self, from: usize, to: usize) -> String {
        self.chars[from.min(to)..to.min(self.chars.len())].iter().collect()
    }
}

/// The tokens with their start and end offsets.
fn spans(c: &Chars, text: &str) -> Vec<(lex::Tok, usize, usize)> {
    let mut ignored = Vec::new();
    lex::lex(text, &mut ignored)
        .into_iter()
        .filter(|t| !matches!(t.kind, TokKind::Eof))
        .map(|t| {
            let (start, end) = match (&t.kind, &t.string) {
                (TokKind::Str, Some(s)) => (s.whole.start, s.whole.end),
                (TokKind::Game, _) => {
                    let start = c.offset(t.at);
                    // To the closing backquote, or to the line's end when it has none yet.
                    let line_end = c.chars[start..].iter().position(|x| *x == '\n').map_or(c.chars.len(), |p| start + p);
                    let close = c.chars[start + 1..line_end].iter().position(|x| *x == '`').map(|p| start + 1 + p + 1);
                    (start, close.unwrap_or(line_end))
                }
                (TokKind::Nl, _) => {
                    let start = c.offset(t.at);
                    (start, start + 1)
                }
                _ => {
                    let start = c.offset(t.at);
                    (start, start + t.text.chars().count())
                }
            };
            (t, start, end)
        })
        .collect()
}

/// The kind of text at offset `p`.
fn spot(c: &Chars, toks: &[(lex::Tok, usize, usize)], p: usize) -> Spot {
    let mut prev_end = 0;
    for (t, start, end) in toks {
        if *start >= p {
            break;
        }
        match &t.kind {
            TokKind::Str if p < *end || (p == *end && c.chars.get(end - 1) != Some(&'"')) => {
                let inside = t.string.as_ref().is_some_and(|s| s.parts.iter().any(|part| matches!(part, lex::RawPart::Expr { span, .. } if span.start <= p && p <= span.end)));
                return if inside { Spot::Code } else { Spot::Str };
            }
            TokKind::Game if p < *end || (p == *end && c.chars.get(end - 1) != Some(&'`')) => return Spot::Game { from: c.pos(start + 1) },
            _ => {}
        }
        prev_end = prev_end.max(*end);
    }
    // Between tokens there are only spaces and comments.
    let (mut i, mut depth) = (prev_end, 0u32);
    while i < p {
        let two = (c.chars.get(i).copied(), c.chars.get(i + 1).copied());
        match two {
            (Some('/'), Some('*')) => {
                depth += 1;
                i += 2;
            }
            (Some('*'), Some('/')) if depth > 0 => {
                depth -= 1;
                i += 2;
            }
            (Some('/'), Some('/')) if depth == 0 => {
                let nl = c.chars[i..].iter().position(|x| *x == '\n').map_or(c.chars.len(), |n| i + n);
                if nl >= p {
                    return Spot::Comment;
                }
                i = nl + 1;
            }
            _ => i += 1,
        }
    }
    if depth > 0 {
        Spot::Comment
    } else {
        Spot::Code
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Analysis {
    /// What may be written at `at` in module `m`.
    pub fn complete(&self, m: usize, at: Pos, game: &GameView) -> Completions {
        let Some(module) = self.modules.get(m) else { return Completions::default() };
        let c = Chars::new(&module.text);
        let toks = spans(&c, &module.text);
        let p = c.offset(at);
        let stem_start = (0..p).rev().take_while(|i| is_word(c.chars[*i])).last().unwrap_or(p);
        match spot(&c, &toks, p) {
            Spot::Comment | Spot::Str => Completions::default(),
            Spot::Game { from } => self.game_names(&c.text(c.offset(from), p), from, at, game),
            Spot::Code => {
                let line_start = c.starts[at.line as usize - 1];
                let head = c.text(line_start, stem_start);
                if let Some(items) = self.complete_use(m, &head, &c, stem_start, at, game) {
                    return Completions { items, incomplete: false };
                }
                if head.ends_with("::") {
                    let path_end = stem_start - 2;
                    let path_start = (0..path_end).rev().take_while(|i| is_word(c.chars[*i]) || c.chars[*i] == ':' || c.chars[*i] == '~').last().unwrap_or(path_end);
                    let path = c.text(path_start, path_end);
                    return Completions { items: game_children(game, &path), incomplete: false };
                }
                if head.ends_with('.') && !head.ends_with("..") {
                    let dot = if head.ends_with("?.") { stem_start - 2 } else { stem_start - 1 };
                    return Completions { items: self.members_at(m, &c, dot, p, game), incomplete: false };
                }
                Completions { items: self.names_at(m, at, game), incomplete: false }
            }
        }
    }

    /// Inside backquotes: the game's names holding what is written (any case), at most [`MOST_GAME_NAMES`].
    fn game_names(&self, written: &str, from: Pos, at: Pos, game: &GameView) -> Completions {
        let w = written.to_lowercase();
        let mut all: Vec<(&String, Kind, String)> = game.game.functions.iter().map(|(n, s)| (n, Kind::Function, s.first().map(|x| x.0.clone()).unwrap_or_default())).collect();
        if let Some(types) = &game.game.types {
            all.extend(types.records.keys().map(|n| (n, Kind::Class, String::new())));
            all.extend(types.enums.keys().map(|n| (n, Kind::Enum, String::new())));
            all.extend(types.globals.iter().map(|(n, t)| (n, Kind::Variable, t.clone())));
        }
        all.retain(|(n, _, _)| n.to_lowercase().contains(&w));
        all.sort_by(|a, b| a.0.cmp(b.0));
        let incomplete = all.len() > MOST_GAME_NAMES;
        let items = all
            .into_iter()
            .take(MOST_GAME_NAMES)
            .map(|(n, kind, detail)| Completion { label: n.clone(), kind, detail, doc: String::new(), replace: Some((from, at)), group: 4 })
            .collect();
        Completions { items, incomplete }
    }

    /// In a `use` line: the services, the mod's modules, the needed mods and their modules.
    fn complete_use(&self, m: usize, head: &str, c: &Chars, stem_start: usize, at: Pos, game: &GameView) -> Option<Vec<Completion>> {
        let rest = head.trim_start().strip_prefix("use ")?;
        let rest = rest.trim_start();
        let rest = rest.strip_prefix("optional ").map_or(rest, str::trim_start);
        let item = |label: String, kind: Kind, detail: String, replace: Option<(Pos, Pos)>| Completion { label, kind, detail, doc: String::new(), replace, group: 1 };
        // The path written so far, from its first character to the place.
        let from = |prefix: &str| c.pos(stem_start - (rest.chars().count() - prefix.chars().count()));
        let mut out = Vec::new();
        if let Some(after) = rest.strip_prefix("svc.") {
            let start = from(after);
            let mut names: Vec<&String> = game.game.services.faces.keys().collect();
            names.sort();
            for s in names {
                out.push(item(s.clone(), Kind::Module, game.game.services.faces[s].header.clone(), Some((start, at))));
            }
        } else if let Some(after) = rest.strip_prefix("./") {
            let start = from(after);
            for (i, module) in self.modules.iter().enumerate() {
                if i != m {
                    out.push(item(module.path.clone(), Kind::Module, String::new(), Some((start, at))));
                }
            }
        } else if let Some((lib, after)) = rest.split_once('.') {
            let id = if lib == "kit" { check::LUMAKIT } else { lib };
            let library = self.libraries.iter().find(|l| l.id == id)?;
            let start = from(after);
            for part in &library.parts {
                out.push(item(part.path.clone(), Kind::Module, library.id.clone(), Some((start, at))));
            }
        } else if !rest.contains(' ') {
            out.push(item("svc".into(), Kind::Module, String::new(), None));
            out.push(item("c".into(), Kind::Keyword, String::new(), None));
            out.push(item("optional".into(), Kind::Keyword, String::new(), None));
            for l in &self.libraries {
                out.push(item(l.id.clone(), Kind::Module, String::new(), None));
                if l.id == check::LUMAKIT {
                    out.push(item("kit".into(), Kind::Module, l.id.clone(), None));
                }
            }
        } else {
            return Some(Vec::new());
        }
        Some(out)
    }

    /// The names an expression or a statement may start with at `at`.
    fn names_at(&self, m: usize, at: Pos, game: &GameView) -> Vec<Completion> {
        let mut out = Vec::new();
        let item = |label: &str, kind: Kind, detail: String, doc: String, group: u8| Completion { label: label.to_string(), kind, detail, doc, replace: None, group };
        // The locals in reach: declared before the place, in a scope that holds it.
        let scopes = self.parsed.get(m).map(scope_ranges).unwrap_or_default();
        let innermost = |p: Pos| scopes.iter().filter(|(s, e)| *s <= p && p <= *e).min_by_key(|(s, e)| (e.line - s.line, e.col.wrapping_sub(s.col)));
        let mut seen = std::collections::BTreeSet::new();
        let mut locals: Vec<&check::Local> = self.checked.locals.iter().filter(|l| l.module == m && l.at < at).collect();
        locals.sort_by_key(|l| std::cmp::Reverse(l.at));
        for l in locals {
            let visible = match innermost(l.at) {
                Some((s, e)) => *s <= at && at <= *e,
                None => true,
            };
            if visible && seen.insert(l.name.clone()) {
                out.push(item(&l.name, Kind::Variable, self.ty_text(&l.ty, game), String::new(), 0));
            }
        }
        if let Some(scope) = self.checked.scopes.get(m) {
            for (name, n) in &scope.names {
                let (kind, detail, doc) = match n {
                    Name::Item { item: i, sort, .. } => {
                        let r = ItemRef { owner: Owner::Mod, module: m, item: *i };
                        (sort_kind(*sort), self.short_text(r), self.doc_of_item(r))
                    }
                    Name::Module { target: ModTarget::Module { owner, module }, .. } => (Kind::Module, self.module_label(ModRef { owner: *owner, module: *module }), String::new()),
                    Name::Module { target: ModTarget::Service(s), .. } => (Kind::Module, format!("svc.{s}"), String::new()),
                    Name::Taken { item: r, .. } => (Kind::Function, self.short_text(*r), self.doc_of_item(*r)),
                };
                out.push(item(name, kind, detail, doc, 1));
            }
        }
        for s in check::std_names() {
            out.push(item(s, Kind::Function, String::new(), String::new(), 2));
        }
        for k in lumascript::lex::KEYWORDS {
            out.push(item(k, Kind::Keyword, String::new(), String::new(), 3));
        }
        for k in lumascript::lex::CONTENT {
            out.push(item(k, Kind::Keyword, String::new(), String::new(), 3));
        }
        // The game's names by their first part: a record, an enum, a namespace (`MR`), a global.
        let mut heads: BTreeMap<&str, Kind> = BTreeMap::new();
        if let Some(types) = &game.game.types {
            for r in types.records.keys().filter(|r| !r.contains('<')) {
                heads.insert(r.split("::").next().unwrap_or(r), Kind::Class);
            }
            for e in types.enums.keys() {
                heads.entry(e.split("::").next().unwrap_or(e)).or_insert(Kind::Enum);
            }
            for g in types.globals.keys() {
                heads.entry(g.split("::").next().unwrap_or(g)).or_insert(if g.contains("::") { Kind::Module } else { Kind::Variable });
            }
        }
        for f in game.game.functions.keys() {
            if let Some((head, _)) = f.split_once("::") {
                heads.entry(head).or_insert(Kind::Module);
            }
        }
        for (h, kind) in heads {
            if !h.is_empty() && h.chars().all(is_word) {
                out.push(item(h, kind, String::new(), String::new(), 4));
            }
        }
        out
    }

    /// The members of the value written before the `.` at offset `dot` (the place is `p`).
    fn members_at(&self, m: usize, c: &Chars, dot: usize, p: usize, game: &GameView) -> Vec<Completion> {
        // A chain of names: a module's items (`kit.actor.`), an enum's cases (`actor.Attack.`), a service's parts.
        let chain_start = (0..dot).rev().take_while(|i| is_word(c.chars[*i]) || c.chars[*i] == '.').last().unwrap_or(dot);
        let chain = c.text(chain_start, dot);
        let first_char_before = chain_start.checked_sub(1).map(|i| c.chars[i]);
        if !chain.is_empty() && !matches!(first_char_before, Some(')' | ']' | '"' | '`' | '?')) {
            if let Some(items) = self.chain_members(m, &chain, game) {
                return items;
            }
        }
        // A value: the type the checker gave the name (or `self`) that ends at the dot. The reading as it is has it when
        // a member is written there already; else the module is read again with a member name no value has in place
        // of the word being written, so the value keeps its place (an argument's type is checked against the
        // parameter's only when the argument reads).
        if let Some(t) = self.type_ending_at(m, &self.modules[m].text, c.pos(dot)) {
            return self.members_of(&t, game);
        }
        let word_end = p + c.chars[p..].iter().take_while(|x| is_word(**x)).count();
        let mut module_texts: Vec<Module> = self.modules.clone();
        let text = format!("{}.{NO_MEMBER}{}", c.text(0, dot), c.text(word_end, c.chars.len()));
        module_texts[m].text = text.clone();
        let reread = Analysis::new(&self.root, module_texts, game);
        match reread.type_ending_at(m, &text, c.pos(dot)) {
            Some(t) => reread.members_of(&t, game),
            None => Vec::new(),
        }
    }

    /// The type the checker gave the name, member or `self` written in module `m` (its text `text`) that ends at `end`.
    fn type_ending_at(&self, m: usize, text: &str, end: Pos) -> Option<Ty> {
        let mut found = Vec::new();
        walk_module(self.parsed.get(m)?, &mut found);
        let self_at = |at: Pos| text.split('\n').nth(at.line as usize - 1).is_some_and(|l| l.chars().skip(at.col as usize - 1).take(4).eq("self".chars()));
        found
            .iter()
            .filter(|(_, at)| at.line == end.line && at.col + if self_at(*at) { 4 } else { name_len(text, *at) } == end.col)
            .find_map(|(id, _)| self.checked.exprs.get(&(m, *id)))
            .filter(|t| !matches!(t, Ty::Error))
            .cloned()
    }

    /// The members a name chain stands for when its first name is a module, an enum or a service: None when it is a
    /// value (its type decides).
    fn chain_members(&self, m: usize, chain: &str, game: &GameView) -> Option<Vec<Completion>> {
        let parts: Vec<&str> = chain.split('.').collect();
        let scope = self.checked.scopes.get(m)?;
        let mut out = Vec::new();
        match scope.names.get(parts[0])? {
            Name::Module { target: ModTarget::Module { owner, module }, .. } => {
                let r = ModRef { owner: *owner, module: *module };
                let their = self.scope_of(r)?;
                match parts.get(1) {
                    None => {
                        for (name, n) in &their.names {
                            if let Name::Item { item, sort, public, .. } = n {
                                if *public || *owner == Owner::Mod {
                                    let ir = r.item(*item);
                                    out.push(Completion { label: name.clone(), kind: sort_kind(*sort), detail: self.short_text(ir), doc: self.doc_of_item(ir), replace: None, group: 1 });
                                }
                            }
                        }
                    }
                    Some(second) => {
                        let Name::Item { item, .. } = their.names.get(*second)? else { return None };
                        out.extend(self.cases_of(r.item(*item)));
                    }
                }
            }
            Name::Module { target: ModTarget::Service(service), .. } => {
                let face = game.game.services.faces.get(service)?;
                match parts.get(1) {
                    None => {
                        for f in &face.functions {
                            out.push(Completion { label: f.name.clone(), kind: Kind::Function, detail: service_fn_label(service, f), doc: f.comment.clone(), replace: None, group: 1 });
                        }
                        out.extend(face.structs.keys().chain(face.handles.iter()).map(|s| Completion { label: s.clone(), kind: Kind::Struct, detail: format!("svc.{service}"), doc: String::new(), replace: None, group: 1 }));
                        out.extend(face.enums.keys().map(|s| Completion { label: s.clone(), kind: Kind::Enum, detail: format!("svc.{service}"), doc: String::new(), replace: None, group: 1 }));
                        out.extend(face.constants.iter().map(|(k, v)| Completion { label: k.clone(), kind: Kind::Constant, detail: v.to_string(), doc: String::new(), replace: None, group: 1 }));
                        if !face.stream.is_empty() {
                            out.push(Completion { label: "Event".into(), kind: Kind::Enum, detail: format!("svc.{service}"), doc: String::new(), replace: None, group: 1 });
                        }
                    }
                    Some(&"Event") => out.extend(face.stream.iter().map(|(k, s)| Completion { label: capitalized(k), kind: Kind::Case, detail: s.number.to_string(), doc: String::new(), replace: None, group: 1 })),
                    Some(second) => out.extend(face.enums.get(*second)?.iter().map(|(case, n)| Completion { label: case.clone(), kind: Kind::Case, detail: n.to_string(), doc: String::new(), replace: None, group: 1 })),
                }
            }
            Name::Item { item, sort: check::Sort::Enum, .. } if parts.len() == 1 => out.extend(self.cases_of(ItemRef { owner: Owner::Mod, module: m, item: *item })),
            Name::Taken { item, .. } if parts.len() == 1 && matches!(self.item_ast(*item).map(|i| &i.kind), Some(ItemKind::Enum(_))) => out.extend(self.cases_of(*item)),
            _ => return None,
        }
        Some(out)
    }

    fn scope_of(&self, r: ModRef) -> Option<&check::Scope> {
        match r.owner {
            Owner::Mod => self.checked.scopes.get(r.module),
            Owner::Library(l) => self.checked.library_scopes.get(l)?.get(r.module),
        }
    }

    fn cases_of(&self, r: ItemRef) -> Vec<Completion> {
        let Some(ItemKind::Enum(e)) = self.item_ast(r).map(|i| &i.kind) else { return Vec::new() };
        let text = self.module_text_of(r.module_ref()).unwrap_or_default();
        e.cases
            .iter()
            .map(|c| {
                let fields = c.fields.iter().map(|f| format!("{}: {}", f.name.name, check::type_text(&f.ty))).collect::<Vec<_>>().join(", ");
                let detail = if fields.is_empty() { String::new() } else if c.braces { format!("{{ {fields} }}") } else { format!("({fields})") };
                Completion { label: c.name.name.clone(), kind: Kind::Case, detail, doc: super::query::doc_at(&text, c.name.at.line), replace: None, group: 1 }
            })
            .collect()
    }

    fn module_text_of(&self, r: ModRef) -> Option<String> {
        match r.owner {
            Owner::Mod => self.modules.get(r.module).map(|m| m.text.clone()),
            Owner::Library(l) => self.libraries.get(l)?.parts.get(r.module).map(|p| p.text.clone()),
        }
    }

    fn doc_of_item(&self, r: ItemRef) -> String {
        match (self.item_ast(r), self.module_text_of(r.module_ref())) {
            (Some(it), Some(text)) => super::query::doc_at(&text, it.at.line),
            _ => String::new(),
        }
    }

    /// An item's head as completion detail: a function's parameters and result, a type's kind, a binding's type.
    fn short_text(&self, r: ItemRef) -> String {
        let Some(it) = self.item_ast(r) else { return String::new() };
        match &it.kind {
            ItemKind::Fn(f) => format!("({}){}", params_text(&f.params), ret_text(&f.ret)),
            ItemKind::Task { params, .. } | ItemKind::Extern { params, .. } | ItemKind::Export { params, .. } => format!("({})", params_text(params)),
            ItemKind::Struct(_) => "struct".into(),
            ItemKind::Enum(_) => "enum".into(),
            ItemKind::Class(c) => format!("class : {}", c.base.text),
            ItemKind::Option { ty, .. } => check::type_text(ty),
            ItemKind::Binding(b) => b.ty.as_ref().map(check::type_text).unwrap_or_default(),
            ItemKind::States(s) => format!("states for {}", s.class.name),
            _ => String::new(),
        }
    }

    /// The members of a value of type `t`: fields, functions, a game record's (its bases' too), a vector's
    /// components, the builtin methods.
    fn members_of(&self, t: &Ty, game: &GameView) -> Vec<Completion> {
        let mut out = Vec::new();
        let field = |label: String, detail: String, doc: String| Completion { label, kind: Kind::Field, detail, doc, replace: None, group: 0 };
        let func = |label: String, detail: String, doc: String| Completion { label, kind: Kind::Function, detail, doc, replace: None, group: 0 };
        match t {
            Ty::Opt(inner) => return self.members_of(inner, game),
            Ty::Struct(r) | Ty::Class(r) => {
                let text = self.module_text_of(r.module_ref()).unwrap_or_default();
                let doc = |at: Pos| super::query::doc_at(&text, at.line);
                match self.item_ast(*r).map(|i| &i.kind) {
                    Some(ItemKind::Struct(s)) => {
                        out.extend(s.fields.iter().map(|f| field(f.name.name.clone(), check::type_text(&f.ty), doc(f.name.at))));
                        out.extend(s.fns.iter().map(|f| func(f.name.name.clone(), format!("({}){}", params_text(&f.params), ret_text(&f.ret)), doc(f.name.at))));
                    }
                    Some(ItemKind::Class(c)) => {
                        for member in &c.members {
                            match member {
                                ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => out.push(func(f.name.name.clone(), format!("({}){}", params_text(&f.params), ret_text(&f.ret)), doc(f.name.at))),
                                ast::ClassMember::Binding(b) => {
                                    if let Some(n) = binding_name(b) {
                                        out.push(field(n.name.clone(), b.ty.as_ref().map(check::type_text).unwrap_or_default(), doc(n.at)));
                                    }
                                }
                                ast::ClassMember::Hook(_) => {}
                            }
                        }
                        let own: std::collections::BTreeSet<String> = out.iter().map(|c| c.label.clone()).collect();
                        out.extend(record_members(game, &c.base.text).into_iter().filter(|c| !own.contains(&c.label)));
                    }
                    _ => {}
                }
            }
            Ty::Game(GameTy::Struct(record) | GameTy::View { record, .. }) => out.extend(record_members(game, record)),
            Ty::Game(GameTy::Place { of, .. }) => {
                if let GameTy::Struct(record) | GameTy::View { record, .. } = of.as_ref() {
                    out.extend(record_members(game, record));
                }
            }
            Ty::Group { record, prefix, .. } => {
                if let Some(r) = game.game.types.as_ref().and_then(|t| t.records.get(record)) {
                    let lead = format!("{prefix}.");
                    let mut names: BTreeMap<String, String> = BTreeMap::new();
                    for (f, ty) in &r.fields {
                        if let Some(rest) = f.strip_prefix(&lead) {
                            names.entry(rest.split('.').next().unwrap_or(rest).to_string()).or_insert_with(|| ty.clone());
                        }
                    }
                    out.extend(names.into_iter().map(|(n, ty)| field(n, ty, String::new())));
                }
            }
            Ty::Service { service, name } => {
                if let Some(fields) = game.game.services.faces.get(service).and_then(|f| f.structs.get(name)) {
                    out.extend(fields.iter().map(|(n, ty)| field(n.clone(), sty_text(ty), String::new())));
                }
            }
            Ty::Value(v) => {
                let parts: &[&str] = match v {
                    lumascript::game::Value::Vec2 => &["x", "y"],
                    lumascript::game::Value::Vec3 => &["x", "y", "z"],
                    lumascript::game::Value::Vec4 | lumascript::game::Value::Quat => &["x", "y", "z", "w"],
                    lumascript::game::Value::Color => &["r", "g", "b", "a"],
                    lumascript::game::Value::Mat34 => &[],
                };
                out.extend(parts.iter().map(|p| field(p.to_string(), "f32".into(), String::new())));
            }
            _ => {}
        }
        for method in check::methods_of(t) {
            let params = method.params.iter().map(|(n, ty)| format!("{n}: {}", self.ty_text(ty, game))).collect::<Vec<_>>().join(", ");
            let ret = self.ty_text(&method.ret, game);
            let detail = if method.closure { "(fn)".to_string() } else if ret.is_empty() || ret == "()" { format!("({params})") } else { format!("({params}) -> {ret}") };
            out.push(func(method.name.to_string(), detail, String::new()));
        }
        out
    }

    /// The call being written at `at`: its function's signatures (every overload of a game function), the one in use
    /// and the parameter the place is in.
    pub fn signatures(&self, m: usize, at: Pos, game: &GameView) -> Option<Signatures> {
        let module = self.modules.get(m)?;
        let c = Chars::new(&module.text);
        let toks = spans(&c, &module.text);
        let p = c.offset(at);
        if spot(&c, &toks, p) != Spot::Code {
            return None;
        }
        // The innermost `(` open at the place, and the commas after it at its depth.
        let before: Vec<&(lex::Tok, usize, usize)> = toks.iter().filter(|(_, s, _)| *s < p).collect();
        let (mut depth, mut commas, mut open) = (0i32, 0u32, None);
        for (i, (t, _, _)) in before.iter().enumerate().rev() {
            if t.kind != TokKind::Op {
                continue;
            }
            match t.text.as_str() {
                ")" | "]" | "}" => depth += 1,
                "(" | "[" | "{" if depth > 0 => depth -= 1,
                "(" => {
                    open = Some(i);
                    break;
                }
                "[" | "{" => return None,
                "," if depth == 0 => commas += 1,
                _ => {}
            }
        }
        let open = open?;
        // The callee: a backquoted name, a path `A::b`, or a name, maybe after `value.`.
        let name_tok = before.get(open.checked_sub(1)?)?;
        let (callee_at, path) = match name_tok.0.kind {
            TokKind::Game => (name_tok.0.at, name_tok.0.text.clone()),
            TokKind::Id | TokKind::Kw => {
                let mut parts = vec![name_tok.0.text.clone()];
                let mut j = open - 1;
                while j >= 2 && before[j - 1].0.is("::") && matches!(before[j - 2].0.kind, TokKind::Id) {
                    parts.insert(0, before[j - 2].0.text.clone());
                    j -= 2;
                }
                (before[j].0.at, parts.join("::"))
            }
            _ => return None,
        };
        let name_at = name_tok.0.at;
        // What the checker resolved the call to, when the call is read.
        let resolved = self.occurrence_target(m, name_at).or_else(|| self.occurrence_target(m, callee_at));
        let list = match resolved {
            Some(Target::R(Resolved::GameFn { name, signature })) => {
                let mut list = game_signatures(game, &name);
                if let Some(i) = list.iter().position(|s| s.label.ends_with(&signature)) {
                    let chosen = list.remove(i);
                    list.insert(0, chosen);
                }
                list
            }
            Some(Target::R(Resolved::Item(r))) => self.item_signature(r, None).into_iter().collect(),
            Some(Target::R(Resolved::Member { owner, name })) => self.item_signature(owner, Some(&name)).into_iter().collect(),
            Some(Target::R(Resolved::ServiceFn { service, name, .. })) => {
                let f = game.game.services.faces.get(&service)?.function(&name)?;
                vec![service_signature(&service, f)]
            }
            _ => {
                // The name read by the scopes when the call does not read yet.
                let dotted = before.get(open.wrapping_sub(2)).is_some_and(|t| t.0.is("."));
                if path.contains("::") || name_tok.0.kind == TokKind::Game {
                    game_signatures(game, &path)
                } else if dotted {
                    let base = before.get(open.wrapping_sub(3)).filter(|t| t.0.kind == TokKind::Id)?;
                    let scope = self.checked.scopes.get(m)?;
                    match scope.names.get(&base.0.text)? {
                        Name::Module { target: ModTarget::Module { owner, module }, .. } => {
                            let r = ModRef { owner: *owner, module: *module };
                            let Name::Item { item, .. } = self.scope_of(r)?.names.get(&path)? else { return None };
                            self.item_signature(r.item(*item), None).into_iter().collect()
                        }
                        Name::Module { target: ModTarget::Service(service), .. } => {
                            let f = game.game.services.faces.get(service)?.function(&path)?;
                            vec![service_signature(service, f)]
                        }
                        _ => return None,
                    }
                } else {
                    match self.checked.scopes.get(m)?.names.get(&path)? {
                        Name::Item { item, .. } => self.item_signature(ItemRef { owner: Owner::Mod, module: m, item: *item }, None).into_iter().collect(),
                        Name::Taken { item, .. } => self.item_signature(*item, None).into_iter().collect(),
                        _ => return None,
                    }
                }
            }
        };
        if list.is_empty() {
            return None;
        }
        // The first signature with a parameter for the place.
        let active = list.iter().position(|s| s.params.len() as u32 > commas).unwrap_or(0);
        Some(Signatures { list, active, param: commas })
    }

    fn occurrence_target(&self, m: usize, at: Pos) -> Option<Target> {
        self.occurrences.iter().find(|o| o.module == m && o.at == at).map(|o| o.target.clone())
    }

    /// A function's (or a struct's or class's function `member`'s) signature from its declaration.
    fn item_signature(&self, r: ItemRef, member: Option<&str>) -> Option<Signature> {
        let it = self.item_ast(r)?;
        let text = self.module_text_of(r.module_ref()).unwrap_or_default();
        let (name, params, ret, at) = match (&it.kind, member) {
            (ItemKind::Fn(f), None) => (f.name.name.clone(), &f.params, f.ret.clone(), it.at),
            (ItemKind::Task { name, params, .. } | ItemKind::Extern { name, params, .. }, None) => (name.name.clone(), params, None, it.at),
            (ItemKind::Export { name, params, ret, .. }, None) => (name.name.clone(), params, ret.clone(), it.at),
            (ItemKind::Struct(s), Some(mname)) => {
                let f = s.fns.iter().find(|f| f.name.name == mname)?;
                (f.name.name.clone(), &f.params, f.ret.clone(), f.name.at)
            }
            (ItemKind::Class(c), Some(mname)) => {
                let f = c.members.iter().find_map(|x| match x {
                    ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) if f.name.name == mname => Some(f),
                    _ => None,
                })?;
                (f.name.name.clone(), &f.params, f.ret.clone(), f.name.at)
            }
            _ => return None,
        };
        let shown: Vec<ast::Param> = params.iter().filter(|p| !p.is_self).cloned().collect();
        let each: Vec<String> = shown.iter().map(|p| params_text(std::slice::from_ref(p))).collect();
        Some(Signature { label: format!("{name}({}){}", each.join(", "), ret_text(&ret)), params: each, doc: super::query::doc_at(&text, at.line) })
    }
}

/// The ranges names are bound in: each function, task, hook, handler and closure from its head to its body's end, each
/// loop and arm from its pattern to its block's end, each block.
fn scope_ranges(module: &ast::Module) -> Vec<(Pos, Pos)> {
    fn block(b: &ast::Block, out: &mut Vec<(Pos, Pos)>) {
        out.push((b.at, b.end));
        for s in &b.stmts {
            stmt(s, out);
        }
    }
    fn fn_decl(at: Pos, f: &ast::FnDecl, out: &mut Vec<(Pos, Pos)>) {
        out.push((at, f.body.end));
        block(&f.body, out);
    }
    fn stmt(s: &ast::Stmt, out: &mut Vec<(Pos, Pos)>) {
        use ast::StmtKind as S;
        match &s.kind {
            S::Binding(b) => {
                if let Some(v) = &b.value {
                    expr(v, out);
                }
            }
            S::Fn(f) => fn_decl(s.at, f, out),
            S::Guard { conds, otherwise } => {
                conds.iter().for_each(|c| expr(&c.value, out));
                block(otherwise, out);
            }
            S::While { cond, body } => {
                expr(cond, out);
                block(body, out);
            }
            S::For { iter, body, .. } => {
                out.push((s.at, body.end));
                expr(iter, out);
                block(body, out);
            }
            S::Defer(st) => stmt(st, out),
            S::Return(Some(e)) | S::Skip(Some(e)) | S::Expect(e) | S::Expr(e) | S::Fail(e) => expr(e, out),
            S::Assign { value, .. } => expr(value, out),
            S::Log { value, .. } => expr(value, out),
            _ => {}
        }
    }
    fn expr(e: &ast::Expr, out: &mut Vec<(Pos, Pos)>) {
        match &e.kind {
            ExprKind::If { arms, otherwise } => {
                for (conds, b) in arms {
                    let start = conds.first().map_or(b.at, |c| c.pattern.as_ref().map_or(c.value.at, |p| p.at));
                    out.push((start, b.end));
                    conds.iter().for_each(|c| expr(&c.value, out));
                    block(b, out);
                }
                if let Some(b) = otherwise {
                    block(b, out);
                }
            }
            ExprKind::Match { subject, arms } => {
                expr(subject, out);
                for arm in arms {
                    if let ast::ArmBody::Block(b) = &arm.body {
                        let start = arm.pattern.as_ref().map_or(b.at, |p| p.at);
                        out.push((start, b.end));
                        block(b, out);
                    } else if let (Some(p), ast::ArmBody::Stmt(s)) = (&arm.pattern, &arm.body) {
                        out.push((p.at, Pos { line: s.at.line, col: u32::MAX }));
                    }
                }
            }
            ExprKind::Closure { body, .. } => {
                out.push((e.at, body.end));
                block(body, out);
            }
            ExprKind::Call { callee, args } => {
                expr(callee, out);
                args.iter().for_each(|a| expr(&a.value, out));
            }
            ExprKind::Paren(x) | ExprKind::Unwrap(x) | ExprKind::Neg(x) | ExprKind::Not(x) | ExprKind::WaitUntil(x) | ExprKind::Start(x) => expr(x, out),
            ExprKind::Binary { left, right, .. } => {
                expr(left, out);
                expr(right, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for item in &module.items {
        match &item.kind {
            ItemKind::Fn(f) => fn_decl(item.at, f, &mut out),
            ItemKind::Task { body, .. } | ItemKind::Export { body, .. } | ItemKind::Test { body, .. } => {
                out.push((item.at, body.end));
                block(body, &mut out);
            }
            ItemKind::Hook(h) => {
                out.push((h.at, h.body.end));
                block(&h.body, &mut out);
            }
            ItemKind::On(o) => {
                out.push((o.at, o.body.end));
                block(&o.body, &mut out);
            }
            ItemKind::Struct(s) => s.fns.iter().for_each(|f| fn_decl(f.name.at, f, &mut out)),
            ItemKind::Class(c) => {
                for member in &c.members {
                    match member {
                        ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => fn_decl(f.name.at, f, &mut out),
                        ast::ClassMember::Hook(h) => {
                            out.push((h.at, h.body.end));
                            block(&h.body, &mut out);
                        }
                        ast::ClassMember::Binding(_) => {}
                    }
                }
            }
            ItemKind::States(s) => {
                for st in &s.states {
                    for part in &st.parts {
                        if let ast::StatePart::Enter(b) | ast::StatePart::Update(b) | ast::StatePart::Exit(b) = part {
                            block(b, &mut out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// After `Path::`: the game names one part longer (functions, records, enums, namespaces) and a game enum's values.
fn game_children(game: &GameView, path: &str) -> Vec<Completion> {
    let lead = format!("{path}::");
    let mut found: BTreeMap<String, (Kind, String)> = BTreeMap::new();
    for (name, sigs) in &game.game.functions {
        if let Some(rest) = name.strip_prefix(&lead) {
            match rest.split_once("::") {
                None => {
                    found.insert(rest.to_string(), (Kind::Function, sigs.first().map(|s| s.0.clone()).unwrap_or_default()));
                }
                Some((head, _)) => {
                    found.entry(head.to_string()).or_insert((Kind::Module, String::new()));
                }
            }
        }
    }
    if let Some(types) = &game.game.types {
        for (names, kind) in [(types.records.keys().collect::<Vec<_>>(), Kind::Class), (types.enums.keys().collect(), Kind::Enum), (types.globals.keys().collect(), Kind::Variable)] {
            for name in names {
                if let Some(rest) = name.strip_prefix(&lead) {
                    let head = rest.split("::").next().unwrap_or(rest);
                    found.entry(head.to_string()).or_insert((kind, String::new()));
                }
            }
        }
        if let Some(e) = types.enums.get(path) {
            for (v, n) in &e.values {
                found.insert(v.clone(), (Kind::Case, n.to_string()));
            }
        }
    }
    found.into_iter().filter(|(n, _)| !n.is_empty()).map(|(label, (kind, detail))| Completion { label, kind, detail, doc: String::new(), replace: None, group: 0 }).collect()
}

/// A game record's fields and functions, its bases' too, each once.
fn record_members(game: &GameView, record: &str) -> Vec<Completion> {
    let mut out = Vec::new();
    let Some(types) = game.game.types.as_ref() else { return out };
    let mut seen = std::collections::BTreeSet::new();
    if let Some(r) = types.records.get(record) {
        let mut fields: Vec<(&String, &String)> = r.fields.iter().collect();
        fields.sort();
        for (f, t) in fields {
            // A member of an anonymous part (`mFlags.isGettable`) is reached through the part.
            let head = f.split('.').next().unwrap_or(f);
            if seen.insert(head.to_string()) {
                out.push(Completion { label: head.to_string(), kind: Kind::Field, detail: if head == f { t.clone() } else { String::new() }, doc: String::new(), replace: None, group: 0 });
            }
        }
    }
    // The record, then its bases breadth first.
    let mut queue = vec![record.to_string()];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(r) = queue.first().cloned() {
        queue.remove(0);
        if !visited.insert(r.clone()) {
            continue;
        }
        let lead = format!("{r}::");
        let short = r.rsplit("::").next().unwrap_or(&r).to_string();
        let mut fns: Vec<(&String, &Vec<(String, bool)>)> = game.game.functions.iter().filter(|(n, _)| n.strip_prefix(&lead).is_some_and(|rest| !rest.contains("::"))).collect();
        fns.sort_by(|a, b| a.0.cmp(b.0));
        for (n, sigs) in fns {
            let name = &n[lead.len()..];
            // Constructors and destructors are not called by name.
            if name == short || name.starts_with('~') {
                continue;
            }
            if seen.insert(name.to_string()) {
                out.push(Completion { label: name.to_string(), kind: Kind::Function, detail: sigs.first().map(|s| s.0.clone()).unwrap_or_default(), doc: String::new(), replace: None, group: 1 });
            }
        }
        if let Some(rec) = types.records.get(&r) {
            queue.extend(rec.bases.iter().map(|b| b.name.clone()));
        }
    }
    out
}

/// Each signature of a game function, its parameters split at the top level.
fn game_signatures(game: &GameView, name: &str) -> Vec<Signature> {
    let Some(sigs) = game.game.functions.get(name) else { return Vec::new() };
    sigs.iter()
        .map(|(signature, _)| {
            let inner = signature.find('(').and_then(|o| signature.rfind(')').map(|c| &signature[o + 1..c])).unwrap_or("");
            Signature { label: format!("{name}{signature}"), params: split_top(inner), doc: String::new() }
        })
        .collect()
}

fn service_fn_label(service: &str, f: &lumascript::svc::SFn) -> String {
    service_signature(service, f).label
}

fn service_signature(service: &str, f: &lumascript::svc::SFn) -> Signature {
    use lumascript::svc::{SParam, Shape};
    let params: Vec<String> = match &f.shape {
        Shape::Callable { params, .. } => params
            .iter()
            .map(|p| match p {
                SParam::Value { name, ty } => format!("{name}: {}", sty_text(ty)),
                SParam::Closure { optional } => if *optional { "fn?".to_string() } else { "fn".to_string() },
                SParam::Callback { name, .. } => format!("{name}: fn"),
            })
            .collect(),
        _ => Vec::new(),
    };
    Signature { label: format!("svc.{service}.{}({})", f.name, params.join(", ")), params, doc: f.comment.clone() }
}

/// A service value's type as a script writes it.
fn sty_text(t: &lumascript::svc::STy) -> String {
    use lumascript::svc::STy;
    match t {
        STy::Bool => "bool".into(),
        STy::Number(c) => match c.as_str() {
            "int8_t" => "i8",
            "int16_t" => "i16",
            "int32_t" => "i32",
            "int64_t" => "i64",
            "uint8_t" => "u8",
            "uint16_t" => "u16",
            "uint32_t" => "u32",
            "uint64_t" => "u64",
            "float" => "f32",
            "double" => "f64",
            other => other,
        }
        .to_string(),
        STy::Text => "text".into(),
        STy::Bytes => "bytes".into(),
        STy::Struct(n) | STy::Enum(n) | STy::Handle(n) => n.clone(),
        STy::HandleOf(o, k) | STy::StructOf(o, k) => format!("{o}.{k}"),
        STy::Vec(n) => format!("vec{n}"),
        STy::Shared(c) => format!("content.{}", c.strip_prefix("Luma").unwrap_or(c)),
        STy::Actor => "LiveActor".into(),
        STy::ContentName(w) => w.clone(),
        STy::Opt(x) => format!("{}?", sty_text(x)),
        STy::Color => "color".into(),
        STy::List(x) => format!("list<{}>", sty_text(x)),
        STy::Answer => "svc.Answer".into(),
        STy::Fixed(x, n) => format!("({})", vec![sty_text(x); *n as usize].join(", ")),
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Splits a parameter list at its top-level commas.
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in text.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(text[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = text[start..].trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    out
}

pub(super) fn sort_kind(sort: check::Sort) -> Kind {
    use check::Sort;
    match sort {
        Sort::Fn | Sort::Task | Sort::Extern | Sort::Export => Kind::Function,
        Sort::Struct => Kind::Struct,
        Sort::Enum => Kind::Enum,
        Sort::Class | Sort::States => Kind::Class,
        Sort::Option | Sort::Let => Kind::Constant,
        Sort::Var => Kind::Variable,
    }
}
