//! What an editor asks of a mod's LumaScript, answered from the compiler's own reading and the game's
//! self-description: each module's diagnostics, what a name stands for (hover), where it is made (go to definition),
//! every place it is used (references, rename), what may be written at a place (completion), the call being written
//! (signature help) and a module's items (symbols). [`crate::lsp`] speaks it to an editor; nothing here reads stdin.
//!
//! Places are the compiler's: line and column from 1, the column in characters. [`crate::lsp`] turns them into the
//! protocol's (from 0, the column in UTF-16 units).

use lumascript::ast::{self, ExprId, ExprKind, Item, ItemKind, Pos};
use lumascript::check::{self, Checked, ItemRef, Owner, Resolved, Source, Ty};
use lumascript::game::{Game, GameTy};
use lumascript::Diagnostic;
use std::collections::{BTreeMap, HashMap};

mod complete;
mod query;

pub use complete::{Completion, Completions, Signature, Signatures, MOST_GAME_NAMES};
pub use query::{doc_at, item_name, Found, Hover, OutlineItem, RenameRefused, Target};
use std::path::{Path, PathBuf};

/// The game a mod is read against: the compiler's view of it (the same `lumalink build` checks with,
/// [`crate::build::game_of`]), and the generated reference pages its names link to.
#[derive(Default)]
pub struct GameView {
    pub game: Game,
    /// The self-description folder it was read from.
    pub folder: Option<PathBuf>,
    /// The SDK include folder the services' faces were read from.
    pub sdk: Option<PathBuf>,
    /// The reference pages (`<folder>/luma_docs/reference`, generated from the self-description) by the name each
    /// one describes, from the pages' own `search.json`: a game record, enum, global or function to its page.
    pub pages: HashMap<String, PathBuf>,
    /// The services of the SDK whose face does not read (each with why): their scripts find no such module.
    pub unread: Vec<String>,
}

impl GameView {
    /// The game of a self-description folder (function rows, `luma_types.json`, `luma_functions.json`, the placement
    /// facts, the numbers), with the services' faces from the SDK include folder `sdk` (else the folder's own `sdk`
    /// when it has one). Err: why it does not read.
    pub fn read(folder: &Path, sdk: Option<&Path>) -> Result<GameView, String> {
        let rows = luma_validate::names::Names::from_rows_folder(folder, 1).map_err(|e| e.to_string())?;
        let own = folder.join("sdk");
        let sdk = sdk.map(Path::to_path_buf).or_else(|| own.join("luma.h").is_file().then_some(own));
        let (game, unread) = crate::build::game_of(&rows, folder, sdk.as_deref()).map_err(|e| match e {
            crate::build::GameError::Tables(t) => t.to_string(),
            crate::build::GameError::Factory(crate::build::FactoryError::NotJson { path, line, column }) => format!("{}:{line}:{column}: not JSON", path.display()),
            crate::build::GameError::Factory(crate::build::FactoryError::Shape(path)) => format!("{}: `factory` is not a list of name and class", path.display()),
        })?;
        Ok(GameView { game, folder: Some(folder.to_path_buf()), sdk, pages: reference_pages(folder), unread: unread.iter().map(|e| e.to_string()).collect() })
    }

    /// The reference page of a game name: its own, else its record's for `Record::member` and `Record.field`.
    pub fn page(&self, name: &str) -> Option<&PathBuf> {
        self.pages.get(name).or_else(|| {
            let owner = name.rsplit_once("::").map(|(o, _)| o).or_else(|| name.split_once('.').map(|(o, _)| o))?;
            self.pages.get(owner)
        })
    }
}

/// The reference pages of a self-description folder by name (`luma_docs/reference/search.json`: [{name, kind, page}]);
/// none when the folder has no pages.
fn reference_pages(folder: &Path) -> HashMap<String, PathBuf> {
    let root = folder.join("luma_docs").join("reference");
    let Ok(bytes) = std::fs::read(root.join("search.json")) else { return HashMap::new() };
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return HashMap::new() };
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        if let (Some(name), Some(page)) = (row.get("name").and_then(|v| v.as_str()), row.get("page").and_then(|v| v.as_str())) {
            out.entry(name.to_string()).or_insert_with(|| root.join(page));
        }
    }
    out
}

/// One module of the mod as the editor has it.
#[derive(Clone, Debug)]
pub struct Module {
    /// Its path in `source/` without `.lum` (`main`, `ball/fly`).
    pub path: String,
    pub file: PathBuf,
    pub text: String,
}

/// What a name at a place stands for, with the place it is at.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub module: usize,
    pub at: Pos,
    /// Its length in characters.
    pub len: u32,
    pub target: Target,
    /// The expression it is (for its type), when it is one.
    pub expr: Option<ExprId>,
}

/// A place in a file: the file and the start and end (end exclusive) in the compiler's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: PathBuf,
    pub at: Pos,
    pub end: Pos,
}

/// The kind of a completion or a symbol, mapped by [`crate::lsp`] to the protocol's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Function,
    Variable,
    Constant,
    Struct,
    Enum,
    Case,
    Class,
    Field,
    Module,
    Event,
}

/// A library mod the mod needs, as the editor reads it: its interface file, each module's part of it, and the
/// library's own source folder beside the mod when it is there (`../<id>/source`), for go to definition.
pub struct LibraryView {
    pub id: String,
    pub lumi: PathBuf,
    pub parts: Vec<lumascript::lumi::Part>,
    /// The parsed modules, in the checker's order (`Owner::Library(i)` is this library's index).
    pub modules: Vec<ast::Module>,
    pub source: Option<PathBuf>,
}

/// A mod read: every module parsed and checked once, and every name's occurrence found.
pub struct Analysis {
    pub root: PathBuf,
    pub modules: Vec<Module>,
    pub parsed: Vec<ast::Module>,
    /// Every diagnostic by module index: what `lumalink build` says of that module.
    pub diags: Vec<Vec<Diagnostic>>,
    /// What the build says of no module (a needed mod's interface refused, luma.json without an id), for luma.json.
    pub outside: Vec<(String, Diagnostic)>,
    pub checked: Checked,
    pub occurrences: Vec<Occurrence>,
    pub libraries: Vec<LibraryView>,
}

/// The mod folder holding `file`: the nearest folder above it with a `luma.json`.
pub fn mod_root(file: &Path) -> Option<PathBuf> {
    file.ancestors().skip(1).find(|d| d.join("luma.json").is_file()).map(Path::to_path_buf)
}

/// Every module under `<root>/source` (sorted by path, docs/modding/language/compiled-form.md, section 1.2), each with the editor's text when it has the file
/// open (`open`, by file), else the file's.
pub fn modules(root: &Path, open: &HashMap<PathBuf, String>) -> Vec<Module> {
    let source = root.join("source");
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(&source).sort_by_file_name().into_iter().flatten() {
        let file = entry.path();
        if !entry.file_type().is_file() || file.extension().is_none_or(|e| e != "lum") {
            continue;
        }
        let rel = file.strip_prefix(&source).unwrap_or(file).with_extension("");
        let path = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        let text = match open.get(file) {
            Some(text) => text.clone(),
            None => std::fs::read_to_string(file).unwrap_or_default(),
        };
        out.push(Module { path, file: file.to_path_buf(), text });
    }
    // A file open in the editor that is not on disk yet is a module too.
    for (file, text) in open {
        if file.starts_with(&source) && file.extension().is_some_and(|e| e == "lum") && !out.iter().any(|m| &m.file == file) {
            let rel = file.strip_prefix(&source).unwrap_or(file).with_extension("");
            let path = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            out.push(Module { path, file: file.clone(), text: text.clone() });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The check inputs of the mod at `root` ([`crate::build::CheckInputs`]); when its luma.json does not read or has no
/// id, the inputs that do not need it (the C headers, the mod's own content), so its modules are still read.
fn inputs(root: &Path) -> crate::build::CheckInputs {
    crate::build::CheckInputs::read(root).unwrap_or_else(|_| crate::build::CheckInputs {
        id: String::new(),
        version: String::new(),
        needs: Vec::new(),
        options: Vec::new(),
        headers: crate::build::c_sources(root).map(|c| c.headers).unwrap_or_default(),
        content: crate::build::content_names(root, &[]),
        interfaces: Vec::new(),
    })
}

impl Analysis {
    /// Reads and checks the mod at `root` with these modules against `game`, on the compiler's thread, as `lumalink
    /// build` does: the same inputs, the standard module after the mod's own, the needed mods' interfaces, and the
    /// back end when the checker said no error (its refusals are build errors too).
    pub fn new(root: &Path, modules: Vec<Module>, game: &GameView) -> Analysis {
        let inputs = inputs(root);
        let texts: Vec<String> = modules.iter().map(|m| m.text.clone()).collect();
        let paths: Vec<String> = modules.iter().map(|m| m.path.clone()).collect();
        let result = lumascript::on_compiler_stack(|| {
            let mut diags: Vec<Vec<Diagnostic>> = Vec::new();
            let mut sources = Vec::new();
            for (path, text) in paths.iter().zip(&texts) {
                let (parsed, said) = lumascript::parse_module(text);
                diags.push(said);
                sources.push(Source { path: path.clone(), module: parsed });
            }
            let syntax = diags.iter().any(|d| !d.is_empty());
            sources.push(check::std_source());
            let (libraries, outside) = inputs.libraries();
            let checked = check::check(&sources, &libraries, inputs.facts(&game.game));
            // The build says the parse errors and the refused interfaces alone when there are any; else the checker's,
            // then the back end's.
            if !syntax && outside.is_empty() {
                for (path, d) in &checked.diags {
                    if let Some(m) = paths.iter().position(|p| p == path) {
                        diags[m].push(d.clone());
                    }
                }
                let errors = checked.diags.iter().any(|(_, d)| !lumascript::diag::is_note(d.code));
                if !errors && !inputs.id.is_empty() {
                    if let Err(said) = lumascript::emit::emit_with(&inputs.id, &inputs.version, &sources, &texts, &libraries, &checked, &game.game) {
                        for (path, d) in said {
                            if let Some(m) = paths.iter().position(|p| *p == path) {
                                diags[m].push(d);
                            }
                        }
                    }
                }
            }
            sources.pop();
            let library_modules: Vec<(String, Vec<ast::Module>)> = libraries.into_iter().map(|l| (l.id, l.modules.into_iter().map(|s| s.module).collect())).collect();
            let mut occurrences = Vec::new();
            for (m, source) in sources.iter().enumerate() {
                let mut found = Vec::new();
                walk_module(&source.module, &mut found);
                for (id, at) in found {
                    if let Some(target) = checked.resolved.get(&(m, id)) {
                        occurrences.push(Occurrence { module: m, at, len: name_len(&texts[m], at), target: Target::R(target.clone()), expr: Some(id) });
                    }
                }
            }
            for ((m, at), target) in &checked.named {
                occurrences.push(Occurrence { module: *m, at: *at, len: name_len(&texts[*m], *at), target: Target::R(target.clone()), expr: None });
            }
            for (m, source) in sources.iter().enumerate() {
                occurrences.extend(Analysis::named_places(&source.module, m, &texts[m], &checked, &game.game));
            }
            // The walk keeps the first of two at one place: a call's own (its chosen function) before its callee's.
            occurrences.sort_by_key(|o| (o.module, o.at));
            occurrences.dedup_by_key(|o| (o.module, o.at));
            (sources.into_iter().map(|s| s.module).collect::<Vec<_>>(), diags, outside, checked, occurrences, library_modules)
        });
        match result {
            Ok((parsed, diags, outside, checked, occurrences, library_modules)) => {
                let libraries = library_modules
                    .into_iter()
                    // Each library is one of the interfaces read (`CheckInputs::libraries`), in the checker's order.
                    .map(|(id, modules)| {
                        let i = inputs.interfaces.iter().find(|i| i.id == id);
                        let lumi = i.map(|i| i.path.clone()).unwrap_or_default();
                        let parts = i.map(|i| lumascript::lumi::parts(&i.text)).unwrap_or_default();
                        let source = lumi.parent().and_then(Path::parent).map(|p| p.join("source")).filter(|s| s.is_dir());
                        LibraryView { id, lumi, parts, modules, source }
                    })
                    .collect();
                Analysis { root: root.to_path_buf(), modules, parsed, diags, outside, checked, occurrences, libraries }
            }
            Err(d) => {
                let n = modules.len();
                let mut diags = vec![Vec::new(); n];
                if let Some(first) = diags.first_mut() {
                    first.push(d);
                }
                Analysis {
                    root: root.to_path_buf(),
                    modules,
                    parsed: vec![ast::Module { items: Vec::new() }; n],
                    diags,
                    outside: Vec::new(),
                    checked: Checked::default(),
                    occurrences: Vec::new(),
                    libraries: Vec::new(),
                }
            }
        }
    }
}

/// The length in characters of the name written at `at`: a backquoted game name with its quotes, else the word.
pub fn name_len(text: &str, at: Pos) -> u32 {
    let Some(line) = text.lines().nth(at.line.saturating_sub(1) as usize) else { return 1 };
    let rest: Vec<char> = line.chars().skip(at.col.saturating_sub(1) as usize).collect();
    if rest.first() == Some(&'`') {
        return rest.iter().skip(1).position(|c| *c == '`').map_or(rest.len(), |p| p + 2) as u32;
    }
    let word = |from: usize| rest[from..].iter().take_while(|c| c.is_alphanumeric() || **c == '_').count();
    let mut n = word(0);
    // A game name written without backquotes is a path: `MR::isDead`, `Mario::~Mario`.
    while n > 0 && rest.get(n) == Some(&':') && rest.get(n + 1) == Some(&':') {
        let tilde = usize::from(rest.get(n + 2) == Some(&'~'));
        let next = word(n + 2 + tilde);
        if next == 0 {
            break;
        }
        n += 2 + tilde + next;
    }
    n.max(1) as u32
}

// ------------------------------------------------------------------ the walk: every name's expression and place

fn walk_module(module: &ast::Module, out: &mut Vec<(ExprId, Pos)>) {
    for item in &module.items {
        walk_item(item, out);
    }
}

fn walk_params(ps: &[ast::Param], out: &mut Vec<(ExprId, Pos)>) {
    for p in ps {
        if let Some(d) = &p.default {
            walk_expr(d, out);
        }
    }
}

fn walk_fn(f: &ast::FnDecl, out: &mut Vec<(ExprId, Pos)>) {
    walk_params(&f.params, out);
    walk_block(&f.body, out);
}

fn walk_binding(b: &ast::Binding, out: &mut Vec<(ExprId, Pos)>) {
    walk_pattern(&b.pattern, out);
    if let Some(v) = &b.value {
        walk_expr(v, out);
    }
}

fn walk_item(item: &Item, out: &mut Vec<(ExprId, Pos)>) {
    match &item.kind {
        ItemKind::Use(_) | ItemKind::Option { .. } | ItemKind::Extern { .. } => {}
        ItemKind::Binding(b) => walk_binding(b, out),
        ItemKind::Fn(f) => walk_fn(f, out),
        ItemKind::Struct(s) => {
            for f in &s.fields {
                if let Some(d) = &f.default {
                    walk_expr(d, out);
                }
            }
            for f in &s.fns {
                walk_fn(f, out);
            }
        }
        ItemKind::Enum(e) => {
            for c in &e.cases {
                for f in &c.fields {
                    if let Some(d) = &f.default {
                        walk_expr(d, out);
                    }
                }
            }
        }
        ItemKind::Class(c) => {
            for m in &c.members {
                match m {
                    ast::ClassMember::Binding(b) => walk_binding(b, out),
                    ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => walk_fn(f, out),
                    ast::ClassMember::Hook(h) => walk_hook(h, out),
                }
            }
        }
        ItemKind::Task { params, body, .. } => {
            walk_params(params, out);
            walk_block(body, out);
        }
        ItemKind::Hook(h) => walk_hook(h, out),
        ItemKind::On(o) => {
            if let ast::OnEvent::Step(Some(p)) = &o.event {
                walk_params(std::slice::from_ref(p), out);
            }
            walk_block(&o.body, out);
        }
        ItemKind::States(s) => {
            for state in &s.states {
                for part in &state.parts {
                    match part {
                        ast::StatePart::Enter(b) | ast::StatePart::Update(b) | ast::StatePart::Exit(b) => walk_block(b, out),
                        ast::StatePart::Stmt(st) => walk_stmt(st, out),
                    }
                }
            }
        }
        ItemKind::Test { body, .. } => walk_block(body, out),
        ItemKind::Export { params, body, .. } => {
            walk_params(params, out);
            walk_block(body, out);
        }
    }
}

fn walk_hook(h: &ast::Hook, out: &mut Vec<(ExprId, Pos)>) {
    if let Some(ps) = &h.params {
        walk_params(ps, out);
    }
    walk_block(&h.body, out);
}

fn walk_block(b: &ast::Block, out: &mut Vec<(ExprId, Pos)>) {
    for s in &b.stmts {
        walk_stmt(s, out);
    }
}

fn walk_stmt(s: &ast::Stmt, out: &mut Vec<(ExprId, Pos)>) {
    use ast::StmtKind as S;
    match &s.kind {
        S::Binding(b) => walk_binding(b, out),
        S::Fn(f) => walk_fn(f, out),
        S::Return(e) | S::Skip(e) => {
            if let Some(e) = e {
                walk_expr(e, out);
            }
        }
        S::Break | S::Continue | S::Stop | S::Go(_) => {}
        S::Defer(st) => walk_stmt(st, out),
        S::Log { fact, value } => {
            if let Some(f) = fact {
                walk_expr(f, out);
            }
            walk_expr(value, out);
        }
        S::Expect(e) | S::Expr(e) | S::Fail(e) => walk_expr(e, out),
        S::Guard { conds, otherwise } => {
            walk_conds(conds, out);
            walk_block(otherwise, out);
        }
        S::While { cond, body } => {
            walk_expr(cond, out);
            walk_block(body, out);
        }
        S::For { pattern, iter, body } => {
            walk_pattern(pattern, out);
            walk_expr(iter, out);
            walk_block(body, out);
        }
        S::Assign { target, value, .. } => {
            walk_expr(target, out);
            walk_expr(value, out);
        }
    }
}

fn walk_conds(conds: &[ast::Cond], out: &mut Vec<(ExprId, Pos)>) {
    for c in conds {
        if let Some(p) = &c.pattern {
            walk_pattern(p, out);
        }
        walk_expr(&c.value, out);
    }
}

fn walk_pattern(p: &ast::Pattern, out: &mut Vec<(ExprId, Pos)>) {
    match &p.kind {
        ast::PatternKind::Tuple(ps) => ps.iter().for_each(|p| walk_pattern(p, out)),
        ast::PatternKind::Literal(e) => walk_expr(e, out),
        ast::PatternKind::Path { args, .. } => {
            if let Some(args) = args {
                args.iter().for_each(|p| walk_pattern(p, out));
            }
        }
    }
}

fn walk_args(args: &[ast::Arg], out: &mut Vec<(ExprId, Pos)>) {
    for a in args {
        walk_expr(&a.value, out);
    }
}

fn walk_str(s: &ast::StrLit, out: &mut Vec<(ExprId, Pos)>) {
    for p in &s.parts {
        if let ast::StrPart::Expr(e) = p {
            walk_expr(e, out);
        }
    }
}

/// Where a callee's name is written: the name, the member's name, the game name.
fn callee_at(callee: &ast::Expr) -> Option<Pos> {
    match &callee.kind {
        ExprKind::Name { name, .. } | ExprKind::Member { name, .. } => Some(name.at),
        ExprKind::Game(g) => Some(g.at),
        ExprKind::Paren(x) => callee_at(x),
        _ => None,
    }
}

fn walk_expr(e: &ast::Expr, out: &mut Vec<(ExprId, Pos)>) {
    match &e.kind {
        ExprKind::Name { name, .. } => out.push((e.id, name.at)),
        ExprKind::Member { base, name, .. } => {
            walk_expr(base, out);
            out.push((e.id, name.at));
        }
        ExprKind::Game(g) => out.push((e.id, g.at)),
        ExprKind::StructLit { path, fields } => {
            if let Some(last) = path.last() {
                out.push((e.id, last.at));
            }
            for (_, v) in fields {
                if let Some(v) = v {
                    walk_expr(v, out);
                }
            }
        }
        // `self` for its type (a member written after it); the checker resolves it to nothing.
        ExprKind::SelfValue => out.push((e.id, e.at)),
        ExprKind::Number(_) | ExprKind::Color(_) | ExprKind::True | ExprKind::False | ExprKind::None | ExprKind::Result | ExprKind::Proceed => {}
        ExprKind::Str(s) => walk_str(s, out),
        ExprKind::Ref(args) => walk_args(args, out),
        ExprKind::If { arms, otherwise } => {
            for (conds, block) in arms {
                walk_conds(conds, out);
                walk_block(block, out);
            }
            if let Some(b) = otherwise {
                walk_block(b, out);
            }
        }
        ExprKind::Match { subject, arms } => {
            walk_expr(subject, out);
            for arm in arms {
                if let Some(p) = &arm.pattern {
                    walk_pattern(p, out);
                }
                match &arm.body {
                    ast::ArmBody::Block(b) => walk_block(b, out),
                    ast::ArmBody::Stmt(s) => walk_stmt(s, out),
                }
            }
        }
        ExprKind::Closure { params, body, .. } => {
            walk_params(params, out);
            walk_block(body, out);
        }
        ExprKind::Paren(x) | ExprKind::Unwrap(x) | ExprKind::Neg(x) | ExprKind::Not(x) | ExprKind::WaitUntil(x) | ExprKind::Start(x) => walk_expr(x, out),
        ExprKind::Tuple(xs) | ExprKind::List(xs) => xs.iter().for_each(|x| walk_expr(x, out)),
        ExprKind::Braces(kvs) => {
            for (k, v) in kvs {
                walk_expr(k, out);
                if let Some(v) = v {
                    walk_expr(v, out);
                }
            }
        }
        ExprKind::Content { path, args, .. } => {
            walk_str(path, out);
            if let Some(args) = args {
                walk_args(args, out);
            }
        }
        ExprKind::Call { callee, args } => {
            // The checker resolves a call (a game function's chosen signature, a standard name, `base.f`) on the
            // call's own id: it stands at the callee's name, ahead of the callee's.
            if let Some(at) = callee_at(callee) {
                out.push((e.id, at));
            }
            walk_expr(callee, out);
            walk_args(args, out);
        }
        ExprKind::Index { base, index } => {
            walk_expr(base, out);
            walk_expr(index, out);
        }
        ExprKind::Binary { left, right, .. } => {
            walk_expr(left, out);
            walk_expr(right, out);
        }
        ExprKind::Wait { amount, .. } => walk_expr(amount, out),
        ExprKind::As { value, .. } => walk_expr(value, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis(source: &str) -> (tempfile::TempDir, Analysis) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("luma.json"), r#"{"id":"test.mod","version":"1.0.0"}"#).unwrap();
        std::fs::create_dir_all(dir.path().join("source")).unwrap();
        std::fs::write(dir.path().join("source").join("main.lum"), source).unwrap();
        let root = dir.path().to_path_buf();
        let a = Analysis::new(&root, modules(&root, &HashMap::new()), &GameView::default());
        (dir, a)
    }

    #[test]
    fn a_syntax_error_is_said_at_its_place() {
        let (_d, a) = analysis("fn f() {\n    let x = (1 +\n}\n");
        assert!(!a.diags[0].is_empty());
        assert!(a.diags[0].iter().all(|d| d.at.line >= 2));
    }

    #[test]
    fn a_local_s_use_goes_to_its_let_and_its_references_are_both() {
        let (_d, a) = analysis("fn f() -> i32 {\n    let speed = 3\n    return speed + speed\n}\n");
        let game = GameView::default();
        // `speed` in `return speed`: line 3, column 12.
        let target = a.at(0, Pos { line: 3, col: 13 }).expect("a name");
        assert!(matches!(target, Target::R(Resolved::Local(_))));
        let def = a.definition(&target, &game).unwrap();
        assert_eq!(def.at, Pos { line: 2, col: 9 });
        assert_eq!(a.references(&target, &game).len(), 3);
        assert!(a.hover(0, Pos { line: 3, col: 12 }, &game).unwrap().code.starts_with("let speed"));
        let renamed = a.rename(0, Pos { line: 3, col: 12 }, "pace", &game).unwrap();
        assert_eq!(renamed.len(), 3);
        assert!(a.rename(0, Pos { line: 3, col: 12 }, "fn", &game).is_err());
    }

    #[test]
    fn completion_gives_the_keywords_the_items_and_the_locals_before_the_place() {
        let (_d, a) = analysis("fn helper() {}\nfn f() {\n    let near = 1\n    \n}\n");
        let labels: Vec<String> = a.complete(0, Pos { line: 4, col: 5 }, &GameView::default()).items.into_iter().map(|c| c.label).collect();
        for want in ["let", "helper", "near", "model"] {
            assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
        }
    }

    #[test]
    fn signature_help_finds_the_call_and_the_parameter() {
        let (_d, a) = analysis("fn add(a: i32, b: i32) -> i32 { return a + b }\nfn f() {\n    add(1, \n}\n");
        let s = a.signatures(0, Pos { line: 3, col: 12 }, &GameView::default()).unwrap();
        assert_eq!(s.list[s.active].params, vec!["a: i32".to_string(), "b: i32".to_string()]);
        assert_eq!(s.param, 1);
    }

    #[test]
    fn the_outline_names_each_item_and_its_members() {
        let (_d, a) = analysis("struct Ball { x: f32 }\nfn roll() {}\n");
        let outline = a.outline(0);
        let names: Vec<&str> = outline.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Ball", "roll"]);
        assert_eq!(outline[0].children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["x"]);
        // An item's name is where it is written; its range ends before the next item's.
        assert_eq!(outline[0].name_at, Pos { line: 1, col: 8 });
        assert!(outline[0].end <= outline[1].start);
    }

    #[test]
    fn the_name_length_counts_a_backquoted_game_name_with_its_quotes() {
        assert_eq!(name_len("x = `Mario::jump`()", Pos { line: 1, col: 5 }), 13);
        assert_eq!(name_len("let speed = 1", Pos { line: 1, col: 5 }), 5);
        assert_eq!(name_len("if MR::isDead(a)", Pos { line: 1, col: 4 }), 10);
        assert_eq!(name_len("Mario::~Mario()", Pos { line: 1, col: 1 }), 13);
        assert_eq!(name_len("x::", Pos { line: 1, col: 1 }), 1);
    }

    /// The members after a value's dot: the checker's type of the value, whether the member is written yet or not,
    /// in an argument or after `self`.
    #[test]
    fn completion_after_a_dot_gives_the_value_s_members() {
        let text = |line7: &str, line11: &str| format!("fn push(s: f32) -> f32 {{\n    return s\n}}\nstruct Ball {{\n    size: f32\n    fn grow(self) -> f32 {{\n{line7}\n    }}\n}}\nfn roll(b: Ball) {{\n{line11}\n}}\n");
        let members = |src: &str, at: Pos| -> Vec<String> {
            let (_d, a) = analysis(src);
            a.complete(0, at, &GameView::default()).items.into_iter().map(|c| c.label).collect()
        };
        let whole = text("        return self.size", "    let n = push(b.size)");
        let empty_arg = text("        return self.size", "    let n = push(b.)");
        let empty_self = text("        return self.", "    let n = push(b.size)");
        for (src, at) in [(&whole, Pos { line: 11, col: 22 }), (&empty_arg, Pos { line: 11, col: 20 }), (&whole, Pos { line: 7, col: 21 }), (&empty_self, Pos { line: 7, col: 21 })] {
            let got = members(src, at);
            assert!(got.contains(&"size".to_string()) && got.contains(&"grow".to_string()), "{at:?} {got:?}\n{src}");
            assert!(!got.contains(&"let".to_string()) && !got.contains(&"roll".to_string()), "{at:?} {got:?}");
        }
        // A value whose type the checker does not know: nothing is guessed.
        assert!(members("fn f() {\n    let n = nothing.\n}\n", Pos { line: 2, col: 21 }).is_empty());
    }
}
