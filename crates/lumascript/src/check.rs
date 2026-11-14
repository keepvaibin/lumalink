//! The checker. Part (b), here: each module's items,
//! its `use`s, and its declarations (every written type resolved; structs, enums, classes, options, states). Part
//! (c), `body`: every expression typed and every name resolved. Every error names its place; one mistake is said
//! once (a part that does not check is `Ty::Error`, which nothing says again).

mod body;

pub use body::{Local, LocalId, Resolved};
pub(crate) use body::service_sty;

use crate::ast::{self, ClassMember, ExprId, ItemKind, Pos, Sep, TypeKind};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::{Game, GameTy, NotCrossing, Scalar, Value};
use std::collections::{BTreeMap, HashMap};

/// One module's source, by its path in `source/` without `.lum` (`main`, `ball/fly`).
#[derive(Clone, Debug)]
pub struct Source {
    pub path: String,
    pub module: ast::Module,
}

/// The standard module's path: no file of a mod gives it (`<` is no part of a module's path).
pub const STD_PATH: &str = "<std>";

/// The standard module: the types every module has without `use` that are written as LumaScript. Its public names
/// come after a module's own (locals, items, members), as the other std names do. A build puts it after the mod's
/// modules ([`std_source`]); without it these names are unknown.
pub const STD_TEXT: &str = "\
// Where `make` puts each object, as a stage's placement row gives it (docs/modding/language/classes.md, make): `at` in the main stage,
// `rotation` in degrees, `scale`, `args` the Obj_arg values from 0 (at most 8; the rest are -1), `dead` made dead
// until something makes it appear (actors only).
pub struct MakePlace {
    at: vec3
    rotation: vec3 = vec3(0.0, 0.0, 0.0)
    scale: vec3 = vec3(1.0, 1.0, 1.0)
    args: list<i32> = []
    dead: bool = false
}

// Where a stage placed an object (docs/modding/language/classes.md, placement; placement_of, placed_at): the main stage, the zone,
// the layer (\"Common\", \"LayerA\" to \"LayerAE\") and its index among that zone's placements of that layer. For placed_at
// an empty `stage` is the main stage.
pub struct Placement {
    stage: text
    zone: text
    layer: text
    index: u32
}

// A gravity make_gravity makes (docs/modding/language/classes.md, gravity): `range` below 0 everywhere, else what
// PlanetGravity::isInRangeDistance compares with; only the highest `priority` pulling at a point counts; `types` the
// mGravityType bits (1 normal, 2 shadow, 4 magnet, 8 launcher); `inverse` the pull reversed.
pub struct GravityPlace {
    range: f32
    priority: i32 = 0
    types: u32 = 1
    inverse: bool = false
}
";

/// The standard names every module has without `use` (language.md 16a), besides the type names: `min`, `vec3`, `json`...
pub fn std_names() -> &'static [&'static str] {
    body::STD_NAMES
}

/// A builtin method of a value (language.md 6.2, 11.1) as the checker reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub name: &'static str,
    /// Each parameter's name and type; a method taking a closure lists none (the call decides its types).
    pub params: Vec<(String, Ty)>,
    /// Its result; `Ty::Error` where the closure decides it (`map`, `sum`).
    pub ret: Ty,
    /// It changes the value it is called on (so the value must be a `var`).
    pub mutates: bool,
    pub closure: bool,
}

/// The builtin methods a value of type `t` has, in one fixed order.
pub fn methods_of(t: &Ty) -> Vec<Method> {
    body::builtins_of(t).into_iter().map(|(name, params, ret, mutates, closure)| Method { name, params, ret, mutates, closure }).collect()
}

/// The standard module as a source, to go after the mod's modules.
pub fn std_source() -> Source {
    Source { path: STD_PATH.to_string(), module: crate::parse_module(STD_TEXT).0 }
}

/// A library mod the mod `use`s, by source (docs/modding/language/compiled-form.md): its id and its modules.
#[derive(Clone, Debug)]
pub struct Library {
    pub id: String,
    pub modules: Vec<Source>,
}

/// An option of `luma.json` (luma-format `options`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionSpec {
    pub key: String,
    pub kind: OptionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionKind {
    Toggle,
    /// `whole`: the default and the min, max and step present are all whole numbers.
    Number { whole: bool },
    Text,
    Choice,
    Key,
}

/// What the checker reads besides the modules: the mod's `luma.json` facts and the game.
#[derive(Clone, Copy, Debug)]
pub struct ModFacts<'a> {
    pub needs: &'a [String],
    pub options: &'a [OptionSpec],
    /// The mod's C headers, as `use c` names them (paths from `source/`).
    pub c_headers: &'a [String],
    pub game: &'a Game,
    /// The content a content literal may name (docs/modding/language/language.md, section 10); None outside a mod's build (a check of loose modules),
    /// and then content literals are not checked.
    pub content: Option<&'a ContentNames>,
}

impl<'a> ModFacts<'a> {
    /// Facts with no needs, options, C headers or content: a module checked against the game alone.
    pub fn of_game(game: &'a Game) -> ModFacts<'a> {
        ModFacts { needs: &[], options: &[], c_headers: &[], game, content: None }
    }
}

/// The names content literals are checked against when a mod is built (docs/modding/language/language.md, section 10, docs/modding/language/references.md, section 2), as LumaLink reads
/// them from the mod's files, the mods it needs, and the game's: by word (`model`, `effect`, `sound`, `layout`, `font`,
/// `movie`, `data`, `nerve`), every name that word finds. A word LumaLink gives no set for is not checked; LumaLink
/// gives every word when it builds a mod.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentNames {
    pub names: BTreeMap<String, std::collections::BTreeSet<String>>,
    /// `text "Key"`: each text key the mod or the game has, with the placeholders its rows name in any language
    /// (each must be given, by name, and no other). None: the build has not the game's message tables, so
    /// text keys and their placeholders are not checked.
    pub texts: Option<BTreeMap<String, std::collections::BTreeSet<String>>>,
    /// The words whose bare names (no `<id>:`, no folder) may be the game's own, which the build has no list of
    /// (`model "GhostMario"` is ObjectData/GhostMario.arc): such a name not in the set is not refused here; `lumalink
    /// check --game` and the loader check it.
    pub game_unlisted: std::collections::BTreeSet<String>,
}

/// LumaKit's mod id; `kit.` is its short form (lumakit.md 1).
pub const LUMAKIT: &str = "lumakit";

/// Which modules: the mod's own, or a library's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    Mod,
    Library(usize),
}

/// An item of a module: whose, which module, which item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemRef {
    pub owner: Owner,
    pub module: usize,
    pub item: usize,
}

/// A module: the mod's or a library's, by its index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModRef {
    pub owner: Owner,
    pub module: usize,
}

impl ModRef {
    pub fn of_mod(module: usize) -> ModRef {
        ModRef { owner: Owner::Mod, module }
    }

    pub fn item(self, item: usize) -> ItemRef {
        ItemRef { owner: self.owner, module: self.module, item }
    }
}

impl ItemRef {
    pub fn module_ref(self) -> ModRef {
        ModRef { owner: self.owner, module: self.module }
    }
}

/// What a module-level name is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Name {
    /// An item of this module.
    Item { item: usize, sort: Sort, public: bool, at: Pos },
    /// A module brought in by `use`; `optional` for `use optional svc.<name>` (the service may not be given: its
    /// alias is `none` then).
    Module { target: ModTarget, at: Pos, optional: bool },
    /// A name taken from a module (`use x.{a}`), the item it names.
    Taken { item: ItemRef, at: Pos },
}

/// Which kind of item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Option,
    Let,
    Var,
    Fn,
    Struct,
    Enum,
    Class,
    Task,
    States,
    Extern,
    Export,
}

/// What a `use` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModTarget {
    Module { owner: Owner, module: usize },
    /// `use svc.<name>`: its LumaScript face is not in the self-description yet.
    Service(String),
}

/// The checker's types (design section 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Bool,
    Int(Scalar),
    Float(Scalar),
    Value(Value),
    Text,
    Bytes,
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Set(Box<Ty>),
    Opt(Box<Ty>),
    Tuple(Vec<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    /// `ref<C>`, C a game class (`Ty::Game(View)`) or the mod's class.
    Ref(Box<Ty>),
    Pending(Box<Ty>),
    /// `json.Value`.
    Json,
    /// A struct, enum or class of the mod or a library.
    Struct(ItemRef),
    Enum(ItemRef),
    Class(ItemRef),
    /// A game type written by name: a class's view, a game struct by value, a game enum.
    Game(GameTy),
    /// A type of a service (`net.Socket` after `use svc.net`): known by its name until the service's face is
    /// described; equal only to itself.
    Service { service: String, name: String },
    Unit,
    /// `a..b`, `a..=b` over an integer type (for loops).
    Range(Box<Ty>),
    /// A task's handle (`start t(...)`): `wait`ed, `.stop()`ped.
    Task,
    /// A content literal of the mod (`model "x"`, `sound "y"`) by its word; part (f) checks the name.
    Content(String),
    /// What `return`, `stop`, `break` and `continue` give: fits every place.
    Never,
    /// While choosing an overload only (never in [`Checked`]): an integer literal, a float literal, and a part
    /// whose type its place decides (`[]`, `{}`, `none`).
    IntLit,
    FloatLit,
    Open,
    /// A member of an anonymous type in a game record (`star.mFlags`): its fields are the record's rows
    /// `prefix.name` (`StarPiece.mFlags.isGettable`), reached through the view it was read from.
    Group { record: String, prefix: String, konst: bool },
    /// A part that did not check; accepted everywhere, said nowhere again.
    Error,
}

impl Ty {
    pub(crate) fn from_game(g: GameTy) -> Ty {
        match g {
            GameTy::Scalar(Scalar::Bool) => Ty::Bool,
            GameTy::Scalar(s @ (Scalar::F32 | Scalar::F64)) => Ty::Float(s),
            GameTy::Scalar(s) => Ty::Int(s),
            GameTy::Value(v) => Ty::Value(v),
            other => Ty::Game(other),
        }
    }
}

/// The service whose name the shared header's structs take (`LumaPlace` is `content.Place`).
pub const SHARED_SERVICE: &str = "content";

/// A type a service's face names: its struct, enum or handle kind, `Request` when a function
/// takes or gives the bare handle, an event record `<Kind>Event`, `Answer` (`svc.Answer`), or a shared struct
/// (`content.Place`, written through any service whose header includes it).
pub fn service_type(services: &crate::svc::Services, service: &str, name: &str) -> Option<Ty> {
    use crate::svc::{Gives, SParam, STy, Shape};
    let face = services.faces.get(service)?;
    let own = || Some(Ty::Service { service: service.to_string(), name: name.to_string() });
    if face.structs.contains_key(name) || face.enums.contains_key(name) || face.handles.iter().any(|h| h == name) {
        return own();
    }
    if face.events.keys().any(|k| body::event_name(k) == name) {
        return own();
    }
    // The service's stream events, one enum.
    if name == "Event" && !face.stream.is_empty() {
        return own();
    }
    if name == "Request" {
        let bare = STy::Handle("Request".into());
        let uses = face.functions.iter().any(|f| match &f.shape {
            Shape::Callable { params, gives } => {
                params.iter().any(|p| matches!(p, SParam::Value { ty, .. } if *ty == bare)) || matches!(gives, Gives::Request { handle, .. } if *handle == bare)
            }
            _ => false,
        });
        return if uses { own() } else { None };
    }
    if name == "Answer" {
        return Some(body::answer_ty());
    }
    if face.shared_structs.contains_key(&format!("Luma{name}")) {
        return Some(Ty::Service { service: SHARED_SERVICE.to_string(), name: name.to_string() });
    }
    None
}

/// One module's scope after part (b).
#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub names: BTreeMap<String, Name>,
}

/// What part (b) gives: each module's scope (the mod's, then each library's) and the diagnostics by module path.
#[derive(Clone, Debug, Default)]
pub struct Checked {
    pub scopes: Vec<Scope>,
    pub library_scopes: Vec<Vec<Scope>>,
    /// The declared types of each item's parts, by item: a struct's fields, an option's type... (read by part c).
    pub types: HashMap<(ItemRef, String), Ty>,
    /// Part (c): the type of each expression of the mod's modules, by module and expression id (4.3 reads it).
    pub exprs: HashMap<(usize, ExprId), Ty>,
    /// Part (c): what each name, member and call of the mod's modules stands for, by module and expression id.
    pub resolved: HashMap<(usize, ExprId), body::Resolved>,
    /// Part (c): every local of the mod's modules ([`body::Resolved::Local`] indexes it).
    pub locals: Vec<body::Local>,
    /// Part (c): what a name with no expression id stands for, by module and place: each binding's local (a
    /// parameter, a `let`, a pattern's name, a loop's), a case written in a pattern, a struct literal's field given by
    /// its name alone.
    pub named: HashMap<(usize, Pos), body::Resolved>,
    /// Part (e): each hook's game function, by the hook's item and key ("" at the top level, `.hook<n>` in a class).
    pub hooks: HashMap<(ItemRef, String), body::HookTarget>,
    /// Part (e): the locals of a task, a state machine or a handler that may wait which are read after a `wait` they
    /// were bound before: 4.5 keeps them in the task's struct (11.1).
    pub across_wait: std::collections::BTreeSet<body::LocalId>,
    /// Each option of `luma.json` by its key: its place in `options` (0 the first), the value of its OPTION event.
    pub option_index: HashMap<String, u32>,
    /// The mod's own models (`layers/*/assets`, no `<id>:` of a need), as the build's content names give them: a
    /// `model "x"` of one of these is the game's resource "Luma/<mod id>/x", where the mod's cooked archive is.
    pub own_models: std::collections::BTreeSet<String>,
    pub diags: Vec<(String, Diagnostic)>,
}

struct Checker<'a> {
    modules: &'a [Source],
    libraries: &'a [Library],
    facts: ModFacts<'a>,
    out: Checked,
    /// Names already given a note that they hide a game type (said once per mod).
    hiding_noted: std::collections::BTreeSet<String>,
    /// Part (c): the types of module-level names and class fields written without one, found from their values.
    inferred: HashMap<(ItemRef, String), Ty>,
    /// Those being found now (a value whose type depends on itself).
    inferring: std::collections::HashSet<(ItemRef, String)>,
    /// Game functions already given their note.
    noted: std::collections::BTreeSet<String>,
    /// Part (c): the locals of library modules, read only to type the mod's uses of them (a library's own are its
    /// build's); [`body::Resolved::Local`] in a library walk indexes these.
    lib_locals: Vec<body::Local>,
    /// Part (c): the game's enum values by every name a script may write for one (`Enum::Value`, and an unscoped
    /// enum's `Value` in the enum's own scope), made once.
    game_values: HashMap<String, body::GameValue>,
}

/// Checks a mod's modules (part b) against its libraries and facts.
pub fn check(modules: &[Source], libraries: &[Library], facts: ModFacts<'_>) -> Checked {
    let mut c = Checker { modules, libraries, facts, out: Checked::default(), hiding_noted: Default::default(), inferred: HashMap::new(), inferring: Default::default(), noted: Default::default(), lib_locals: Vec::new(), game_values: HashMap::new() };
    c.out.option_index = c.facts.options.iter().enumerate().map(|(i, o)| (o.key.clone(), i as u32)).collect();
    if let Some(content) = c.facts.content {
        if let Some(models) = content.names.get("model") {
            c.out.own_models = models.iter().filter(|m| !m.contains(':')).cloned().collect();
        }
    }
    // Pass 1: items. A library's items are read for its names only: it is checked when it is built.
    for (m, source) in modules.iter().enumerate() {
        let scope = c.items(&source.module, ModRef::of_mod(m));
        c.out.scopes.push(scope);
    }
    for (l, library) in libraries.iter().enumerate() {
        let scopes = library.modules.iter().enumerate().map(|(m, s)| c.items(&s.module, ModRef { owner: Owner::Library(l), module: m })).collect();
        c.out.library_scopes.push(scopes);
    }
    // Pass 2: uses, then cycles among the mod's modules.
    let mut edges: Vec<Vec<(usize, Pos)>> = vec![Vec::new(); modules.len()];
    for (m, source) in modules.iter().enumerate() {
        for item in &source.module.items {
            if let ItemKind::Use(u) = &item.kind {
                if let Some(local) = c.use_item(ModRef::of_mod(m), u) {
                    edges[m].push((local, item.at));
                }
            }
        }
    }
    // C5: a mod with any `pub` item is a library, whose modules may use each other in a cycle (each module's functions
    // are declared before any body; module values start in the order the modules are listed).
    let library = modules.iter().filter(|s| s.path != STD_PATH).any(|s| s.module.items.iter().any(|i| i.public));
    if !library {
        c.cycles(&edges);
    }
    // A library's uses and declarations are read for its names and types only (said when it is built).
    for (l, library) in libraries.iter().enumerate() {
        for (m, source) in library.modules.iter().enumerate() {
            for item in &source.module.items {
                if let ItemKind::Use(u) = &item.kind {
                    c.use_item(ModRef { owner: Owner::Library(l), module: m }, u);
                }
            }
        }
    }
    // Pass 3: declarations.
    for (m, source) in modules.iter().enumerate() {
        for (i, item) in source.module.items.iter().enumerate() {
            c.declaration(ModRef::of_mod(m), i, item);
        }
    }
    for (l, library) in libraries.iter().enumerate() {
        for (m, source) in library.modules.iter().enumerate() {
            for (i, item) in source.module.items.iter().enumerate() {
                c.declaration(ModRef { owner: Owner::Library(l), module: m }, i, item);
            }
        }
    }
    c.value_cycles();
    // Pass 4: bodies (part c).
    body::bodies(&mut c);
    c.out.diags.sort_by_key(|d| (module_order(modules, &d.0), d.1.at));
    c.out
}

fn module_order(modules: &[Source], path: &str) -> usize {
    modules.iter().position(|s| s.path == path).unwrap_or(usize::MAX)
}

/// The phrase that says why a game type does not cross.
pub fn why_phrase(why: &NotCrossing) -> &'static str {
    match why {
        NotCrossing::Unreadable(_) => "Script.Why.Unreadable",
        NotCrossing::NotKnown(_) => "Script.Why.NotKnown",
        NotCrossing::LeftOut { .. } => "Script.Why.LeftOut",
        NotCrossing::PlatformWidth(_) => "Script.Why.PlatformWidth",
        NotCrossing::CharSign => "Script.Why.CharSign",
        NotCrossing::Buffer => "Script.Why.Buffer",
        NotCrossing::VoidPointer => "Script.Why.VoidPointer",
        NotCrossing::VoidValue => "Script.Why.VoidValue",
        NotCrossing::Function => "Script.Why.Function",
        NotCrossing::MemberPointer => "Script.Why.MemberPointer",
        NotCrossing::PointerToPointer => "Script.Why.PointerToPointer",
        NotCrossing::Volatile => "Script.Why.Volatile",
        NotCrossing::PointerToValue => "Script.Why.PointerToValue",
        NotCrossing::Kept => "Script.Why.Kept",
        NotCrossing::MarkNotFit => "Script.Why.MarkNotFit",
        NotCrossing::MatrixRows => "Script.Why.MatrixRows",
        NotCrossing::ClassByValue(_) => "Script.Why.ClassByValue",
        NotCrossing::OwnsMemory(_) => "Script.Why.OwnsMemory",
        NotCrossing::CannotDrop(_) => "Script.Why.CannotDrop",
        NotCrossing::CopiesItself(_) => "Script.Why.CopiesItself",
        NotCrossing::CannotCopy(_) => "Script.Why.CannotCopy",
        NotCrossing::Variadic => "Script.Why.Variadic",
        NotCrossing::BitFieldType(_) => "Script.Why.BitFieldType",
        NotCrossing::Other(_) => "Script.Why.Other",
    }
}

/// The language's own type names and how many type arguments each takes.
const BUILTIN_TYPES: [(&str, usize); 29] = [
    ("bool", 0),
    ("i8", 0),
    ("i16", 0),
    ("i32", 0),
    ("i64", 0),
    ("u8", 0),
    ("u16", 0),
    ("u32", 0),
    ("u64", 0),
    ("int", 0),
    ("f32", 0),
    ("f64", 0),
    ("float", 0),
    ("vec2", 0),
    ("vec3", 0),
    ("vec4", 0),
    ("quat", 0),
    ("mat34", 0),
    ("color", 0),
    ("text", 0),
    ("bytes", 0),
    ("list", 1),
    ("set", 1),
    // Content values (C2): a name of the mod's content (`model "x"`), a text at run time.
    ("model", 0),
    ("effect", 0),
    ("sound", 0),
    ("layout", 0),
    ("font", 0),
    ("movie", 0),
];

/// The generic names past [`BUILTIN_TYPES`]: `map<K, V>` and `pending<T>`.
const GENERIC_TYPES: [(&str, usize); 2] = [("map", 2), ("pending", 1)];

fn arity(name: &str) -> Option<usize> {
    BUILTIN_TYPES.iter().chain(GENERIC_TYPES.iter()).find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// A type as written, for messages.
pub fn type_text(t: &ast::Type) -> String {
    let mut s = match &t.kind {
        TypeKind::Tuple(items) => format!("({})", items.iter().map(type_text).collect::<Vec<_>>().join(", ")),
        TypeKind::Fn { params, ret } => {
            let ps = params.iter().map(type_text).collect::<Vec<_>>().join(", ");
            match ret {
                Some(r) => format!("fn({ps}) -> {}", type_text(r)),
                None => format!("fn({ps})"),
            }
        }
        TypeKind::Ref(args) => format!("ref<{}>", args.iter().map(type_text).collect::<Vec<_>>().join(", ")),
        TypeKind::Named { parts, seps, quoted, args } => {
            let mut s = String::new();
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    s.push_str(if seps.get(i - 1) == Some(&Sep::Colons) { "::" } else { "." });
                }
                s.push_str(&part.name);
            }
            if *quoted {
                s = format!("`{s}`");
            }
            if !args.is_empty() {
                s.push_str(&format!("<{}>", args.iter().map(type_text).collect::<Vec<_>>().join(", ")));
            }
            s
        }
    };
    for _ in 0..t.optional {
        s.push('?');
    }
    s
}

/// The names nearest `name` among `known` (within a third of its length in edits, at least one and at most three), at
/// most three, closest first.
pub fn nearest<'k>(name: &str, known: impl Iterator<Item = &'k str>) -> Vec<&'k str> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    let mut found: Vec<(usize, &str)> = known.filter(|k| *k != name).filter_map(|k| Some((edits(name, k, limit)?, k))).collect();
    found.sort();
    found.dedup();
    found.into_iter().take(3).map(|(_, k)| k).collect()
}

/// Edit distance, ignoring ASCII case, or None past `limit`; two names that differ only in case are one apart
/// (`Vec3` for `vec3`).
fn edits(a: &str, b: &str, limit: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca.eq_ignore_ascii_case(cb) { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    let d = prev[b.len()];
    (d <= limit && (d > 0 || a != b)).then_some(d.max(1))
}

/// The names a module-level binding's pattern binds.
fn pattern_names(p: &ast::Pattern, out: &mut Vec<ast::Ident>) {
    match &p.kind {
        ast::PatternKind::Tuple(items) => items.iter().for_each(|i| pattern_names(i, out)),
        ast::PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => out.push(path[0].clone()),
        _ => {}
    }
}

impl Checker<'_> {
    /// A diagnostic in a module of the mod; a library's are not said here (it is checked when it is built).
    fn say(&mut self, m: ModRef, d: Diagnostic) {
        if m.owner == Owner::Mod {
            self.out.diags.push((self.modules[m.module].path.clone(), d));
        }
    }

    fn twice(&mut self, m: ModRef, name: &ast::Ident, first: Pos) {
        let d = Diagnostic::new(code::NAME_TWICE, name.at).with("name", Arg::Text(name.name.clone())).with("line", Arg::Text(first.line.to_string()));
        self.say(m, d);
    }

    /// A module's source.
    fn source(&self, m: ModRef) -> &Source {
        match m.owner {
            Owner::Mod => &self.modules[m.module],
            Owner::Library(l) => &self.libraries[l].modules[m.module],
        }
    }

    fn scope(&self, m: ModRef) -> &Scope {
        match m.owner {
            Owner::Mod => &self.out.scopes[m.module],
            Owner::Library(l) => &self.out.library_scopes[l][m.module],
        }
    }

    fn scope_mut(&mut self, m: ModRef) -> &mut Scope {
        match m.owner {
            Owner::Mod => &mut self.out.scopes[m.module],
            Owner::Library(l) => &mut self.out.library_scopes[l][m.module],
        }
    }

    /// Pass 1: the module's items by name.
    fn items(&mut self, module: &ast::Module, m: ModRef) -> Scope {
        let mut scope = Scope::default();
        for (i, item) in module.items.iter().enumerate() {
            let mut named: Vec<(ast::Ident, Sort)> = Vec::new();
            match &item.kind {
                ItemKind::Option { key, .. } => named.push((key.clone(), Sort::Option)),
                ItemKind::Binding(b) => {
                    let mut idents = Vec::new();
                    pattern_names(&b.pattern, &mut idents);
                    named.extend(idents.into_iter().map(|n| (n, if b.var { Sort::Var } else { Sort::Let })));
                }
                ItemKind::Fn(f) => named.push((f.name.clone(), Sort::Fn)),
                ItemKind::Struct(s) => named.push((s.name.clone(), Sort::Struct)),
                ItemKind::Enum(e) => named.push((e.name.clone(), Sort::Enum)),
                ItemKind::Class(c) => named.push((c.name.clone(), Sort::Class)),
                ItemKind::Task { name, .. } => named.push((name.clone(), Sort::Task)),
                ItemKind::States(s) => named.push((s.name.clone(), Sort::States)),
                ItemKind::Extern { name, .. } => named.push((name.clone(), Sort::Extern)),
                ItemKind::Export { name, .. } => named.push((name.clone(), Sort::Export)),
                ItemKind::Use(_) | ItemKind::Hook(_) | ItemKind::On(_) | ItemKind::Test { .. } => {}
            }
            for (ident, sort) in named {
                if let Some(first) = scope.names.get(&ident.name) {
                    let first = name_at(first);
                    self.twice(m, &ident, first);
                } else {
                    scope.names.insert(ident.name.clone(), Name::Item { item: i, sort, public: item.public, at: ident.at });
                }
            }
        }
        scope
    }

    /// The library a `use` path's first part names, if it names one (`kit` is LumaKit).
    fn library(&self, first: &str) -> Option<usize> {
        let id = if first == "kit" { LUMAKIT } else { first };
        self.libraries.iter().position(|l| l.id == id)
    }

    /// Pass 2: one `use`. Gives the module of the same owner it uses, for the cycle check. A library's own `./`
    /// modules are its own; its needs were checked when it was built.
    fn use_item(&mut self, m: ModRef, u: &ast::Use) -> Option<usize> {
        let (optional, local, path, names, alias) = match u {
            ast::Use::C(header) => {
                let written: String = header.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect();
                if !self.facts.c_headers.contains(&written) {
                    self.say(m, Diagnostic::new(code::NO_C_HEADER, header.at).with("found", Arg::Text(format!("\"{written}\""))));
                }
                return None;
            }
            ast::Use::Module { optional, local, path, names, alias } => (*optional, *local, path, names, alias),
        };
        let parts: Vec<&str> = path.iter().map(|p| p.name.as_str()).collect();
        let written = if local { format!("./{}", parts.join("/")) } else { parts.join(".") };
        let found = || Arg::Text(written.clone());
        let target = if local {
            let own: &[Source] = match m.owner {
                Owner::Mod => self.modules,
                Owner::Library(l) => &self.libraries[l].modules,
            };
            match own.iter().position(|s| s.path == parts.join("/")) {
                Some(t) => Some(ModTarget::Module { owner: m.owner, module: t }),
                None => {
                    self.say(m, Diagnostic::new(code::NO_MODULE, path[0].at).with("found", found()));
                    None
                }
            }
        } else if parts[0] == "svc" && parts.len() == 1 {
            self.say(m, Diagnostic::new(code::NO_MODULE, path[0].at).with("found", found()));
            None
        } else if parts[0] == "svc" {
            let service = parts[1..].join(".");
            let services = &self.facts.game.services;
            if services.faces.is_empty() {
                // No SDK headers were given: the service's parts are not checked (a note).
                let d = Diagnostic::new(code::NOT_DESCRIBED, path[0].at).with("found", found()).with("what", Arg::Phrase("Script.What.ServiceFaces"));
                self.say(m, d);
            } else if !services.faces.contains_key(&service) {
                self.say(m, Diagnostic::new(code::NO_MODULE, path[0].at).with("found", found()));
                return None;
            }
            Some(ModTarget::Service(service))
        } else {
            let id = if parts[0] == "kit" { LUMAKIT } else { parts[0] };
            if m.owner == Owner::Mod && !self.facts.needs.iter().any(|n| n == id) {
                let d = Diagnostic::new(code::NOT_NEEDED, path[0].at).with("found", found()).with("need", Arg::Text(id.to_string()));
                self.say(m, d);
                None
            } else {
                let module = parts[1..].join("/");
                match self.library(parts[0]).and_then(|l| Some((l, self.libraries[l].modules.iter().position(|s| s.path == module)?))) {
                    Some((l, t)) if parts.len() >= 2 => Some(ModTarget::Module { owner: Owner::Library(l), module: t }),
                    _ => {
                        self.say(m, Diagnostic::new(code::NO_MODULE, path[0].at).with("found", found()));
                        None
                    }
                }
            }
        };
        let target = target?;
        // The names it binds: the module (by its alias or last part), and each name taken from it.
        if names.is_empty() || alias.is_some() {
            let bound = alias.clone().unwrap_or_else(|| path[path.len() - 1].clone());
            let optional = optional && matches!(target, ModTarget::Service(_));
            self.bind(m, bound.clone(), Name::Module { target: target.clone(), at: bound.at, optional });
        }
        if let ModTarget::Module { owner, module } = &target {
            for name in names {
                let scope = self.scope(ModRef { owner: *owner, module: *module });
                let module_name = || Arg::Text(written.clone());
                match scope.names.get(&name.name) {
                    Some(Name::Item { item, public: true, .. }) => {
                        let item = ItemRef { owner: *owner, module: *module, item: *item };
                        self.bind(m, name.clone(), Name::Taken { item, at: name.at });
                    }
                    Some(Name::Item { .. }) => {
                        let d = Diagnostic::new(code::NAME_NOT_PUB, name.at).with("name", Arg::Text(name.name.clone())).with("module", module_name());
                        self.say(m, d);
                    }
                    _ => {
                        let d = Diagnostic::new(code::NO_NAME_IN_MODULE, name.at).with("name", Arg::Text(name.name.clone())).with("module", module_name());
                        self.say(m, d);
                    }
                }
            }
        }
        match target {
            ModTarget::Module { owner, module } if owner == m.owner => Some(module),
            _ => None,
        }
    }

    fn bind(&mut self, m: ModRef, ident: ast::Ident, name: Name) {
        if let Some(first) = self.scope(m).names.get(&ident.name) {
            let first = name_at(first);
            // Said at the later of the two.
            let (later, earlier) = if first < ident.at { (ident, first) } else { (ast::Ident { name: ident.name.clone(), at: first }, ident.at) };
            self.twice(m, &later, earlier);
            if later.at == first {
                self.scope_mut(m).names.insert(later.name, name);
            }
        } else {
            self.scope_mut(m).names.insert(ident.name, name);
        }
    }

    /// Cycles among the mod's modules: each one the walk finds is said once, at the `use` that closes it (a module
    /// in two cycles may show one now and the other once it is broken).
    fn cycles(&mut self, edges: &[Vec<(usize, Pos)>]) {
        let n = edges.len();
        // 0 unseen, 1 on the path, 2 done.
        let mut state = vec![0u8; n];
        let mut said = std::collections::BTreeSet::new();
        for start in 0..n {
            if state[start] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            let mut path: Vec<usize> = vec![start];
            state[start] = 1;
            while let Some((node, next)) = stack.last_mut() {
                let node = *node;
                if *next < edges[node].len() {
                    let (to, at) = edges[node][*next];
                    *next += 1;
                    match state[to] {
                        0 => {
                            state[to] = 1;
                            stack.push((to, 0));
                            path.push(to);
                        }
                        1 => {
                            let from = path.iter().position(|p| *p == to).unwrap_or(0);
                            let mut ring: Vec<usize> = path[from..].to_vec();
                            // One message per cycle, whichever module the walk entered it by.
                            let low = ring.iter().enumerate().min_by_key(|(_, p)| **p).map(|(i, _)| i).unwrap_or(0);
                            ring.rotate_left(low);
                            if said.insert(ring.clone()) {
                                let mut chain: Vec<&str> = path[from..].iter().map(|p| self.modules[*p].path.as_str()).collect();
                                chain.push(&self.modules[to].path);
                                let d = Diagnostic::new(code::MODULE_CYCLE, at).with("chain", Arg::Text(chain.join(" -> ")));
                                self.say(ModRef::of_mod(node), d);
                            }
                        }
                        _ => {}
                    }
                } else {
                    state[node] = 2;
                    stack.pop();
                    path.pop();
                }
            }
        }
    }

    /// The names a module's types may use besides the builtins (for "nearest" in messages).
    fn type_names(&self, m: ModRef) -> Vec<String> {
        let mut out: Vec<String> = BUILTIN_TYPES.iter().chain(GENERIC_TYPES.iter()).map(|(n, _)| n.to_string()).collect();
        for (name, n) in &self.scope(m).names {
            if matches!(n, Name::Item { sort: Sort::Struct | Sort::Enum | Sort::Class, .. } | Name::Taken { .. }) {
                out.push(name.clone());
            }
        }
        out
    }

    /// The item a scope name stands for, as a type.
    fn item_type(&self, item: ItemRef) -> Option<Ty> {
        match &self.source(item.module_ref()).module.items.get(item.item)?.kind {
            ItemKind::Struct(_) => Some(Ty::Struct(item)),
            ItemKind::Enum(_) => Some(Ty::Enum(item)),
            ItemKind::Class(_) => Some(Ty::Class(item)),
            _ => None,
        }
    }

    /// A name in module `m`'s scope as a type of the mod or a library; else the standard module's.
    fn scope_type(&self, m: ModRef, name: &str) -> Option<Ty> {
        match self.scope(m).names.get(name) {
            Some(Name::Item { item, .. }) => self.item_type(m.item(*item)),
            Some(Name::Taken { item, .. }) => self.item_type(*item),
            Some(Name::Module { .. }) => None,
            None => self.item_type(self.std_item(name)?),
        }
    }

    /// The standard module's public item `name` ([`STD_TEXT`]), when the build gave the module.
    pub(crate) fn std_item(&self, name: &str) -> Option<ItemRef> {
        let m = self.modules.iter().rposition(|s| s.path == STD_PATH)?;
        match self.out.scopes.get(m)?.names.get(name)? {
            Name::Item { item, public: true, .. } => Some(ModRef::of_mod(m).item(*item)),
            _ => None,
        }
    }

    /// A pub type of a used module (`item.Item`).
    fn module_type(&mut self, m: ModRef, target: &ModTarget, ident: &ast::Ident, written: &str) -> Ty {
        let ModTarget::Module { owner, module } = target else {
            let ModTarget::Service(service) = target else { unreachable!("a module target is matched above") };
            let services = &self.facts.game.services;
            if services.faces.is_empty() {
                // Not described (the note is at its `use`): known by its name.
                return Ty::Service { service: service.clone(), name: ident.name.clone() };
            }
            return match service_type(services, service, &ident.name) {
                Some(t) => t,
                None => {
                    let d = Diagnostic::new(code::NO_NAME_IN_MODULE, ident.at).with("name", Arg::Text(ident.name.clone())).with("module", Arg::Text(format!("svc.{service}")));
                    self.say(m, d);
                    Ty::Error
                }
            };
        };
        let scope = self.scope(ModRef { owner: *owner, module: *module });
        match scope.names.get(&ident.name) {
            Some(Name::Item { item, public, .. }) => {
                let item = ItemRef { owner: *owner, module: *module, item: *item };
                match (self.item_type(item), public) {
                    (Some(ty), true) => ty,
                    (Some(_), false) => {
                        let module = written.rsplit_once('.').map_or(written, |(a, _)| a).to_string();
                        let d = Diagnostic::new(code::NAME_NOT_PUB, ident.at).with("name", Arg::Text(ident.name.clone())).with("module", Arg::Text(module));
                        self.say(m, d);
                        Ty::Error
                    }
                    (None, _) => self.unknown_type(m, ident.at, written),
                }
            }
            _ => self.unknown_type(m, ident.at, written),
        }
    }

    fn unknown_type(&mut self, m: ModRef, at: Pos, written: &str) -> Ty {
        let known = self.type_names(m);
        let near = nearest(written, known.iter().map(String::as_str));
        let d = if near.is_empty() {
            Diagnostic::new(code::UNKNOWN_TYPE, at).with("found", Arg::Text(written.to_string()))
        } else {
            let near = near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
            Diagnostic::new(code::UNKNOWN_TYPE_NEAR, at).with("found", Arg::Text(written.to_string())).with("near", Arg::Text(near))
        };
        self.say(m, d);
        Ty::Error
    }

    /// A game type by its name, or what is wrong with it.
    fn game_type(&mut self, m: ModRef, at: Pos, name: &str, written: &str) -> Ty {
        match self.facts.game.type_by_name(name) {
            Ok(g) => Ty::from_game(g),
            Err(NotCrossing::NotKnown(_)) => self.unknown_type(m, at, written),
            Err(why) => {
                let d = Diagnostic::new(code::GAME_TYPE_NOT_CROSSING, at).with("found", Arg::Text(written.to_string())).with("why", Arg::Phrase(why_phrase(&why)));
                self.say(m, d);
                Ty::Error
            }
        }
    }

    /// A written type in module `m` (design section 3; lookup: the language's names, the module's and its uses', then
    /// the game's).
    fn resolve(&mut self, m: ModRef, t: &ast::Type) -> Ty {
        let mut ty = self.resolve_bare(m, t);
        for _ in 0..t.optional {
            if ty != Ty::Error {
                ty = Ty::Opt(Box::new(ty));
            }
        }
        ty
    }

    fn resolve_bare(&mut self, m: ModRef, t: &ast::Type) -> Ty {
        match &t.kind {
            TypeKind::Tuple(items) if items.is_empty() => Ty::Unit,
            TypeKind::Tuple(items) => Ty::Tuple(items.iter().map(|i| self.resolve(m, i)).collect()),
            TypeKind::Fn { params, ret } => {
                let ps = params.iter().map(|p| self.resolve(m, p)).collect();
                let r = ret.as_ref().map_or(Ty::Unit, |r| self.resolve(m, r));
                Ty::Fn(ps, Box::new(r))
            }
            TypeKind::Ref(args) => {
                if args.len() != 1 {
                    let d = Diagnostic::new(code::TYPE_ARG_COUNT, t.at).with("found", Arg::Text("ref".into())).with("expected", Arg::Text("1".into()));
                    self.say(m, d);
                    return Ty::Error;
                }
                match self.resolve(m, &args[0]) {
                    Ty::Error => Ty::Error,
                    of @ (Ty::Class(_) | Ty::Game(GameTy::View { .. })) => Ty::Ref(Box::new(of)),
                    _ => {
                        self.say(m, Diagnostic::new(code::REF_OF, args[0].at).with("found", Arg::Text(type_text(&args[0]))));
                        Ty::Error
                    }
                }
            }
            TypeKind::Named { parts, seps, quoted, args } => {
                let written = type_text(&ast::Type { at: t.at, kind: t.kind.clone(), optional: 0 });
                let at = parts[0].at;
                // A game name: backquoted or with `::`.
                if *quoted || seps.contains(&Sep::Colons) {
                    let name = parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("::");
                    if !args.is_empty() {
                        // Template arguments are part of a game name only inside backquotes (`` `Vec3<f32>` ``).
                        return self.arg_count(m, t.at, &written, 0);
                    }
                    return self.game_type(m, at, &name, &written);
                }
                if parts.len() == 1 {
                    let name = parts[0].name.as_str();
                    if let Some(n) = arity(name) {
                        if args.len() != n {
                            return self.arg_count(m, t.at, name, n);
                        }
                        let a: Vec<Ty> = args.iter().map(|a| self.resolve(m, a)).collect();
                        let ty = self.builtin(m, name, a, args);
                        self.hides_game_type(m, at, name, &ty);
                        return ty;
                    }
                    if !args.is_empty() {
                        return self.arg_count(m, t.at, &written, 0);
                    }
                    if let Some(ty) = self.scope_type(m, name) {
                        self.hides_game_type(m, at, name, &ty);
                        return ty;
                    }
                    return self.game_type(m, at, name, &written);
                }
                // `json.Value`, or `<module>.<Type>`.
                if parts.len() == 2 && parts[0].name == "json" && parts[1].name == "Value" && !self.scope(m).names.contains_key("json") {
                    return Ty::Json;
                }
                if !args.is_empty() {
                    return self.arg_count(m, t.at, &written, 0);
                }
                match self.scope(m).names.get(&parts[0].name).cloned() {
                    Some(Name::Module { target, .. }) if parts.len() == 2 => self.module_type(m, &target, &parts[1], &written),
                    _ => self.unknown_type(m, at, &written),
                }
            }
        }
    }

    /// A note when a name of the language or the mod hides a game type of that name: the game's
    /// is reached in backquotes. Once per name; none when both are the same type (`f32`).
    fn hides_game_type(&mut self, m: ModRef, at: Pos, name: &str, ours: &Ty) {
        // A library's names were noted when it was built; the note is the mod's, once.
        if m.owner != Owner::Mod {
            return;
        }
        let Some(types) = self.facts.game.types.as_ref() else { return };
        if types.record(name).is_none() && !types.enums.contains_key(name) && !types.typedefs.contains_key(name) {
            return;
        }
        if self.facts.game.type_by_name(name).map(Ty::from_game).as_ref() == Ok(ours) || !self.hiding_noted.insert(name.to_string()) {
            return;
        }
        self.say(m, Diagnostic::new(code::NAME_HIDES_GAME, at).with("found", Arg::Text(name.to_string())));
    }

    /// Structs and enums are values (docs/modding/language/language.md, section 3): one that holds itself by value, through its fields, tuples,
    /// optionals or its cases' fields, has no size. Each such cycle is said once, at the field that
    /// closes it, with its chain; a list, map, set, `ref` or function between them breaks it.
    fn value_cycles(&mut self) {
        let mut nodes: Vec<ItemRef> = Vec::new();
        let mut edges: HashMap<ItemRef, Vec<(ItemRef, Pos)>> = HashMap::new();
        for (m, source) in self.modules.iter().enumerate() {
            for (i, item) in source.module.items.iter().enumerate() {
                let me = ItemRef { owner: Owner::Mod, module: m, item: i };
                let fields: Vec<(String, Pos)> = match &item.kind {
                    ItemKind::Struct(s) => s.fields.iter().map(|f| (format!(".{}", f.name.name), f.ty.at)).collect(),
                    ItemKind::Enum(e) => e.cases.iter().flat_map(|c| c.fields.iter().map(move |f| (format!("{}.{}", c.name.name, f.name.name), f.ty.at))).collect(),
                    _ => continue,
                };
                nodes.push(me);
                let out = edges.entry(me).or_default();
                for (key, at) in fields {
                    let mut held = Vec::new();
                    if let Some(ty) = self.out.types.get(&(me, key)) {
                        held_by_value(ty, &mut held);
                    }
                    out.extend(held.into_iter().map(|to| (to, at)));
                }
            }
        }
        // 0 unseen, 1 on the path, 2 done; as `cycles`.
        let mut state: HashMap<ItemRef, u8> = HashMap::new();
        let mut said = std::collections::BTreeSet::new();
        for start in nodes {
            if state.get(&start).copied().unwrap_or(0) != 0 {
                continue;
            }
            let mut stack: Vec<(ItemRef, usize)> = vec![(start, 0)];
            let mut path: Vec<ItemRef> = vec![start];
            state.insert(start, 1);
            while let Some((node, next)) = stack.last_mut() {
                let node = *node;
                let list = edges.get(&node).map(Vec::as_slice).unwrap_or(&[]);
                if *next < list.len() {
                    let (to, at) = list[*next];
                    *next += 1;
                    match state.get(&to).copied().unwrap_or(0) {
                        0 => {
                            state.insert(to, 1);
                            stack.push((to, 0));
                            path.push(to);
                        }
                        1 => {
                            let from = path.iter().position(|p| *p == to).unwrap_or(0);
                            let mut ring: Vec<ItemRef> = path[from..].to_vec();
                            let low = ring.iter().enumerate().min_by_key(|(_, p)| **p).map(|(i, _)| i).unwrap_or(0);
                            ring.rotate_left(low);
                            if said.insert(ring) {
                                let mut chain: Vec<String> = path[from..].iter().map(|p| self.item_name(*p)).collect();
                                chain.push(self.item_name(to));
                                let d = Diagnostic::new(code::VALUE_CYCLE, at).with("chain", Arg::Text(chain.join(" -> ")));
                                self.say(node.module_ref(), d);
                            }
                        }
                        _ => {}
                    }
                } else {
                    state.insert(node, 2);
                    stack.pop();
                    path.pop();
                }
            }
        }
    }

    /// An item's name as written.
    fn item_name(&self, item: ItemRef) -> String {
        let module = match item.owner {
            Owner::Mod => &self.modules[item.module].module,
            Owner::Library(l) => &self.libraries[l].modules[item.module].module,
        };
        match &module.items[item.item].kind {
            ItemKind::Struct(s) => s.name.name.clone(),
            ItemKind::Enum(e) => e.name.name.clone(),
            ItemKind::Class(c) => c.name.name.clone(),
            _ => String::new(),
        }
    }

    fn arg_count(&mut self, m: ModRef, at: Pos, name: &str, n: usize) -> Ty {
        let d = Diagnostic::new(code::TYPE_ARG_COUNT, at).with("found", Arg::Text(name.to_string())).with("expected", Arg::Text(n.to_string()));
        self.say(m, d);
        Ty::Error
    }

    fn builtin(&mut self, m: ModRef, name: &str, mut a: Vec<Ty>, written: &[ast::Type]) -> Ty {
        let int = |s| Ty::Int(s);
        match name {
            "bool" => Ty::Bool,
            "i8" => int(Scalar::I8),
            "i16" => int(Scalar::I16),
            "i32" | "int" => int(Scalar::I32),
            "i64" => int(Scalar::I64),
            "u8" => int(Scalar::U8),
            "u16" => int(Scalar::U16),
            "u32" => int(Scalar::U32),
            "u64" => int(Scalar::U64),
            "f32" | "float" => Ty::Float(Scalar::F32),
            "f64" => Ty::Float(Scalar::F64),
            "vec2" => Ty::Value(Value::Vec2),
            "vec3" => Ty::Value(Value::Vec3),
            "vec4" => Ty::Value(Value::Vec4),
            "quat" => Ty::Value(Value::Quat),
            "mat34" => Ty::Value(Value::Mat34),
            "color" => Ty::Value(Value::Color),
            "text" => Ty::Text,
            "bytes" => Ty::Bytes,
            "model" | "effect" | "sound" | "layout" | "font" | "movie" => Ty::Content(name.to_string()),
            "list" => Ty::List(Box::new(a.remove(0))),
            "set" | "map" => {
                // A key is an integer, text, bool or an enum without data (docs/modding/language/language.md, section 3).
                let key_ok = match &a[0] {
                    Ty::Int(_) | Ty::Text | Ty::Bool | Ty::Error | Ty::Game(GameTy::Enum(_)) => true,
                    Ty::Enum(e) => self.enum_without_data(*e),
                    _ => false,
                };
                if !key_ok {
                    self.say(m, Diagnostic::new(code::MAP_KEY, written[0].at).with("found", Arg::Text(type_text(&written[0]))));
                    return Ty::Error;
                }
                let k = a.remove(0);
                if name == "set" {
                    Ty::Set(Box::new(k))
                } else {
                    Ty::Map(Box::new(k), Box::new(a.remove(0)))
                }
            }
            "pending" => Ty::Pending(Box::new(a.remove(0))),
            _ => Ty::Error,
        }
    }

    fn enum_without_data(&self, e: ItemRef) -> bool {
        matches!(&self.source(e.module_ref()).module.items[e.item].kind, ItemKind::Enum(d) if d.cases.iter().all(|c| c.fields.is_empty()))
    }

    /// Parameters: each named once, each written type resolved. (The parser has required the types outside hook and
    /// override heads, and `self` first in a member's list only: S0019.)
    fn params(&mut self, m: ModRef, owner: ItemRef, key: &str, params: &[ast::Param]) {
        let mut seen: HashMap<String, Pos> = HashMap::new();
        for (i, p) in params.iter().enumerate() {
            if let Some(name) = &p.name {
                if let Some(first) = seen.get(&name.name) {
                    self.twice(m, name, *first);
                } else {
                    seen.insert(name.name.clone(), name.at);
                }
            }
            if let Some(t) = &p.ty {
                let ty = self.resolve(m, t);
                let pname = p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone());
                self.out.types.insert((owner, format!("{key}({pname})")), ty);
            }
        }
    }

    fn fn_decl(&mut self, m: ModRef, owner: ItemRef, key: &str, f: &ast::FnDecl) {
        self.params(m, owner, key, &f.params);
        if let Some(r) = &f.ret {
            let ty = self.resolve(m, r);
            self.out.types.insert((owner, format!("{key}->")), ty);
        }
    }

    /// A hook's written parameter and result types (a class's hooks are keyed `.hook<member index>`).
    fn hook(&mut self, m: ModRef, owner: ItemRef, key: &str, h: &ast::Hook) {
        if let Some(params) = &h.params {
            self.params(m, owner, key, params);
        }
        if let Some(ast::HookRet::Type(t)) = &h.ret {
            let ty = self.resolve(m, t);
            self.out.types.insert((owner, format!("{key}->")), ty);
        }
    }

    /// Fields named once in one scope, each type resolved.
    fn fields(&mut self, m: ModRef, owner: ItemRef, prefix: &str, fields: &[ast::Field], seen: &mut HashMap<String, Pos>) {
        for f in fields {
            if let Some(first) = seen.get(&f.name.name) {
                self.twice(m, &f.name, *first);
                continue;
            }
            seen.insert(f.name.name.clone(), f.name.at);
            let ty = self.resolve(m, &f.ty);
            self.out.types.insert((owner, format!("{prefix}{}", f.name.name)), ty);
        }
    }

    /// Pass 3: one item's declaration.
    fn declaration(&mut self, m: ModRef, i: usize, item: &ast::Item) {
        let me = m.item(i);
        match &item.kind {
            ItemKind::Option { key, ty } => self.option(m, me, key, ty),
            ItemKind::Binding(b) => {
                if let Some(t) = &b.ty {
                    let ty = self.resolve(m, t);
                    self.out.types.insert((me, String::new()), ty);
                }
            }
            ItemKind::Fn(f) => self.fn_decl(m, me, "", f),
            ItemKind::Task { params, ret, .. } => {
                self.params(m, me, "", params);
                if let Some(r) = ret {
                    let ty = self.resolve(m, r);
                    self.out.types.insert((me, "->".into()), ty);
                }
            }
            ItemKind::Extern { params, ret, .. } | ItemKind::Export { params, ret, .. } => {
                self.params(m, me, "", params);
                if let Some(r) = ret {
                    let ty = self.resolve(m, r);
                    self.out.types.insert((me, "->".into()), ty);
                }
            }
            ItemKind::Struct(s) => {
                let mut seen = HashMap::new();
                self.fields(m, me, ".", &s.fields, &mut seen);
                for f in &s.fns {
                    if let Some(first) = seen.get(&f.name.name) {
                        self.twice(m, &f.name, *first);
                        continue;
                    }
                    seen.insert(f.name.name.clone(), f.name.at);
                    self.fn_decl(m, me, &format!(".{}", f.name.name), f);
                }
            }
            ItemKind::Enum(e) => {
                let mut cases = HashMap::new();
                for case in &e.cases {
                    if let Some(first) = cases.get(&case.name.name) {
                        self.twice(m, &case.name, *first);
                        continue;
                    }
                    cases.insert(case.name.name.clone(), case.name.at);
                    let mut seen = HashMap::new();
                    self.fields(m, me, &format!("{}.", case.name.name), &case.fields, &mut seen);
                }
            }
            ItemKind::Class(c) => self.class(m, me, c),
            ItemKind::States(s) => self.states(m, s),
            ItemKind::Hook(h) => self.hook(m, me, "", h),
            ItemKind::Use(_) | ItemKind::On(_) | ItemKind::Test { .. } => {}
        }
    }

    /// `option key: type` against luma.json.
    fn option(&mut self, m: ModRef, me: ItemRef, key: &ast::Ident, t: &ast::Type) {
        let Some(spec) = self.facts.options.iter().find(|o| o.key == key.name) else {
            self.say(m, Diagnostic::new(code::OPTION_NOT_IN_LUMA_JSON, key.at).with("found", Arg::Text(key.name.clone())));
            return;
        };
        let (expected, ok) = match spec.kind {
            OptionKind::Key => {
                // `input.Action` of `use svc.input`, by its type, not its spelling.
                let ty = self.resolve(m, t);
                let ok = ty == Ty::Error || ty == Ty::Service { service: "input".into(), name: "Action".into() };
                self.out.types.insert((me, String::new()), ty);
                ("input.Action", ok)
            }
            kind => {
                let ty = self.resolve(m, t);
                let (expected, want) = match kind {
                    OptionKind::Toggle => ("bool", Ty::Bool),
                    OptionKind::Number { whole: true } => ("int", Ty::Int(Scalar::I32)),
                    OptionKind::Number { whole: false } => ("f32", Ty::Float(Scalar::F32)),
                    OptionKind::Text | OptionKind::Choice | OptionKind::Key => ("text", Ty::Text),
                };
                self.out.types.insert((me, String::new()), ty.clone());
                (expected, ty == want || ty == Ty::Error)
            }
        };
        if !ok {
            let d = Diagnostic::new(code::OPTION_TYPE, t.at).with("found", Arg::Text(key.name.clone())).with("expected", Arg::Text(expected.into()));
            self.say(m, d);
        }
    }

    /// `class X : Base { ... }` (docs/modding/language/language.md, section 5.5).
    fn class(&mut self, m: ModRef, me: ItemRef, c: &ast::ClassDecl) {
        let base = &c.base;
        let mut base_record = None;
        let mut gravity = false;
        match self.facts.game.type_by_name(&base.text) {
            // The kernel makes a mod's class from a polymorphic class with NameObj at its start (docs/modding/language/classes.md).
            Ok(GameTy::View { record, .. }) => {
                if self.facts.game.derivable(&record) {
                    base_record = Some(record);
                } else if self.facts.game.gravity_base(&record) {
                    // A gravity class: no stage places it and no factory row makes it, so `made by` and `replaces`
                    // name nothing it could be (the kernel refuses the definition, -1).
                    for named in [&c.maker, &c.replaces].into_iter().flatten() {
                        self.say(m, Diagnostic::new(code::NOT_DERIVABLE, named.at).with("found", Arg::Text(base.text.clone())));
                    }
                    gravity = true;
                    base_record = Some(record);
                } else {
                    self.say(m, Diagnostic::new(code::NOT_DERIVABLE, base.at).with("found", Arg::Text(base.text.clone())));
                }
            }
            Ok(_) => self.say(m, Diagnostic::new(code::BASE_NOT_GAME_CLASS, base.at).with("found", Arg::Text(base.text.clone()))),
            Err(NotCrossing::NotKnown(_)) => {
                self.unknown_type(m, base.at, &base.text);
            }
            Err(why) => {
                let d = Diagnostic::new(code::GAME_TYPE_NOT_CROSSING, base.at).with("found", Arg::Text(base.text.clone())).with("why", Arg::Phrase(why_phrase(&why)));
                self.say(m, d);
            }
        }
        // `made by <row>` and `replaces <name>` name rows of the game's object factory
        // (docs/modding/language/game-description.md, section 9). The maker, `made by`'s row or else the base's own name, must make exactly the base:
        // the kernel keeps the game's object otherwise (docs/modding/language/classes.md). With no placement facts the kernel says
        // these at define_class; a name replaced already is always its to say.
        if self.facts.game.factory.is_some() && !gravity {
            for named in [&c.maker, &c.replaces].into_iter().flatten() {
                if self.facts.game.factory_row(&named.text).is_none() {
                    self.say(m, Diagnostic::new(code::CLASS_NAME_UNKNOWN, named.at).with("found", Arg::Text(named.text.clone())));
                }
            }
            if let Some(record) = &base_record {
                let maker = c.maker.as_ref().unwrap_or(base);
                match self.facts.game.factory_row(&maker.text) {
                    None if c.maker.is_none() => {
                        let d = Diagnostic::new(code::BASE_NOT_MADE, base.at).with("found", Arg::Text(c.name.name.clone())).with("base", Arg::Text(base.text.clone()));
                        self.say(m, d);
                    }
                    Some(Some(made)) if made != record.as_str() => {
                        let d = Diagnostic::new(code::MAKER_MAKES_OTHER, maker.at)
                            .with("found", Arg::Text(maker.text.clone()))
                            .with("made", Arg::Text(made.to_string()))
                            .with("base", Arg::Text(record.clone()));
                        self.say(m, d);
                    }
                    _ => {}
                }
            }
        }
        if let Some(r) = &c.replaces {
            if r.text == c.name.name {
                self.say(m, Diagnostic::new(code::REPLACES_ITSELF, r.at).with("found", Arg::Text(r.text.clone())));
            }
        }
        let mut seen: HashMap<String, Pos> = HashMap::new();
        let mut virtuals_noted = false;
        for (n, member) in c.members.iter().enumerate() {
            match member {
                ClassMember::Binding(b) => {
                    let mut idents = Vec::new();
                    pattern_names(&b.pattern, &mut idents);
                    for ident in idents {
                        if let Some(first) = seen.get(&ident.name) {
                            self.twice(m, &ident, *first);
                        } else {
                            seen.insert(ident.name.clone(), ident.at);
                            if let Some(t) = &b.ty {
                                let ty = self.resolve(m, t);
                                self.out.types.insert((me, format!(".{}", ident.name)), ty);
                            }
                        }
                    }
                }
                ClassMember::Fn(f) | ClassMember::Override(f) => {
                    let is_override = matches!(member, ClassMember::Override(_));
                    if let Some(first) = seen.get(&f.name.name) {
                        self.twice(m, &f.name, *first);
                        continue;
                    }
                    seen.insert(f.name.name.clone(), f.name.at);
                    // The virtual it replaces: a row the function facts mark virtual in the base's chain (body.rs types it).
                    // A game with no function facts ('s file) does not say which are virtual: a note, once a class.
                    if let (true, Some(record)) = (is_override, &base_record) {
                        let arity = f.params.iter().filter(|p| !p.is_self).count();
                        if self.facts.game.facts.is_none() {
                            if !virtuals_noted {
                                virtuals_noted = true;
                                let d = Diagnostic::new(code::NOT_DESCRIBED, f.name.at).with("found", Arg::Text(f.name.name.clone())).with("what", Arg::Phrase("Script.What.Virtuals"));
                                self.say(m, d);
                            }
                        } else if self.facts.game.virtual_of(record, &f.name.name, Some(arity)).is_none() {
                            let d = match self.facts.game.virtuals_named(record, &f.name.name) {
                                // Several and the parameter count does not choose: each is named.
                                Some((_, sigs)) if sigs.len() > 1 => Diagnostic::new(code::OVERRIDE_AMBIGUOUS, f.name.at)
                                    .with("found", Arg::Text(f.name.name.clone()))
                                    .with("class", Arg::Text(base.text.clone()))
                                    .with("signatures", Arg::Text(sigs.join("; "))),
                                _ => Diagnostic::new(code::OVERRIDE_NOT_VIRTUAL, f.name.at).with("found", Arg::Text(f.name.name.clone())).with("class", Arg::Text(base.text.clone())),
                            };
                            self.say(m, d);
                        }
                    }
                    self.fn_decl(m, me, &format!(".{}", f.name.name), f);
                }
                ClassMember::Hook(h) => self.hook(m, me, &format!(".hook{n}"), h),
            }
        }
    }

    /// `states Name for Class { ... start S }` (docs/modding/language/language.md, section 11.2).
    fn states(&mut self, m: ModRef, s: &ast::States) {
        match self.scope_type(m, &s.class.name) {
            // The class whose objects each run the machine: the back end keeps it in their data (states.rs).
            Some(Ty::Class(class)) => {
                if m.owner == Owner::Mod {
                    self.out.named.insert((m.module, s.class.at), body::Resolved::Item(class));
                }
            }
            _ => self.say(m, Diagnostic::new(code::STATES_FOR, s.class.at).with("found", Arg::Text(s.class.name.clone()))),
        }
        let mut seen: HashMap<String, Pos> = HashMap::new();
        for state in &s.states {
            if let Some(first) = seen.get(&state.name.name) {
                self.twice(m, &state.name, *first);
            } else {
                seen.insert(state.name.name.clone(), state.name.at);
            }
        }
        if !s.start.name.is_empty() && !seen.contains_key(&s.start.name) {
            self.say(m, Diagnostic::new(code::START_NOT_STATE, s.start.at).with("found", Arg::Text(s.start.name.clone())));
        }
    }
}

/// The mod's structs and enums a type holds by value: through tuples and optionals, not through a container, a
/// `ref` or a function.
fn held_by_value(ty: &Ty, out: &mut Vec<ItemRef>) {
    match ty {
        Ty::Struct(i) | Ty::Enum(i) if i.owner == Owner::Mod => out.push(*i),
        Ty::Tuple(items) => items.iter().for_each(|t| held_by_value(t, out)),
        Ty::Opt(t) => held_by_value(t, out),
        _ => {}
    }
}

fn name_at(n: &Name) -> Pos {
    match n {
        Name::Item { at, .. } | Name::Module { at, .. } | Name::Taken { at, .. } => *at,
    }
}
