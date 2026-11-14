//! The editor's questions about a mod already read ([`Analysis`]): what a name is (hover), where it is made (go to
//! definition), where it is used (references, rename), the call being written (signature help), a module's outline and
//! the names a search finds (workspace symbols). Every answer comes from the checker's reading, the scopes it made, the
//! needed mods' interfaces and the game's self-description; a question none of them answers gets no answer.

use super::*;
use lumascript::check::{ModRef, ModTarget, Name};

/// What a name at a place stands for: the checker's resolution, a module named by `use` or by a type's path, or a game
/// type written by name (a class's base, a type of the mod's code).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    R(Resolved),
    Module(ModRef),
    GameType(String),
}

/// What an editor shows over a name: its declaration as code, the documentation written above it (or after it on its
/// line), and for a game name its reference page and the count of its other signatures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hover {
    pub code: String,
    pub doc: String,
    pub page: Option<PathBuf>,
    pub others: usize,
}

/// One symbol of a module's outline: its range (the whole item), the range of its name, and its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    pub name: String,
    pub detail: String,
    pub kind: Kind,
    pub start: Pos,
    pub end: Pos,
    pub name_at: Pos,
    pub name_len: u32,
    pub children: Vec<OutlineItem>,
}

/// A name a workspace search found: what it is, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub kind: Kind,
    pub container: String,
    pub place: Place,
}

/// What to look for inside an item when going to it: the item itself, a member, an enum's case, a state.
#[derive(Clone, Copy, Debug)]
enum Inner<'a> {
    Whole,
    Member(&'a str),
    Case(&'a str),
    State(&'a str),
}

impl Analysis {
    pub fn module_of(&self, file: &Path) -> Option<usize> {
        self.modules.iter().position(|m| m.file == file)
    }

    /// The occurrence under `at` in module `m` (its first character to its last), or the definition there.
    pub fn at(&self, m: usize, at: Pos) -> Option<Target> {
        if let Some(o) = self.occurrence(m, at) {
            return Some(o.target.clone());
        }
        // A definition's own name: an item of this module, or a member, case or state of one.
        let hit = |n: &ast::Ident| n.at.line == at.line && n.at.col <= at.col && at.col < n.at.col + n.name.chars().count() as u32;
        for (i, item) in self.parsed.get(m)?.items.iter().enumerate() {
            let me = ItemRef { owner: Owner::Mod, module: m, item: i };
            if item_name(item).is_some_and(hit) {
                return Some(Target::R(Resolved::Item(me)));
            }
            match &item.kind {
                ItemKind::Struct(s) => {
                    if let Some(n) = s.fields.iter().map(|f| &f.name).chain(s.fns.iter().map(|f| &f.name)).find(|n| hit(n)) {
                        return Some(Target::R(Resolved::Member { owner: me, name: n.name.clone() }));
                    }
                }
                ItemKind::Enum(e) => {
                    if let Some(c) = e.cases.iter().find(|c| hit(&c.name)) {
                        return Some(Target::R(Resolved::Case { item: me, case: c.name.name.clone() }));
                    }
                }
                ItemKind::Class(c) => {
                    for member in &c.members {
                        let n = match member {
                            ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => Some(&f.name),
                            ast::ClassMember::Binding(b) => binding_name(b),
                            ast::ClassMember::Hook(_) => None,
                        };
                        if let Some(n) = n.filter(|n| hit(n)) {
                            return Some(Target::R(Resolved::Member { owner: me, name: n.name.clone() }));
                        }
                    }
                }
                ItemKind::States(s) => {
                    if let Some(st) = s.states.iter().find(|st| hit(&st.name)) {
                        return Some(Target::R(Resolved::State { machine: me, state: st.name.name.clone() }));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn occurrence(&self, m: usize, at: Pos) -> Option<&Occurrence> {
        self.occurrences.iter().find(|o| o.module == m && o.at.line == at.line && o.at.col <= at.col && at.col < o.at.col + o.len.max(1))
    }

    /// An item of the mod or of a needed library.
    pub fn item_ast(&self, r: ItemRef) -> Option<&Item> {
        match r.owner {
            Owner::Mod => self.parsed.get(r.module)?.items.get(r.item),
            Owner::Library(l) => self.libraries.get(l)?.modules.get(r.module)?.items.get(r.item),
        }
    }

    /// A module's text: the mod's file, or the library module's part of its interface file.
    fn module_text(&self, r: ModRef) -> Option<&str> {
        match r.owner {
            Owner::Mod => self.modules.get(r.module).map(|m| m.text.as_str()),
            Owner::Library(l) => self.libraries.get(l)?.parts.get(r.module).map(|p| p.text.as_str()),
        }
    }

    /// A module as `use` names it: `ball/fly` for the mod's, `lumakit.actor` for a library's.
    pub fn module_label(&self, r: ModRef) -> String {
        match r.owner {
            Owner::Mod => self.modules.get(r.module).map(|m| m.path.clone()).unwrap_or_default(),
            Owner::Library(l) => match self.libraries.get(l) {
                Some(lib) => format!("{}.{}", lib.id, lib.parts.get(r.module).map(|p| p.path.as_str()).unwrap_or("")),
                None => String::new(),
            },
        }
    }

    /// The place of a name written at `at` in module `m` of the mod.
    fn mod_place(&self, m: usize, at: Pos) -> Option<Place> {
        let module = self.modules.get(m)?;
        let len = name_len(&module.text, at);
        Some(Place { file: module.file.clone(), at, end: Pos { line: at.line, col: at.col + len } })
    }

    /// Where an item (or a part of it) is made. A library's: in its source beside the mod when that is there, else
    /// in its interface file.
    fn item_place(&self, r: ItemRef, inner: Inner) -> Option<Place> {
        match r.owner {
            Owner::Mod => {
                let it = self.item_ast(r)?;
                self.mod_place(r.module, inner_at(it, inner)?)
            }
            Owner::Library(l) => {
                let lib = self.libraries.get(l)?;
                let it = self.item_ast(r)?;
                let part = lib.parts.get(r.module)?;
                if let Some(source) = &lib.source {
                    let file = source.join(format!("{}.lum", part.path));
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let name = item_name(it).map(|n| n.name.clone());
                        let found = lumascript::on_compiler_stack(|| {
                            let (parsed, _) = lumascript::parse_module(&text);
                            parsed.items.iter().find(|i| item_name(i).map(|n| n.name.clone()) == name && std::mem::discriminant(&i.kind) == std::mem::discriminant(&it.kind)).and_then(|i| inner_at(i, inner))
                        });
                        if let Ok(Some(at)) = found {
                            let len = name_len(&text, at);
                            return Some(Place { file, at, end: Pos { line: at.line, col: at.col + len } });
                        }
                    }
                }
                let at = inner_at(it, inner)?;
                let len = name_len(&part.text, at);
                let line = at.line + part.first_line - 1;
                Some(Place { file: lib.lumi.clone(), at: Pos { line, col: at.col }, end: Pos { line, col: at.col + len } })
            }
        }
    }

    /// Where a module starts: the mod's file, or the library's source file (else its part of the interface file).
    fn module_place(&self, r: ModRef) -> Option<Place> {
        let start = Pos { line: 1, col: 1 };
        match r.owner {
            Owner::Mod => Some(Place { file: self.modules.get(r.module)?.file.clone(), at: start, end: start }),
            Owner::Library(l) => {
                let lib = self.libraries.get(l)?;
                let part = lib.parts.get(r.module)?;
                if let Some(file) = lib.source.as_ref().map(|s| s.join(format!("{}.lum", part.path))).filter(|f| f.is_file()) {
                    return Some(Place { file, at: start, end: start });
                }
                let at = Pos { line: part.first_line.saturating_sub(1).max(1), col: 1 };
                Some(Place { file: lib.lumi.clone(), at, end: at })
            }
        }
    }

    /// Where `target` is made: a place in a module, in a needed mod, in an SDK header, or the line of the game's
    /// reference page that describes it.
    pub fn definition(&self, target: &Target, game: &GameView) -> Option<Place> {
        match target {
            Target::R(Resolved::Local(id)) => {
                let local = self.checked.locals.get(*id as usize)?;
                self.mod_place(local.module, local.at)
            }
            Target::R(Resolved::Item(item)) => self.item_place(*item, Inner::Whole),
            Target::R(Resolved::Member { owner, name }) => self.item_place(*owner, Inner::Member(name)),
            Target::R(Resolved::Case { item, case }) => self.item_place(*item, Inner::Case(case)),
            Target::R(Resolved::State { machine, state }) => self.item_place(*machine, Inner::State(state)),
            Target::R(Resolved::GameFn { name, .. }) => page_place(game, name, &format!("{name}(")),
            Target::R(Resolved::GameField { record, field }) => page_place(game, record, &format!("`{field}`")),
            Target::R(Resolved::GameGlobal(name) | Resolved::GameConstant { name, .. }) => page_place(game, name, name),
            Target::R(Resolved::GameEnumValue { enum_name, value, .. }) => page_place(game, enum_name, value),
            Target::GameType(name) => page_place(game, name, name),
            Target::R(Resolved::ServiceFn { service, name, .. }) => {
                let face = game.game.services.faces.get(service)?;
                let f = face.function(name)?;
                let file = game.sdk.as_ref()?.join(&face.header);
                let at = Pos { line: f.c.line.max(1), col: 1 };
                Some(Place { file, at, end: at })
            }
            Target::Module(r) => self.module_place(*r),
            _ => None,
        }
    }

    /// Every place `target` is written in the mod, its definition first when it is in the mod.
    pub fn references(&self, target: &Target, game: &GameView) -> Vec<Place> {
        let mut out = Vec::new();
        if let Some(def) = self.definition(target, game) {
            if self.modules.iter().any(|m| m.file == def.file) {
                out.push(def);
            }
        }
        for o in self.occurrences.iter().filter(|o| &o.target == target) {
            let place = Place { file: self.modules[o.module].file.clone(), at: o.at, end: Pos { line: o.at.line, col: o.at.col + o.len } };
            if !out.contains(&place) {
                out.push(place);
            }
        }
        out
    }

    /// The edits that rename the mod's name at `at` to `new`: every place it is written. Err: why not (a game's or
    /// a library's name, a name that is no name, a keyword).
    pub fn rename(&self, m: usize, at: Pos, new: &str, game: &GameView) -> Result<Vec<Place>, RenameRefused> {
        let valid = new.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && new.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid || lumascript::lex::KEYWORDS.contains(&new) {
            return Err(RenameRefused::NotAName);
        }
        let target = self.at(m, at).ok_or(RenameRefused::NoName)?;
        let own = match &target {
            Target::R(Resolved::Local(_)) => true,
            Target::R(Resolved::Item(i)) => i.owner == Owner::Mod,
            Target::R(Resolved::Member { owner, .. }) => owner.owner == Owner::Mod,
            Target::R(Resolved::Case { item, .. }) => item.owner == Owner::Mod,
            Target::R(Resolved::State { machine, .. }) => machine.owner == Owner::Mod,
            _ => false,
        };
        if !own {
            return Err(RenameRefused::NotOwn);
        }
        Ok(self.references(&target, game))
    }

    /// What an editor shows over the name at `at` in module `m`.
    pub fn hover(&self, m: usize, at: Pos, game: &GameView) -> Option<Hover> {
        let target = self.at(m, at)?;
        let ty = self.occurrence(m, at).and_then(|o| o.expr).and_then(|id| self.checked.exprs.get(&(m, id))).map(|t| self.ty_text(t, game)).filter(|t| !t.is_empty());
        let mut h = Hover::default();
        match &target {
            Target::R(Resolved::Local(id)) => {
                let l = self.checked.locals.get(*id as usize)?;
                h.code = format!("{} {}: {}", if l.var { "var" } else { "let" }, l.name, self.ty_text(&l.ty, game));
                return Some(h);
            }
            Target::R(Resolved::Item(item)) => {
                h.code = self.item_text(*item)?;
                h.doc = self.doc_of(*item, Inner::Whole);
                return Some(h);
            }
            Target::R(Resolved::Member { owner, name }) => {
                h.code = self.member_text(*owner, name).unwrap_or_else(|| format!("{}.{name}", self.item_name_of(*owner).unwrap_or_default()));
                h.doc = self.doc_of(*owner, Inner::Member(name));
                return Some(h);
            }
            Target::R(Resolved::Case { item, case }) => {
                h.code = format!("{}.{case}", self.item_name_of(*item).unwrap_or_default());
                h.doc = self.doc_of(*item, Inner::Case(case));
                return Some(h);
            }
            Target::R(Resolved::State { machine, state }) => {
                h.code = format!("state {state}");
                h.doc = self.doc_of(*machine, Inner::State(state));
                return Some(h);
            }
            Target::R(Resolved::GameFn { name, signature }) => {
                let facts = game.game.facts.as_ref().and_then(|f| f.rows.get(&(name.clone(), signature.clone())));
                let virt = if facts.is_some_and(|f| f.is_virtual) { "virtual " } else { "" };
                let konst = if facts.is_some_and(|f| f.konst) { " const" } else { "" };
                h.code = format!("{virt}{name}{signature}{konst}");
                if let Some(ty) = &ty {
                    h.code.push_str(&format!(" -> {ty}"));
                }
                h.page = game.page(name).cloned();
                h.others = game.game.functions.get(name).map_or(0, |s| s.len().saturating_sub(1));
                return Some(h);
            }
            Target::R(Resolved::GameField { record, field }) => {
                let row = game.game.types.as_ref().and_then(|t| t.records.get(record)).and_then(|r| r.fields.get(field)).cloned().unwrap_or_default();
                h.code = format!("{record}.{field}: {row}");
                h.page = game.page(record).cloned();
            }
            Target::R(Resolved::GameGlobal(name)) => {
                let row = game.game.types.as_ref().and_then(|t| t.globals.get(name)).cloned().unwrap_or_default();
                h.code = format!("{name}: {row}");
                h.page = game.page(name).cloned();
            }
            Target::R(Resolved::GameConstant { name, value }) => {
                h.code = format!("{name} = {value}");
                h.page = game.page(name).cloned();
            }
            Target::R(Resolved::GameEnumValue { enum_name, value, number }) => {
                h.code = format!("{enum_name}::{value} = {number}");
                h.page = game.page(enum_name).cloned();
                return Some(h);
            }
            Target::GameType(name) => {
                let record = game.game.types.as_ref().and_then(|t| t.records.get(name));
                let bases = record.map(|r| r.bases.iter().map(|b| b.name.clone()).collect::<Vec<_>>()).unwrap_or_default();
                h.code = if bases.is_empty() { name.clone() } else { format!("{name} : {}", bases.join(", ")) };
                h.page = game.page(name).cloned();
                return Some(h);
            }
            Target::Module(r) => {
                h.code = format!("use {}", self.module_label(*r));
                return Some(h);
            }
            Target::R(Resolved::ServiceFn { service, name, .. }) => {
                let f = game.game.services.faces.get(service).and_then(|f| f.function(name));
                h.code = format!("svc.{service}.{name}");
                if let Some(ty) = &ty {
                    h.code.push_str(&format!(" -> {ty}"));
                }
                h.doc = f.map(|f| f.comment.clone()).unwrap_or_default();
                return Some(h);
            }
            Target::R(Resolved::Std(name)) => {
                h.code = (*name).to_string();
                if let Some(ty) = &ty {
                    h.code.push_str(&format!(" -> {ty}"));
                }
                return Some(h);
            }
            Target::R(Resolved::Method(name)) => {
                h.code = format!(".{name}");
                if let Some(ty) = &ty {
                    h.code.push_str(&format!(" -> {ty}"));
                }
                return Some(h);
            }
            Target::R(Resolved::BaseCall) => {
                let word: String = self.modules[m].text.lines().nth(at.line as usize - 1).unwrap_or("").chars().skip(at.col as usize - 1).take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                h.code = format!("base.{word}");
                if let Some(ty) = &ty {
                    h.code.push_str(&format!(" -> {ty}"));
                }
                return Some(h);
            }
            Target::R(Resolved::Table { write, name, offset, .. }) => h.code = format!("{}({name}) +{offset}", if *write { "set_table" } else { "table" }),
            Target::R(Resolved::Limit { ty: s, max }) => h.code = format!("{}.{}", format!("{s:?}").to_lowercase(), if *max { "max" } else { "min" }),
            Target::R(Resolved::TupleIndex(i)) => h.code = format!(".{i}"),
            Target::R(Resolved::Component(i)) => h.code = format!(".{}", ["x", "y", "z", "w"].get(*i).copied().unwrap_or("?")),
            Target::R(Resolved::ServiceCase { service, ty: t, case, number }) => h.code = format!("svc.{service}.{t}.{case} = {number}"),
            Target::R(Resolved::ServiceConst { service, name, value }) => h.code = format!("svc.{service}.{name} = {value}"),
            Target::R(Resolved::ServiceField { service, record, field }) => h.code = format!("svc.{service}.{record}.{field}"),
            Target::R(Resolved::StreamCase { service, kind, number }) => h.code = format!("svc.{service}.Event.{kind} = {number}"),
            Target::R(Resolved::ServiceGiven(s)) => h.code = format!("use optional svc.{s}"),
        }
        if let Some(ty) = ty {
            h.code.push_str(&format!("\n: {ty}"));
        }
        Some(h)
    }

    fn item_name_of(&self, item: ItemRef) -> Option<String> {
        item_name(self.item_ast(item)?).map(|n| n.name.clone())
    }

    /// The documentation of an item or of a part of it: the comment lines right above it, else the comment after it
    /// on its line.
    fn doc_of(&self, item: ItemRef, inner: Inner) -> String {
        let (Some(it), Some(text)) = (self.item_ast(item), self.module_text(item.module_ref())) else { return String::new() };
        let Some(at) = inner_at(it, inner) else { return String::new() };
        // An item's comment is above its first line (`pub` or a keyword before its name).
        let line = if matches!(inner, Inner::Whole) { it.at.line } else { at.line };
        doc_at(text, line)
    }

    /// An item as one line: its kind, name and the types written.
    fn item_text(&self, item: ItemRef) -> Option<String> {
        let it = self.item_ast(item)?;
        let module = self.module_label(item.module_ref());
        let public = if it.public { "pub " } else { "" };
        Some(match &it.kind {
            ItemKind::Fn(f) => format!("{public}fn {}({}){}  // {module}", f.name.name, params_text(&f.params), ret_text(&f.ret)),
            ItemKind::Task { name, params: ps, .. } => format!("{public}task {}({})  // {module}", name.name, params_text(ps)),
            ItemKind::Extern { name, params: ps, ret: r } => format!("extern c fn {}({}){}", name.name, params_text(ps), ret_text(r)),
            ItemKind::Export { name, params: ps, ret: r, .. } => format!("export c fn {}({}){}", name.name, params_text(ps), ret_text(r)),
            ItemKind::Struct(s) => {
                let mut lines = vec![format!("{public}struct {} {{", s.name.name)];
                lines.extend(s.fields.iter().map(|f| format!("    {}: {}", f.name.name, check::type_text(&f.ty))));
                lines.extend(s.fns.iter().map(|f| format!("    fn {}({}){}", f.name.name, params_text(&f.params), ret_text(&f.ret))));
                lines.push(format!("}}  // {module}"));
                lines.join("\n")
            }
            ItemKind::Enum(e) => {
                let mut lines = vec![format!("{public}enum {} {{", e.name.name)];
                for c in &e.cases {
                    let fields = c.fields.iter().map(|f| format!("{}: {}", f.name.name, check::type_text(&f.ty))).collect::<Vec<_>>().join(", ");
                    lines.push(if c.fields.is_empty() { format!("    {}", c.name.name) } else if c.braces { format!("    {} {{ {fields} }}", c.name.name) } else { format!("    {}({fields})", c.name.name) });
                }
                lines.push(format!("}}  // {module}"));
                lines.join("\n")
            }
            ItemKind::Class(c) => format!("{public}class {} : {}  // {module}", c.name.name, c.base.text),
            ItemKind::Option { key, ty } => format!("option {}: {}", key.name, check::type_text(ty)),
            ItemKind::Binding(b) => {
                let name = item_name(it).map(|n| n.name.clone()).unwrap_or_default();
                let ty = match &b.ty {
                    Some(t) => format!(": {}", check::type_text(t)),
                    None => self.checked.types.get(&(item, String::new())).map(|t| format!(": {}", self.ty_text(t, &GameView::default()))).unwrap_or_default(),
                };
                format!("{public}{} {name}{ty}  // {module}", if b.var { "var" } else { "let" })
            }
            ItemKind::States(s) => format!("states {} for {}", s.name.name, s.class.name),
            ItemKind::Test { name, .. } => format!("test {}", str_text(name)),
            ItemKind::Hook(h) => format!("hook {}", h.target.text),
            ItemKind::On(_) => "on".to_string(),
            ItemKind::Use(_) => "use".to_string(),
        })
    }

    /// A struct's or class's member as a line: a field with its type, a function with its parameters.
    fn member_text(&self, owner: ItemRef, name: &str) -> Option<String> {
        let it = self.item_ast(owner)?;
        let owner_name = item_name(it)?.name.clone();
        let fn_text = |f: &ast::FnDecl| format!("fn {owner_name}.{}({}){}", f.name.name, params_text(&f.params), ret_text(&f.ret));
        match &it.kind {
            ItemKind::Struct(s) => {
                if let Some(f) = s.fields.iter().find(|f| f.name.name == name) {
                    return Some(format!("{owner_name}.{}: {}", f.name.name, check::type_text(&f.ty)));
                }
                s.fns.iter().find(|f| f.name.name == name).map(fn_text)
            }
            ItemKind::Class(c) => c.members.iter().find_map(|m| match m {
                ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) if f.name.name == name => Some(fn_text(f)),
                ast::ClassMember::Binding(b) if binding_name(b).is_some_and(|n| n.name == name) => {
                    Some(format!("{} {owner_name}.{name}{}", if b.var { "var" } else { "let" }, b.ty.as_ref().map(|t| format!(": {}", check::type_text(t))).unwrap_or_default()))
                }
                _ => None,
            }),
            _ => None,
        }
    }

    /// A checker type as a script writes it.
    pub fn ty_text(&self, t: &Ty, game: &GameView) -> String {
        let inner = |t: &Ty| self.ty_text(t, game);
        match t {
            Ty::Bool => "bool".into(),
            Ty::Int(s) | Ty::Float(s) => format!("{s:?}").to_lowercase(),
            Ty::Value(v) => format!("{v:?}").to_lowercase(),
            Ty::Text => "text".into(),
            Ty::Bytes => "bytes".into(),
            Ty::List(x) => format!("list<{}>", inner(x)),
            Ty::Map(k, v) => format!("map<{}, {}>", inner(k), inner(v)),
            Ty::Set(x) => format!("set<{}>", inner(x)),
            Ty::Opt(x) => format!("{}?", inner(x)),
            Ty::Tuple(xs) => format!("({})", xs.iter().map(inner).collect::<Vec<_>>().join(", ")),
            Ty::Fn(ps, r) => format!("fn({}) -> {}", ps.iter().map(inner).collect::<Vec<_>>().join(", "), inner(r)),
            Ty::Ref(x) => format!("ref<{}>", inner(x)),
            Ty::Pending(x) => format!("pending<{}>", inner(x)),
            Ty::Json => "json.Value".into(),
            Ty::Struct(i) | Ty::Enum(i) | Ty::Class(i) => self.item_name_of(*i).unwrap_or_else(|| "?".into()),
            Ty::Game(g) => game_ty_text(g),
            Ty::Service { service, name } => format!("svc.{service}.{name}"),
            Ty::Unit => "()".into(),
            Ty::Range(x) => format!("range<{}>", inner(x)),
            Ty::Task => "task".into(),
            Ty::Content(word) => word.clone(),
            Ty::Never => "never".into(),
            Ty::IntLit => "int".into(),
            Ty::FloatLit => "f32".into(),
            Ty::Open => "_".into(),
            Ty::Group { record, prefix, .. } => format!("{record}.{prefix}"),
            Ty::Error => String::new(),
        }
    }

    /// The module's outline: each item with its range and its parts (fields, functions, cases, states).
    pub fn outline(&self, m: usize) -> Vec<OutlineItem> {
        let Some(parsed) = self.parsed.get(m) else { return Vec::new() };
        let text = &self.modules[m].text;
        let lines: Vec<&str> = text.split('\n').collect();
        let last = Pos { line: lines.len() as u32, col: lines.last().map_or(1, |l| l.chars().count() as u32 + 1) };
        // An item reaches to the last line before the next item that is not blank or a comment above it.
        let ends: Vec<Pos> = (0..parsed.items.len())
            .map(|i| match parsed.items.get(i + 1) {
                None => last,
                Some(next) => {
                    let mut line = next.at.line.saturating_sub(1);
                    while line > parsed.items[i].at.line && lines.get(line as usize - 1).is_some_and(|l| l.trim().is_empty() || l.trim_start().starts_with("//")) {
                        line -= 1;
                    }
                    Pos { line, col: lines.get(line as usize - 1).map_or(1, |l| l.trim_end_matches('\r').chars().count() as u32 + 1) }
                }
            })
            .collect();
        let leaf = |name: &ast::Ident, kind: Kind, detail: String| OutlineItem {
            name: name.name.clone(),
            detail,
            kind,
            start: name.at,
            end: Pos { line: name.at.line, col: name.at.col + name.name.chars().count() as u32 },
            name_at: name.at,
            name_len: name.name.chars().count() as u32,
            children: Vec::new(),
        };
        let fn_leaf = |f: &ast::FnDecl| OutlineItem { end: f.body.end, ..leaf(&f.name, Kind::Function, format!("({}){}", params_text(&f.params), ret_text(&f.ret))) };
        let mut out = Vec::new();
        for (i, item) in parsed.items.iter().enumerate() {
            let (kind, detail, children): (Kind, String, Vec<OutlineItem>) = match &item.kind {
                ItemKind::Fn(f) => (Kind::Function, format!("({}){}", params_text(&f.params), ret_text(&f.ret)), Vec::new()),
                ItemKind::Extern { params, ret, .. } | ItemKind::Export { params, ret, .. } => (Kind::Function, format!("({}){}", params_text(params), ret_text(ret)), Vec::new()),
                ItemKind::Task { params, .. } => (Kind::Function, format!("task ({})", params_text(params)), Vec::new()),
                ItemKind::Struct(s) => {
                    let mut c: Vec<OutlineItem> = s.fields.iter().map(|f| leaf(&f.name, Kind::Field, check::type_text(&f.ty))).collect();
                    c.extend(s.fns.iter().map(fn_leaf));
                    (Kind::Struct, String::new(), c)
                }
                ItemKind::Enum(e) => (Kind::Enum, String::new(), e.cases.iter().map(|c| leaf(&c.name, Kind::Case, String::new())).collect()),
                ItemKind::Class(c) => {
                    let children = c
                        .members
                        .iter()
                        .filter_map(|m| match m {
                            ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => Some(fn_leaf(f)),
                            ast::ClassMember::Binding(b) => binding_name(b).map(|n| leaf(n, Kind::Field, b.ty.as_ref().map(check::type_text).unwrap_or_default())),
                            ast::ClassMember::Hook(h) => Some(OutlineItem {
                                name: format!("hook {}", h.target.text),
                                detail: String::new(),
                                kind: Kind::Event,
                                start: h.at,
                                end: h.body.end,
                                name_at: h.target.at,
                                name_len: name_len(text, h.target.at),
                                children: Vec::new(),
                            }),
                        })
                        .collect();
                    (Kind::Class, format!(": {}", c.base.text), children)
                }
                ItemKind::Option { ty, .. } => (Kind::Constant, check::type_text(ty), Vec::new()),
                ItemKind::Binding(b) => (if b.var { Kind::Variable } else { Kind::Constant }, b.ty.as_ref().map(check::type_text).unwrap_or_default(), Vec::new()),
                ItemKind::States(s) => (Kind::Class, format!("for {}", s.class.name), s.states.iter().map(|st| leaf(&st.name, Kind::Case, String::new())).collect()),
                ItemKind::Hook(_) | ItemKind::On(_) => (Kind::Event, String::new(), Vec::new()),
                ItemKind::Test { .. } => (Kind::Function, String::new(), Vec::new()),
                ItemKind::Use(_) => continue,
            };
            let (name, name_at) = match &item.kind {
                ItemKind::Hook(h) => (format!("hook {}", h.target.text), h.target.at),
                ItemKind::On(o) => (format!("on {}", on_text(&o.event)), item.at),
                ItemKind::Test { name, .. } => (format!("test {}", str_text(name)), item.at),
                _ => match item_name(item) {
                    Some(n) => (n.name.clone(), n.at),
                    None => continue,
                },
            };
            let len = name_len(text, name_at);
            let mut end = ends[i];
            // A child that reaches past the computed end (a function's body) widens it.
            for c in &children {
                if c.end > end {
                    end = c.end;
                }
            }
            out.push(OutlineItem { name, detail, kind, start: item.at, end, name_at, name_len: len, children });
        }
        out
    }

    /// The names of the mod's modules and of the game whose name holds `query` (any case), at most `limit`.
    pub fn search(&self, query: &str, game: &GameView, limit: usize) -> Vec<Found> {
        let q = query.to_lowercase();
        let mut out = Vec::new();
        // Each item and each member, by the module (and the item) it is in.
        fn walk(items: &[OutlineItem], file: &Path, container: &str, q: &str, out: &mut Vec<Found>) {
            for item in items {
                if item.name.to_lowercase().contains(q) {
                    let place = Place { file: file.to_path_buf(), at: item.name_at, end: Pos { line: item.name_at.line, col: item.name_at.col + item.name_len } };
                    out.push(Found { name: item.name.clone(), kind: item.kind, container: container.to_string(), place });
                }
                walk(&item.children, file, &format!("{container}.{}", item.name), q, out);
            }
        }
        for m in 0..self.modules.len() {
            walk(&self.outline(m), &self.modules[m].file, &self.modules[m].path, &q, &mut out);
        }
        if !q.is_empty() {
            let mut names: Vec<(&String, Kind)> = game.game.functions.keys().map(|n| (n, Kind::Function)).collect();
            if let Some(types) = &game.game.types {
                names.extend(types.records.keys().map(|n| (n, Kind::Class)));
                names.extend(types.enums.keys().map(|n| (n, Kind::Enum)));
                names.extend(types.globals.keys().map(|n| (n, Kind::Variable)));
            }
            names.retain(|(n, _)| n.to_lowercase().contains(&q));
            // The shortest first: a name that is the query, then the names that start with it.
            names.sort_by_key(|(n, _)| (!n.to_lowercase().ends_with(&q), n.len(), n.to_string()));
            for (name, kind) in names {
                if out.len() >= limit {
                    break;
                }
                let place = match kind {
                    Kind::Function => page_place(game, name, &format!("{name}(")),
                    _ => page_place(game, name, name),
                };
                if let Some(place) = place {
                    let container = name.rsplit_once("::").map(|(o, _)| o.to_string()).unwrap_or_default();
                    out.push(Found { name: name.clone(), kind, container, place });
                }
            }
        }
        out.truncate(limit);
        out
    }

    /// The occurrences the checker does not record: names in types, modules named by `use`, the names a `use`
    /// takes, game classes a class derives from, and a hook's game function.
    pub(super) fn named_places(module: &ast::Module, m: usize, text: &str, checked: &Checked, game: &Game) -> Vec<Occurrence> {
        let mut out = Vec::new();
        let scope = checked.scopes.get(m);
        let mut push = |at: Pos, target: Target| out.push(Occurrence { module: m, at, len: name_len(text, at), target, expr: None });
        let mut types: Vec<&ast::Type> = Vec::new();
        collect_types_module(module, &mut types);
        for t in types {
            type_places(t, m, scope, checked, game, &mut push);
        }
        for (i, item) in module.items.iter().enumerate() {
            let me = ItemRef { owner: Owner::Mod, module: m, item: i };
            match &item.kind {
                ItemKind::Use(ast::Use::Module { path, names, alias, .. }) => {
                    let key = alias.as_ref().or(path.last()).map(|n| n.name.clone());
                    if let (Some(last), Some(Name::Module { target: ModTarget::Module { owner, module }, .. })) = (path.last(), key.and_then(|k| scope.and_then(|s| s.names.get(&k)))) {
                        push(last.at, Target::Module(ModRef { owner: *owner, module: *module }));
                    }
                    for n in names {
                        if let Some(Name::Taken { item, .. }) = scope.and_then(|s| s.names.get(&n.name)) {
                            push(n.at, Target::R(Resolved::Item(*item)));
                        }
                    }
                }
                ItemKind::Class(c) => {
                    push(c.base.at, Target::GameType(c.base.text.clone()));
                    // The checker keys a class's hook by its place among the class's members.
                    for (n, member) in c.members.iter().enumerate() {
                        if let ast::ClassMember::Hook(h) = member {
                            if let Some(t) = checked.hooks.get(&(me, format!(".hook{n}"))) {
                                push(h.target.at, Target::R(Resolved::GameFn { name: t.name.clone(), signature: t.signature.clone() }));
                            }
                        }
                    }
                }
                ItemKind::Hook(h) => {
                    if let Some(t) = checked.hooks.get(&(me, String::new())) {
                        push(h.target.at, Target::R(Resolved::GameFn { name: t.name.clone(), signature: t.signature.clone() }));
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Why a rename is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameRefused {
    NotAName,
    NoName,
    NotOwn,
}

/// The names a type writes: `a.B` (a module's item), `B` (the module's own or one it took), a game record or enum.
fn type_places(t: &ast::Type, m: usize, scope: Option<&check::Scope>, checked: &Checked, game: &Game, push: &mut impl FnMut(Pos, Target)) {
    match &t.kind {
        ast::TypeKind::Tuple(xs) | ast::TypeKind::Ref(xs) => xs.iter().for_each(|x| type_places(x, m, scope, checked, game, push)),
        ast::TypeKind::Fn { params, ret } => {
            params.iter().for_each(|x| type_places(x, m, scope, checked, game, push));
            if let Some(r) = ret {
                type_places(r, m, scope, checked, game, push);
            }
        }
        ast::TypeKind::Named { parts, seps, quoted, args } => {
            args.iter().for_each(|x| type_places(x, m, scope, checked, game, push));
            let is_game = |name: &str| game.types.as_ref().is_some_and(|t| t.records.contains_key(name) || t.enums.contains_key(name) || t.aliases.contains_key(name));
            let joined = parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("::");
            if *quoted || seps.iter().any(|s| *s == ast::Sep::Colons) {
                if let Some(first) = parts.first() {
                    if is_game(&joined) {
                        push(first.at, Target::GameType(joined));
                    }
                }
                return;
            }
            let Some(first) = parts.first() else { return };
            match scope.and_then(|s| s.names.get(&first.name)) {
                Some(Name::Item { item, .. }) if parts.len() == 1 => push(first.at, Target::R(Resolved::Item(ItemRef { owner: Owner::Mod, module: m, item: *item }))),
                Some(Name::Taken { item, .. }) if parts.len() == 1 => push(first.at, Target::R(Resolved::Item(*item))),
                Some(Name::Module { target: ModTarget::Module { owner, module }, .. }) => {
                    let r = ModRef { owner: *owner, module: *module };
                    push(first.at, Target::Module(r));
                    let their = match owner {
                        Owner::Mod => checked.scopes.get(*module),
                        Owner::Library(l) => checked.library_scopes.get(*l).and_then(|s| s.get(*module)),
                    };
                    if let (Some(second), Some(Name::Item { item, .. })) = (parts.get(1), parts.get(1).and_then(|p| their.and_then(|s| s.names.get(&p.name)))) {
                        push(second.at, Target::R(Resolved::Item(r.item(*item))));
                    }
                }
                None if parts.len() == 1 && is_game(&first.name) => push(first.at, Target::GameType(first.name.clone())),
                _ => {}
            }
        }
    }
}

fn collect_types_module<'a>(module: &'a ast::Module, out: &mut Vec<&'a ast::Type>) {
    fn params<'a>(ps: &'a [ast::Param], out: &mut Vec<&'a ast::Type>) {
        out.extend(ps.iter().filter_map(|p| p.ty.as_ref()));
    }
    fn fn_decl<'a>(f: &'a ast::FnDecl, out: &mut Vec<&'a ast::Type>) {
        params(&f.params, out);
        out.extend(f.ret.as_ref());
        block(&f.body, out);
    }
    fn block<'a>(b: &'a ast::Block, out: &mut Vec<&'a ast::Type>) {
        for s in &b.stmts {
            stmt(s, out);
        }
    }
    fn stmt<'a>(s: &'a ast::Stmt, out: &mut Vec<&'a ast::Type>) {
        use ast::StmtKind as S;
        match &s.kind {
            S::Binding(b) => out.extend(b.ty.as_ref()),
            S::Fn(f) => fn_decl(f, out),
            S::Guard { otherwise, .. } => block(otherwise, out),
            S::While { body, .. } | S::For { body, .. } => block(body, out),
            S::Defer(st) => stmt(st, out),
            _ => {}
        }
    }
    for item in &module.items {
        match &item.kind {
            ItemKind::Fn(f) => fn_decl(f, out),
            ItemKind::Task { params: ps, body, .. } | ItemKind::Export { params: ps, body, .. } => {
                params(ps, out);
                block(body, out);
            }
            ItemKind::Extern { params: ps, ret, .. } => {
                params(ps, out);
                out.extend(ret.as_ref());
            }
            ItemKind::Struct(s) => {
                out.extend(s.fields.iter().map(|f| &f.ty));
                s.fns.iter().for_each(|f| fn_decl(f, out));
            }
            ItemKind::Enum(e) => e.cases.iter().for_each(|c| out.extend(c.fields.iter().map(|f| &f.ty))),
            ItemKind::Class(c) => {
                for m in &c.members {
                    match m {
                        ast::ClassMember::Binding(b) => out.extend(b.ty.as_ref()),
                        ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => fn_decl(f, out),
                        ast::ClassMember::Hook(h) => {
                            if let Some(ps) = &h.params {
                                params(ps, out);
                            }
                            block(&h.body, out);
                        }
                    }
                }
            }
            ItemKind::Option { ty, .. } => out.push(ty),
            ItemKind::Binding(b) => out.extend(b.ty.as_ref()),
            ItemKind::Hook(h) => {
                if let Some(ps) = &h.params {
                    params(ps, out);
                }
                block(&h.body, out);
            }
            ItemKind::On(o) => block(&o.body, out),
            ItemKind::Test { body, .. } => block(body, out),
            ItemKind::States(s) => {
                for st in &s.states {
                    for part in &st.parts {
                        match part {
                            ast::StatePart::Enter(b) | ast::StatePart::Update(b) | ast::StatePart::Exit(b) => block(b, out),
                            ast::StatePart::Stmt(s) => stmt(s, out),
                        }
                    }
                }
            }
            ItemKind::Use(_) => {}
        }
    }
}

/// The place of a part of an item: its name, a member's, a case's, a state's.
fn inner_at(item: &Item, inner: Inner) -> Option<Pos> {
    match inner {
        Inner::Whole => Some(item_name(item).map_or(item.at, |n| n.at)),
        Inner::Member(name) => member_at(item, name),
        Inner::Case(case) => match &item.kind {
            ItemKind::Enum(e) => e.cases.iter().find(|c| c.name.name == case).map(|c| c.name.at),
            _ => None,
        },
        Inner::State(state) => match &item.kind {
            ItemKind::States(s) => s.states.iter().find(|x| x.name.name == state).map(|x| x.name.at),
            _ => None,
        },
    }
}

/// The comment lines right above line `line` (from 1) of `text`, without their `//`; else the comment that ends that
/// line.
pub fn doc_at(text: &str, line: u32) -> String {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut above = Vec::new();
    let mut i = line as usize;
    while i >= 2 {
        let l = lines.get(i - 2).map(|l| l.trim_start()).unwrap_or("");
        let Some(rest) = l.strip_prefix("//") else { break };
        above.push(rest.trim_start_matches('/').strip_prefix(' ').unwrap_or(rest.trim_start_matches('/')).to_string());
        i -= 1;
    }
    if !above.is_empty() {
        above.reverse();
        return above.join("\n");
    }
    lines.get(line as usize - 1).and_then(|l| trailing_comment(l)).unwrap_or_default()
}

/// The `//` comment that ends a line of code, outside strings and backquotes.
fn trailing_comment(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let (mut quote, mut tick, mut i) = (false, false, 0);
    while i < chars.len() {
        match chars[i] {
            '\\' if quote => i += 1,
            '"' if !tick => quote = !quote,
            '`' if !quote => tick = !tick,
            '/' if !quote && !tick && chars.get(i + 1) == Some(&'/') => {
                let code_before = chars[..i].iter().any(|c| !c.is_whitespace());
                return code_before.then(|| chars[i + 2..].iter().collect::<String>().trim().to_string()).filter(|s| !s.is_empty());
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The line of a game name's reference page that names it: the first line holding `needle`, else the page's first.
fn page_place(game: &GameView, name: &str, needle: &str) -> Option<Place> {
    let file = game.page(name)?.clone();
    let text = std::fs::read_to_string(&file).ok()?;
    let (line, col) = text.lines().enumerate().find_map(|(i, l)| l.find(needle).map(|c| (i, l[..c].chars().count()))).unwrap_or((0, 0));
    let at = Pos { line: line as u32 + 1, col: col as u32 + 1 };
    let end = Pos { line: at.line, col: at.col + needle.chars().count() as u32 };
    Some(Place { file, at, end })
}

pub(super) fn params_text(ps: &[ast::Param]) -> String {
    ps.iter()
        .map(|p| {
            let name = if p.is_self { "self".to_string() } else { p.name.as_ref().map(|n| n.name.clone()).unwrap_or_default() };
            let mut s = match &p.ty {
                Some(t) => format!("{}{name}: {}", if p.var { "var " } else { "" }, check::type_text(t)),
                None => name,
            };
            if p.default.is_some() {
                s.push_str(" = ..");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn ret_text(r: &Option<ast::Type>) -> String {
    r.as_ref().map(|t| format!(" -> {}", check::type_text(t))).unwrap_or_default()
}

pub(super) fn game_ty_text(g: &GameTy) -> String {
    match g {
        GameTy::Unit => "()".into(),
        GameTy::Scalar(s) => format!("{s:?}").to_lowercase(),
        GameTy::Value(v) => format!("{v:?}").to_lowercase(),
        GameTy::Enum(e) => e.clone(),
        GameTy::Text => "text".into(),
        GameTy::Struct(r) => r.clone(),
        GameTy::View { record, konst } => format!("{}{record}", if *konst { "const " } else { "" }),
        GameTy::Place { of, konst } => format!("{}place<{}>", if *konst { "const " } else { "" }, game_ty_text(of)),
        GameTy::Opt(x) => format!("{}?", game_ty_text(x)),
        GameTy::Bits { of, width } => format!("{}:{width}", format!("{of:?}").to_lowercase()),
        GameTy::Array { of, extents } => format!("{}{}", game_ty_text(of), extents.iter().map(|e| format!("[{e}]")).collect::<String>()),
        GameTy::List { of, .. } => format!("list<{}>", game_ty_text(of)),
        GameTy::Null => "none".into(),
    }
}

fn on_text(e: &ast::OnEvent) -> String {
    let full = format!("{e:?}");
    let word: String = full.chars().take_while(|c| c.is_alphanumeric()).collect();
    word.to_lowercase()
}

pub(super) fn str_text(s: &ast::StrLit) -> String {
    let inner: String = s.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "{..}" }).collect();
    format!("\"{inner}\"")
}

/// A class field's or a module binding's single name.
pub(super) fn binding_name(b: &ast::Binding) -> Option<&ast::Ident> {
    match &b.pattern.kind {
        ast::PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => path.first(),
        _ => None,
    }
}

/// An item's own name, when it has one.
pub fn item_name(item: &Item) -> Option<&ast::Ident> {
    match &item.kind {
        ItemKind::Option { key, .. } => Some(key),
        ItemKind::Binding(b) => binding_name(b),
        ItemKind::Fn(f) => Some(&f.name),
        ItemKind::Struct(s) => Some(&s.name),
        ItemKind::Enum(e) => Some(&e.name),
        ItemKind::Class(c) => Some(&c.name),
        ItemKind::Task { name, .. } | ItemKind::Extern { name, .. } | ItemKind::Export { name, .. } => Some(name),
        ItemKind::States(s) => Some(&s.name),
        ItemKind::Use(_) | ItemKind::Hook(_) | ItemKind::On(_) | ItemKind::Test { .. } => None,
    }
}

/// Where a struct's or class's member `name` is named.
fn member_at(item: &Item, name: &str) -> Option<Pos> {
    match &item.kind {
        ItemKind::Struct(s) => s.fields.iter().map(|f| &f.name).chain(s.fns.iter().map(|f| &f.name)).find(|n| n.name == name).map(|n| n.at),
        ItemKind::Class(c) => c.members.iter().find_map(|m| match m {
            ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => (f.name.name == name).then_some(f.name.at),
            ast::ClassMember::Binding(b) => binding_name(b).filter(|n| n.name == name).map(|n| n.at),
            ast::ClassMember::Hook(_) => None,
        }),
        _ => None,
    }
}
