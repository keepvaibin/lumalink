//! Part (c): bodies (design section 4, pass 4; revisions 2 to 4). Every expression of the mod's modules gets its type
//! and every name, member and call what it stands for, in [`super::Checked`]. Lookup is in this order: locals, the
//! module, its uses, the class's members, the standard names, then the game's. Literals take the type their place
//! needs (docs/modding/language/language.md, section 3); a game call chooses its signature by its arguments (7.1). One mistake is said once:
//! a part that does not check is `Ty::Error`, which fits everything after.
//!
//! Part (d), flow, is walked with it: a function with a result that can reach its end (S0101), a `guard` whose `else`
//! stays (S0102), a view kept where it outlives its statement (S0103: a module value, a field, a closure that may run
//! later, a task's parameter), a view used after a `wait` it was made before (S0104), `wait` where it cannot be
//! (S0105), and leaving or waiting inside `defer` (S0106).
//!
//! Part (e), hooks, tasks and states, is walked with them too: a hook's head chooses the game function's row (by a
//! quoted signature, the parameters' count, their written types) and gives its parameters, `self` and result their
//! types from it (S0112 to S0116, S0128); `skip` (S0117 to S0119), `result` (S0120, S0124), `proceed()` (S0121,
//! S0122) and a wrap's or replace's every path giving the result (S0123) follow the paths as part (d)'s waits do;
//! `stop` and `go` are placed (S0125 to S0127, S0130), a state's part is given once (S0129). `on step`, a
//! second `on` of one kind (S0109, in [`bodies`]), a literal negative wait (S0110).
//!
//! Part (f): services (`service.rs`). What is part (f)'s and is not said here yet: content names, options'
//! events. An event of a module that is no service binds its names as `Ty::Error`.

mod call;
mod expr;
mod pat;
mod service;

pub(crate) use service::{answer_ty, event_name, sty as service_sty};
pub(crate) use call::{builtins_of, STD_NAMES};

use super::{nearest, Checker, ItemRef, ModRef, Name, Owner, Sort, Ty};
use crate::ast::{self, AssignOp, Block, ClassMember, Expr, ExprId, ExprKind, ItemKind, Pos, Stmt, StmtKind};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::{GameTy, Scalar, Value};
use std::collections::HashMap;

/// A local's index in [`super::Checked::locals`].
pub type LocalId = u32;

/// A local: a parameter, a `let` or `var` in a body, a pattern's or a loop's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub at: Pos,
    pub ty: Ty,
    /// It may be written (`var`, a `var` parameter, a hook's parameter).
    pub var: bool,
    /// The module of the mod it is in.
    pub module: usize,
}

/// What a name, member or call stands for (4.3 reads it to write the C).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Local(LocalId),
    /// An item of the mod or a library: a function, task, `let`, `var`, option, extern, export, type.
    Item(ItemRef),
    /// A field or function of the mod's struct or class (`owner`), by name.
    Member { owner: ItemRef, name: String },
    /// An enum's case, by name.
    Case { item: ItemRef, case: String },
    /// A game function, with the signature chosen (its row's text).
    GameFn { name: String, signature: String },
    /// A field of a game record: its import row `record.field`.
    GameField { record: String, field: String },
    /// A game global by its qualified name (its kind 7 row).
    GameGlobal(String),
    /// A constant the game's module does not define: its value, folded.
    GameConstant { name: String, value: String },
    /// A value of a game enum.
    GameEnumValue { enum_name: String, value: String, number: i64 },
    /// A standard name (16a): `min`, `json.decode`, `vec3`...
    Std(&'static str),
    /// `set_table(...)` (`write`) or `table(...)` laid out (docs/modding/language/numbers.md): the table's name, the
    /// offset its literal steps reach, each index argument that is no literal (its place among the arguments, the
    /// element's stride and the array's count), and the place written or read.
    Table { write: bool, name: String, offset: u64, indexes: Vec<(usize, u64, u64)>, leaf: crate::game::TableLeaf },
    /// A builtin method of a container, text, bytes, a handle, a task or `json.Value` (language.md).
    Method(&'static str),
    /// `u32.max`, `i16.min`.
    Limit { ty: Scalar, max: bool },
    /// A tuple's `.0`, `.1`.
    TupleIndex(usize),
    /// A vector's, quaternion's or colour's component by index (`x y z w`, `r g b a`).
    Component(usize),
    /// `go` to a state: its machine (a `states` item) and the state's name (part e).
    State { machine: ItemRef, state: String },
    /// A service's function: `waited` when called without its request's closure (`wait`).
    ServiceFn { service: String, name: String, waited: bool },
    /// A case of a service's enum, or of `svc.Answer` (`service` "svc"), with its number.
    ServiceCase { service: String, ty: String, case: String, number: i64 },
    /// A case of a service's stream events `<svc>.Event.<Kind>`: its kind, lower case, and number.
    StreamCase { service: String, kind: String, number: i64 },
    /// A service's `#define` constant, folded.
    ServiceConst { service: String, name: String, value: u64 },
    /// A field of a service's struct or event record (`content` for the shared header's).
    ServiceField { service: String, record: String, field: String },
    /// `base.<name>(...)` in an `override fn <name>` (docs/modding/language/language.md, section 5.5): the base class's own function.
    BaseCall,
    /// An optional service's alias tested against `none` (`use optional svc.<name>`): whether the service was given.
    ServiceGiven(String),
}

/// A hook's game function as part (e) chose it (4.3 writes the hook's row from it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookTarget {
    pub name: String,
    /// The row's signature text.
    pub signature: String,
    /// With `every override of`: each override the function facts list, directly or through another, by name and
    /// signature (kernel part `K.vhook` hooks each).
    pub overrides: Vec<(String, String)>,
}

/// What the walk of a hook's body knows of its head (part e).
#[derive(Clone, Debug)]
struct HookCx {
    kind: ast::HookKind,
    /// The hooked function's name, for messages.
    name: String,
    /// Its result's type: None when it returns nothing (a constructor's and a destructor's too); `Err` with the phrase
    /// saying why when the type does not cross (said where the body first needs it).
    result: Option<Result<Ty, &'static str>>,
    /// S0124 said once a hook.
    not_yet_said: bool,
}

/// What is known of the paths from a body's start to the place being walked (part d's waits, part e's hook flow).
/// A branch that leaves takes its path with it; the paths of branches that go on are joined.
#[derive(Clone, Debug)]
pub(super) struct Path {
    /// The waits on some path, their tick and place, in tick order (S0104).
    waits: Vec<(u32, Pos)>,
    /// A `proceed()` on some path: the first one's place (S0122).
    proceeded: Option<Pos>,
    /// On every path the hooked function's result is given: `proceed()` ran or `result` was set (S0123, S0124).
    given: bool,
}

impl Path {
    fn new() -> Path {
        Path { waits: Vec::new(), proceeded: None, given: true }
    }

    /// `from` joined into `into` (None: no path yet, so `from` itself).
    pub(super) fn join(into: &mut Option<Path>, from: &Path) {
        match into {
            None => *into = Some(from.clone()),
            Some(p) => {
                join_waits(&mut p.waits, &from.waits);
                p.proceeded = p.proceeded.or(from.proceeded);
                p.given &= from.given;
            }
        }
    }
}

/// A value of a game enum as a script may name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameValue {
    /// The enum's row name; empty for an anonymous enum's value, which is an integer.
    pub enum_name: String,
    pub value: String,
    pub number: i64,
}

/// What a value is wanted as.
#[derive(Clone, Copy, Debug)]
pub(super) enum Want<'t> {
    /// A statement's value, dropped.
    Drop,
    /// Any value: its own type stands (a literal takes its default).
    Any,
    Ty(&'t Ty),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fn,
    Task,
    Hook,
    Handler,
    State,
    Test,
    /// A module `let`'s or a field's value, a parameter's default.
    Init,
    Closure,
}

/// Whose members a body reaches without `self.`: the mod's class, or the game record a hook is on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Members {
    Mod(ItemRef),
    Game(String),
}

struct Frame {
    kind: Kind,
    /// What `return` gives; None: no value.
    ret: Option<Ty>,
    /// What `self` is here, and whether it may be written (a struct function's `var self`).
    self_ty: Option<Ty>,
    self_var: bool,
    members: Option<Members>,
    scopes: Vec<HashMap<String, LocalId>>,
    loops: u32,
    /// A closure written as a built-in's closure argument (`filter`, `map`, `sort_by` ...): it runs at once, inside
    /// the statement, so it may read the views around it (S0103).
    runs_now: bool,
    /// `wait` may be here: a task, a state's body or `enter`, a handler of start, a stage, an option or
    /// an event, a game test.
    waits_ok: bool,
    /// The paths from the body's start to the place being walked (A42D-3; part e).
    path: Path,
    /// The loops being walked, innermost last (S0104 and a `while true` that never ends).
    loop_marks: Vec<LoopMark>,
    /// For each `defer` being walked, the loops open where it started (S0106: `break` and `continue` of a loop inside
    /// it may stay).
    defers: Vec<u32>,
    /// In a state's `update` or `exit` (which run on later frames than the state's body): the tick they start at. A view
    /// the body made before it would be kept by the state (S0103).
    later_from: Option<u32>,
    /// A hook's own body (not a closure in it): its head (part e).
    hook: Option<HookCx>,
    /// `self` is there in name only: a constructor's `before`, a destructor's `after` (S0128, the phrase).
    self_gone: Option<&'static str>,
    /// A state machine's body: its `states` item (part e, `go`).
    machine: Option<ItemRef>,
    /// An `on start`'s or `on stop`'s own body: which (`keep` and `kept`).
    event: Option<&'static str>,
    /// An `override fn`'s own body (docs/modding/language/language.md, section 5.5): what `base.<name>(...)` calls.
    pub(super) override_of: Option<OverrideCx>,
}

/// The virtual an `override fn` replaces, as `base.` calls it: its name, parameter types, result (None: none).
#[derive(Clone, Debug)]
pub(super) struct OverrideCx {
    pub(super) name: String,
    pub(super) params: Vec<Ty>,
    pub(super) ret: Option<Ty>,
}

impl Frame {
    fn new(kind: Kind, ret: Option<Ty>) -> Frame {
        Frame {
            kind,
            ret,
            self_ty: None,
            self_var: false,
            members: None,
            scopes: vec![HashMap::new()],
            loops: 0,
            runs_now: false,
            waits_ok: false,
            path: Path::new(),
            loop_marks: Vec::new(),
            defers: Vec::new(),
            later_from: None,
            hook: None,
            self_gone: None,
            machine: None,
            event: None,
            override_of: None,
        }
    }
}

/// A loop being walked: where it started, whether a `break` leaves it, and the views it reads that were made before it
/// (each read again on the next time round, after any `wait` on a path back round).
struct LoopMark {
    start: u32,
    broke: bool,
    uses: Vec<(LocalId, Pos)>,
    /// The path where the loop may end without a `break`: at its start (`for`) or after its condition (`while`).
    entry: Path,
    /// The paths that go round again by `continue`.
    rounds: Option<Path>,
    /// The paths that leave by `break`.
    exits: Option<Path>,
}

/// Adds the waits of `from` that `into` lacks, keeping tick order.
fn join_waits(into: &mut Vec<(u32, Pos)>, from: &[(u32, Pos)]) {
    for w in from {
        if !into.iter().any(|x| x.0 == w.0) {
            into.push(*w);
        }
    }
    into.sort_by_key(|w| w.0);
}

/// What a value reads first: through `( )`, `!`, `-`, `not`, a binary's left, a member's or index's base, `as`, a
/// method call's object (S0105's whole-wait rule).
fn first_read(e: &Expr) -> &Expr {
    let mut inner = e;
    loop {
        inner = match &inner.kind {
            ExprKind::Paren(x) | ExprKind::Unwrap(x) | ExprKind::Neg(x) | ExprKind::Not(x) => x,
            ExprKind::Binary { left, .. } => left,
            ExprKind::Member { base, .. } | ExprKind::Index { base, .. } => base,
            ExprKind::As { value, .. } => value,
            ExprKind::Call { callee, .. } if matches!(callee.kind, ExprKind::Member { .. }) => callee,
            _ => return inner,
        };
    }
}

/// Whether a value of this type is or holds a view of the game's memory (docs/modding/language/language.md, section 6.1): a game object, a game array or
/// place, a class's object, a field group; through `?`, tuples and containers. A copy of a plain game struct is not.
pub(super) fn holds_view(t: &Ty) -> bool {
    match t {
        Ty::Class(_) | Ty::Group { .. } => true,
        Ty::Game(g) => game_holds_view(g),
        Ty::Opt(e) | Ty::List(e) | Ty::Set(e) => holds_view(e),
        Ty::Map(k, v) => holds_view(k) || holds_view(v),
        Ty::Tuple(items) => items.iter().any(holds_view),
        _ => false,
    }
}

fn game_holds_view(g: &GameTy) -> bool {
    match g {
        GameTy::View { .. } | GameTy::Array { .. } | GameTy::Place { .. } => true,
        GameTy::Opt(of) => game_holds_view(of),
        _ => false,
    }
}

/// Why a place cannot be written (S0064's phrase), or Ok.
type Writable = Result<(), &'static str>;

/// One walk of one module's bodies (or of one module-level value, found on demand).
pub(super) struct Walk<'w, 'a> {
    c: &'w mut Checker<'a>,
    m: ModRef,
    /// The mod's own: what is found goes into [`super::Checked`] and errors are said.
    record: bool,
    frames: Vec<Frame>,
    tys: HashMap<ExprId, Ty>,
    res: HashMap<ExprId, Resolved>,
    named: HashMap<Pos, Resolved>,
    /// Expressions that are a place in the game's storage a script may write through (a `T &` result, a pointer
    /// field's `!`), for [`Walk::place`].
    game_places: std::collections::HashSet<ExprId>,
    /// Names that read a local a closure copied (5.4: read only there).
    captured: std::collections::HashSet<ExprId>,
    /// Game fields, globals and elements a script may not write, with the phrase that says why.
    read_only: HashMap<ExprId, &'static str>,
    /// The locals that are parameters (S0064 says "a parameter" for them).
    params: std::collections::HashSet<LocalId>,
    /// Inside an expression of literals whose whole was fit-checked (A42C-2): its parts are not checked again.
    folding: bool,
    /// Part (d): a counter of binds, loop starts and waits, in walk order, and each local's tick when bound.
    tick: u32,
    bound_tick: HashMap<LocalId, u32>,
    /// Locals already said by S0104 (once each).
    across_said: std::collections::HashSet<LocalId>,
    /// The next closure walked is a built-in's closure argument (call.rs sets it just before).
    next_runs_now: bool,
    /// The `wait` that is a whole statement, a `let`'s, an assignment's or a condition's whole value (S0105).
    whole_wait: Option<ExprId>,
    /// The optional services (`use optional svc.<name>`, by alias) a test before this place showed given: an `if`'s
    /// or a `guard`'s `x != none`, or an `if x == none` whose block always leaves. Each scope's start is marked in
    /// `given_marks`; its end forgets what it learned. A service given at load stays given, so no wait undoes it.
    given: Vec<String>,
    given_marks: Vec<usize>,
}

/// Part (c) over the mod's modules. Libraries are read only where the mod uses them (a module `let`'s type).
pub(super) fn bodies(c: &mut Checker<'_>) {
    c.game_values = game_values(c);
    let modules = c.modules;
    // one `on frame`, `start`, `stop` and `mods.stopped` a module (a kit module ticks
    // itself; frames and starts run in module order, stops in reverse); one `on load`, `layers` and `step` a mod, over
    // all its modules (S0109).
    let mut first: HashMap<(&'static str, Option<usize>), (usize, Pos)> = HashMap::new();
    for (m, source) in modules.iter().enumerate() {
        for item in &source.module.items {
            let ItemKind::On(on) = &item.kind else { continue };
            let what = match on.event {
                ast::OnEvent::Frame => ("frame", Some(m)),
                ast::OnEvent::Start => ("start", Some(m)),
                ast::OnEvent::Stop => ("stop", Some(m)),
                ast::OnEvent::Event { .. } if on.event.is_kit_stopped() => ("mods.stopped", Some(m)),
                ast::OnEvent::Load => ("load", None),
                ast::OnEvent::Layers(_) => ("layers", None),
                ast::OnEvent::Step(_) => ("step", None),
                _ => continue,
            };
            match first.get(&what) {
                Some((fm, at)) => {
                    let place = format!("{}:{}", modules[*fm].path, at.line);
                    let d = Diagnostic::new(code::ON_TWICE, on.at).with("what", Arg::Text(what.0.into())).with("first", Arg::Text(place));
                    c.say(ModRef::of_mod(m), d);
                }
                None => {
                    first.insert(what, (m, on.at));
                }
            }
        }
    }
    for (m, source) in modules.iter().enumerate() {
        let mut w = Walk::new(c, ModRef::of_mod(m));
        for (i, item) in source.module.items.iter().enumerate() {
            w.item(i, item);
        }
        w.finish();
    }
}

/// Every name a script may write for a game enum's value.
fn game_values(c: &Checker<'_>) -> HashMap<String, GameValue> {
    let mut out = HashMap::new();
    let Some(types) = c.facts.game.types.as_ref() else { return out };
    let mut names: Vec<&String> = types.enums.keys().collect();
    names.sort();
    for name in names {
        let e = &types.enums[name];
        let scope = name.rsplit_once("::").map(|(s, _)| s);
        for (value, number) in &e.values {
            let v = GameValue { enum_name: name.clone(), value: value.clone(), number: *number };
            out.entry(format!("{name}::{value}")).or_insert_with(|| v.clone());
            if !e.scoped {
                let bare = match scope {
                    Some(s) => format!("{s}::{value}"),
                    None => value.clone(),
                };
                out.entry(bare).or_insert(v);
            }
        }
    }
    for e in &types.anonymous_enums {
        for (value, number) in &e.values {
            out.entry(value.clone()).or_insert_with(|| GameValue { enum_name: String::new(), value: value.clone(), number: *number });
        }
    }
    out
}

impl<'w, 'a> Walk<'w, 'a> {
    fn new(c: &'w mut Checker<'a>, m: ModRef) -> Self {
        Walk {
            c,
            m,
            record: m.owner == Owner::Mod,
            frames: Vec::new(),
            tys: HashMap::new(),
            res: HashMap::new(),
            named: HashMap::new(),
            game_places: Default::default(),
            captured: Default::default(),
            read_only: HashMap::new(),
            params: Default::default(),
            folding: false,
            tick: 0,
            bound_tick: HashMap::new(),
            across_said: Default::default(),
            next_runs_now: false,
            whole_wait: None,
            given: Vec::new(),
            given_marks: Vec::new(),
        }
    }

    /// What this walk found, into [`super::Checked`] (the mod's modules only).
    fn finish(self) {
        if !self.record {
            return;
        }
        let module = self.m.module;
        let out = &mut self.c.out;
        out.exprs.extend(self.tys.into_iter().map(|(id, t)| ((module, id), t)));
        out.resolved.extend(self.res.into_iter().map(|(id, r)| ((module, id), r)));
        out.named.extend(self.named.into_iter().map(|(at, r)| ((module, at), r)));
    }

    fn say(&mut self, d: Diagnostic) {
        self.c.say(self.m, d);
    }

    fn module_of(&self, m: ModRef) -> &'a ast::Module {
        let (modules, libraries) = (self.c.modules, self.c.libraries);
        match m.owner {
            Owner::Mod => &modules[m.module].module,
            Owner::Library(l) => &libraries[l].modules[m.module].module,
        }
    }

    fn item_of(&self, item: ItemRef) -> &'a ast::Item {
        &self.module_of(item.module_ref()).items[item.item]
    }

    fn game(&self) -> &'a crate::game::Game {
        self.c.facts.game
    }

    // ------------------------------------------------------------ frames, scopes, locals

    fn frame(&self) -> &Frame {
        self.frames.last().expect("a body is walked inside a frame")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("a body is walked inside a frame")
    }

    fn scope_push(&mut self) {
        self.frame_mut().scopes.push(HashMap::new());
        self.given_marks.push(self.given.len());
    }

    fn scope_pop(&mut self) {
        self.frame_mut().scopes.pop();
        if let Some(n) = self.given_marks.pop() {
            self.given.truncate(n);
        }
    }

    /// The service `name` names here when it is an optional one (`use optional svc.<s>`, no local of the name).
    pub(super) fn optional_service(&self, name: &str) -> Option<String> {
        if self.find_local(name).is_some() {
            return None;
        }
        match self.c.scope(self.m).names.get(name) {
            Some(Name::Module { target: super::ModTarget::Service(s), optional: true, .. }) => Some(s.clone()),
            _ => None,
        }
    }

    /// The optional service aliases `e` shows given when it is `holds` (true: `x != none`, `a && b`; false:
    /// `x == none`, `a || b`; `!` turns it).
    fn given_by(&self, e: &Expr, holds: bool, out: &mut Vec<String>) {
        match &e.kind {
            ExprKind::Paren(x) => self.given_by(x, holds, out),
            ExprKind::Not(x) => self.given_by(x, !holds, out),
            ExprKind::Binary { op: ast::BinOp::And, left, right } if holds => {
                self.given_by(left, true, out);
                self.given_by(right, true, out);
            }
            ExprKind::Binary { op: ast::BinOp::Or, left, right } if !holds => {
                self.given_by(left, false, out);
                self.given_by(right, false, out);
            }
            ExprKind::Binary { op: op @ (ast::BinOp::Eq | ast::BinOp::Ne), left, right } if (*op == ast::BinOp::Ne) == holds => {
                let other = match (&left.kind, &right.kind) {
                    (ExprKind::None, _) => right,
                    (_, ExprKind::None) => left,
                    _ => return,
                };
                if let ExprKind::Name { name, .. } = &other.kind {
                    if self.optional_service(&name.name).is_some() {
                        out.push(name.name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// Whether the optional service alias `name` was shown given before this place.
    pub(super) fn is_given(&self, name: &str) -> bool {
        self.given.iter().any(|g| g == name)
    }

    fn locals(&self) -> &Vec<Local> {
        if self.record {
            &self.c.out.locals
        } else {
            &self.c.lib_locals
        }
    }

    fn local(&self, id: LocalId) -> &Local {
        &self.locals()[id as usize]
    }

    /// A new local in the innermost scope (a name may shadow an earlier one), and its binding place noted.
    fn bind(&mut self, name: &ast::Ident, ty: Ty, var: bool) -> LocalId {
        let local = Local { name: name.name.clone(), at: name.at, ty, var, module: self.m.module };
        let list = if self.record { &mut self.c.out.locals } else { &mut self.c.lib_locals };
        let id = list.len() as LocalId;
        list.push(local);
        let tick = self.next_tick();
        self.bound_tick.insert(id, tick);
        self.frame_mut().scopes.last_mut().expect("a frame has a scope").insert(name.name.clone(), id);
        self.named.insert(name.at, Resolved::Local(id));
        id
    }

    /// A local by name, innermost first, with whether it is a closure's copy (captured: read only, 5.4).
    fn find_local(&self, name: &str) -> Option<(LocalId, bool)> {
        let mut captured = false;
        for frame in self.frames.iter().rev() {
            for scope in frame.scopes.iter().rev() {
                if let Some(id) = scope.get(name) {
                    return Some((*id, captured));
                }
            }
            // A closure or a local `fn` sees the locals around it as copies; any other body sees none.
            if frame.kind != Kind::Closure {
                return None;
            }
            captured = true;
        }
        None
    }

    /// The names a mistyped name may have meant: locals, the module's names, the standard names.
    fn known_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for frame in &self.frames {
            for scope in &frame.scopes {
                out.extend(scope.keys().cloned());
            }
        }
        out.extend(self.c.scope(self.m).names.keys().cloned());
        out.extend(call::STD_NAMES.iter().map(|s| s.to_string()));
        if let Some(Members::Mod(class)) = self.frames.iter().rev().find_map(|f| f.members.clone()) {
            if let ItemKind::Class(cd) = &self.item_of(class).kind {
                for member in &cd.members {
                    match member {
                        ClassMember::Binding(b) => {
                            let mut idents = Vec::new();
                            super::pattern_names(&b.pattern, &mut idents);
                            out.extend(idents.into_iter().map(|i| i.name));
                        }
                        ClassMember::Fn(f) | ClassMember::Override(f) => out.push(f.name.name.clone()),
                        ClassMember::Hook(_) => {}
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn unknown_name(&mut self, at: Pos, name: &str) {
        let known = self.known_names();
        let near = nearest(name, known.iter().map(String::as_str));
        let d = if near.is_empty() {
            Diagnostic::new(code::UNKNOWN_NAME, at).with("found", Arg::Text(name.to_string()))
        } else {
            let near = near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
            Diagnostic::new(code::UNKNOWN_NAME_NEAR, at).with("found", Arg::Text(name.to_string())).with("near", Arg::Text(near))
        };
        self.say(d);
    }

    // ------------------------------------------------------------ flow (part d)

    fn next_tick(&mut self) -> u32 {
        let t = self.tick;
        self.tick += 1;
        t
    }

    /// S0103 for a value of type `t` named `found`, kept where `why` says.
    fn kept(&mut self, t: &Ty, found: &str, at: Pos, why: &'static str) {
        if holds_view(t) {
            self.say(Diagnostic::new(code::VIEW_KEPT, at).with("found", Arg::Text(found.to_string())).with("why", Arg::Phrase(why)));
        }
    }

    /// Whether reading the local `name` from here copies it into a closure that may run later: a closure frame between
    /// here and the body that holds it, other than a built-in's closure argument.
    fn capture_kept(&self, name: &str) -> bool {
        for frame in self.frames.iter().rev() {
            if frame.scopes.iter().any(|s| s.contains_key(name)) {
                return false;
            }
            if frame.kind != Kind::Closure {
                return false;
            }
            if !frame.runs_now {
                return true;
            }
        }
        false
    }

    /// Whether `self` (written or reached through a member) is read from a closure that may run later.
    fn self_kept(&self) -> bool {
        for frame in self.frames.iter().rev() {
            if frame.kind != Kind::Closure {
                return false;
            }
            if !frame.runs_now {
                return true;
            }
        }
        false
    }

    /// `self`, written or through a member, read here: S0103 when a closure that may run later would keep its view.
    pub(super) fn self_read(&mut self, at: Pos) {
        let Some(t) = self.frames.iter().rev().find_map(|f| f.self_ty.clone()) else { return };
        if self.self_kept() {
            self.kept(&t, "self", at, "Script.Keep.Closure");
        }
    }

    /// A local read at `at` (part d): a view copied into a closure that may run later (S0103), or read after a `wait`
    /// it was made before (S0104), now or the next time round a loop.
    pub(super) fn local_read(&mut self, id: LocalId, captured: bool, at: Pos) {
        let (name, ty) = {
            let l = self.local(id);
            (l.name.clone(), l.ty.clone())
        };
        // Any local read after a `wait` it was bound before, in the body that binds it (part e: a task's field).
        if self.record && !captured {
            let bound = self.bound_tick.get(&id).copied().unwrap_or(0);
            let holder = self.frames.iter().rposition(|f| f.scopes.iter().any(|s| s.values().any(|v| *v == id)));
            if holder.is_some_and(|h| self.frames[h].path.waits.iter().any(|(t, _)| *t > bound)) {
                self.c.out.across_wait.insert(id);
            }
        }
        if !holds_view(&ty) {
            return;
        }
        if captured && self.capture_kept(&name) {
            self.kept(&ty, &name, at, "Script.Keep.Closure");
            return;
        }
        // A built-in's closure argument runs where it is written: its read of a local around it is a read in the body
        // that holds the local, against that body's waits and loops (A42D-1).
        let holder = self.frames.iter().rposition(|f| f.scopes.iter().any(|s| s.values().any(|v| *v == id))).unwrap_or(self.frames.len() - 1);
        let bound = self.bound_tick.get(&id).copied().unwrap_or(0);
        let frame = &mut self.frames[holder];
        if frame.later_from.is_some_and(|since| bound < since) {
            self.kept(&ty, &name, at, "Script.Keep.State");
            return;
        }
        let crossed = frame.path.waits.iter().find(|(t, _)| *t > bound).map(|(_, p)| *p);
        match crossed {
            Some(w) => self.across(id, &name, at, w),
            None => {
                if let Some(mark) = frame.loop_marks.last_mut() {
                    mark.uses.push((id, at));
                }
            }
        }
    }

    fn across(&mut self, id: LocalId, name: &str, at: Pos, wait: Pos) {
        if self.across_said.insert(id) {
            let d = Diagnostic::new(code::VIEW_ACROSS_WAIT, at).with("found", Arg::Text(name.to_string())).with("line", Arg::Text(wait.line.to_string()));
            self.say(d);
        }
    }

    /// A loop starts: its mark.
    fn loop_start(&mut self) {
        let start = self.next_tick();
        let entry = self.frame().path.clone();
        self.frame_mut().loop_marks.push(LoopMark { start, broke: false, uses: Vec::new(), entry, rounds: None, exits: None });
    }

    /// The place where the loop may end without a `break` (after a `while`'s condition, at a `for`'s start): its path.
    fn loop_entry(&mut self) {
        let path = self.frame().path.clone();
        if let Some(mark) = self.frame_mut().loop_marks.last_mut() {
            mark.entry = path;
        }
    }

    /// `break` or `continue`: this path goes with it to the loop's end or round again.
    fn loop_leave(&mut self, is_break: bool) {
        let path = self.frame().path.clone();
        if let Some(mark) = self.frame_mut().loop_marks.last_mut() {
            if is_break {
                mark.broke = true;
                Path::join(&mut mark.exits, &path);
            } else {
                Path::join(&mut mark.rounds, &path);
            }
        }
    }

    /// A loop ends (its body walked; `body_leaves` when the body's end is never reached). A view made before it and
    /// read in it is read again after a `wait` on a path back round (S0104). Its reads go to the loop around it. What
    /// follows it has the waits of every way out: the condition, `break`. Gives whether a `break` leaves it, and the
    /// first wait on a path back round.
    fn loop_end(&mut self, body_leaves: bool) -> (bool, Option<Pos>) {
        let mark = self.frame_mut().loop_marks.pop().expect("a loop was started");
        let mut round = mark.rounds.clone();
        if !body_leaves {
            let end = self.frame().path.clone();
            Path::join(&mut round, &end);
        }
        let first_wait = round.as_ref().and_then(|r| r.waits.iter().find(|(t, _)| *t > mark.start).map(|(_, p)| *p));
        // A `proceed()` in the loop on a path back round runs again the next time round (S0122).
        if let (Some(p), None) = (round.as_ref().and_then(|r| r.proceeded), mark.entry.proceeded) {
            self.say(Diagnostic::new(code::PROCEED_TWICE, p).with("line", Arg::Text(p.line.to_string())));
        }
        if let Some(w) = first_wait {
            for (id, at) in &mark.uses {
                if self.bound_tick.get(id).copied().unwrap_or(0) < mark.start {
                    let name = self.local(*id).name.clone();
                    self.across(*id, &name, *at, w);
                }
            }
        }
        let mut after = Some(mark.entry.clone());
        if let Some(r) = &round {
            Path::join(&mut after, r);
        }
        if let Some(x) = &mark.exits {
            Path::join(&mut after, x);
        }
        let frame = self.frame_mut();
        frame.path = after.expect("the entry's path");
        if let Some(outer) = frame.loop_marks.last_mut() {
            outer.uses.extend(mark.uses);
        }
        (mark.broke, first_wait)
    }

    /// The waits on this path from tick `from` on.
    fn waited_from(&self, from: u32) -> Option<Pos> {
        self.frame().path.waits.iter().find(|(t, _)| *t >= from).map(|(_, p)| *p)
    }

    /// Branches (an `if`'s arms, a `match`'s): each starts from `base`; what follows has the waits of the arms that go
    /// on, or `base` when none does (A42D-3).
    pub(super) fn arm_waits(&mut self, base: &Path) {
        self.frame_mut().path = base.clone();
    }

    /// The path waits now, for [`Walk::arm_waits`] and [`Walk::arms_joined`].
    pub(super) fn path_waits(&self) -> Path {
        self.frame().path.clone()
    }

    /// After every arm: the joined paths of those that go on (None: none did, so `base`).
    pub(super) fn arms_joined(&mut self, reach: Option<Path>, base: Path) {
        self.frame_mut().path = reach.unwrap_or(base);
    }

    /// A `wait` (part d): where it may be (S0105), inside `defer` (S0106); recorded for S0104.
    pub(super) fn wait_here(&mut self, e: &Expr) {
        let whole = self.whole_wait == Some(e.id);
        if whole {
            self.whole_wait = None;
        }
        let (ok, in_defer) = (self.frame().waits_ok, !self.frame().defers.is_empty());
        if in_defer {
            self.say(Diagnostic::new(code::DEFER_LEAVES, e.at).with("found", Arg::Text("wait".into())));
        } else if !ok {
            self.say(Diagnostic::new(code::WAIT_HERE, e.at).with("why", Arg::Phrase("Script.Wait.Where")));
        } else if !whole {
            self.say(Diagnostic::new(code::WAIT_HERE, e.at).with("why", Arg::Phrase("Script.Wait.Inside")));
        }
        if ok {
            let t = self.next_tick();
            self.frame_mut().path.waits.push((t, e.at));
        }
    }

    /// The value of a statement, a `let`, an assignment or a condition: a `wait` that is the first thing it reads
    /// (`wait p`, `wait p ?? ""`, `(wait p).name`, `wait p as int`) waits before anything of it is held (S0105).
    fn whole_value(&mut self, e: &Expr) {
        let inner = first_read(e);
        if matches!(inner.kind, ExprKind::Wait { .. } | ExprKind::WaitUntil(_)) {
            self.whole_wait = Some(inner.id);
        }
    }

    /// A statement that leaves (`return`, `stop`, `go`, `skip`; `break` or `continue` of a loop outside it) inside
    /// `defer` (S0106).
    fn leaves_defer(&mut self, at: Pos, word: &str, loop_word: bool) {
        let frame = self.frame();
        let Some(&loops) = frame.defers.last() else { return };
        if loop_word && frame.loops > loops {
            return;
        }
        self.say(Diagnostic::new(code::DEFER_LEAVES, at).with("found", Arg::Text(word.to_string())));
    }

    /// S0101 when a body with a result can reach its end.
    fn returns(&mut self, ret: &Option<Ty>, end: &Ty, at: Pos) {
        let Some(r) = ret else { return };
        if matches!(r, Ty::Unit | Ty::Error | Ty::Never) || *end == Ty::Never {
            return;
        }
        let expected = self.ty_text(r);
        self.say(Diagnostic::new(code::MISSING_RETURN, at).with("expected", Arg::Text(expected)));
    }

    // ------------------------------------------------------------ items

    fn item(&mut self, i: usize, item: &'a ast::Item) {
        let me = self.m.item(i);
        match &item.kind {
            ItemKind::Binding(_) => self.module_binding(me),
            ItemKind::Fn(f) => self.function(me, "", f, None, None),
            ItemKind::Export { params, ret, body, .. } => {
                let r = ret.as_ref().map(|_| self.declared(me, "->"));
                self.body_with_params(Kind::Fn, r, me, "", params, body, None, None);
            }
            ItemKind::Struct(s) => {
                for f in &s.fields {
                    let ty = self.declared(me, &format!(".{}", f.name.name));
                    self.kept(&ty, &f.name.name, f.name.at, "Script.Keep.Field");
                    if let Some(d) = &f.default {
                        self.init_value(d, &ty, None);
                    }
                }
                for f in &s.fns {
                    self.function(me, &format!(".{}", f.name.name), f, Some(Ty::Struct(me)), None);
                }
            }
            ItemKind::Enum(e) => {
                for case in &e.cases {
                    for f in &case.fields {
                        let ty = self.declared(me, &format!("{}.{}", case.name.name, f.name.name));
                        self.kept(&ty, &f.name.name, f.name.at, "Script.Keep.Field");
                        if let Some(d) = &f.default {
                            self.init_value(d, &ty, None);
                        }
                    }
                }
            }
            ItemKind::Class(cd) => self.class(me, cd),
            // A task with a result: its `return v` gives it, checked as a function's on every path (11.1a).
            ItemKind::Task { params, body, ret, .. } => {
                let r = ret.as_ref().map(|_| self.declared(me, "->"));
                self.body_with_params(Kind::Task, r, me, "", params, body, None, None)
            }
            ItemKind::Hook(h) => self.hook(me, "", h, None),
            ItemKind::On(on) => self.on(on),
            ItemKind::States(s) => self.states(me, s),
            ItemKind::Test { game, body, .. } => {
                let mut frame = Frame::new(Kind::Test, None);
                frame.waits_ok = *game;
                self.frames.push(frame);
                self.block(body, Want::Drop);
                self.frames.pop();
            }
            ItemKind::Use(_) | ItemKind::Option { .. } | ItemKind::Extern { .. } => {}
        }
    }

    /// A type part (b) wrote for an item's part; `Ty::Error` when (b) could not (it said why).
    fn declared(&self, item: ItemRef, key: &str) -> Ty {
        self.c.out.types.get(&(item, key.to_string())).cloned().unwrap_or(Ty::Error)
    }

    /// A value with its declared type, outside any body (a field's default, a parameter's default).
    fn init_value(&mut self, value: &'a Expr, ty: &Ty, members: Option<Members>) {
        let mut frame = Frame::new(Kind::Init, None);
        frame.members = members;
        self.frames.push(frame);
        self.expr(value, Want::Ty(ty));
        self.frames.pop();
    }

    /// A function of the module (`key` ""), of a struct (`.name`, `self_ty` the struct) or of a class.
    fn function(&mut self, owner: ItemRef, key: &str, f: &'a ast::FnDecl, self_ty: Option<Ty>, members: Option<Members>) {
        let ret = f.ret.as_ref().map(|_| self.declared(owner, &format!("{key}->")));
        self.body_with_params(Kind::Fn, ret, owner, key, &f.params, &f.body, self_ty, members);
    }

    #[allow(clippy::too_many_arguments)]
    fn body_with_params(&mut self, kind: Kind, ret: Option<Ty>, owner: ItemRef, key: &str, params: &'a [ast::Param], body: &'a Block, self_ty: Option<Ty>, members: Option<Members>) {
        // Defaults first, outside the body (they see the module, not the parameters).
        for (i, p) in params.iter().enumerate() {
            if let Some(d) = &p.default {
                let ty = self.declared(owner, &param_key(key, p, i));
                self.init_value(d, &ty, members.clone());
            }
        }
        let mut frame = Frame::new(kind, ret.clone());
        frame.members = members;
        frame.self_ty = self_ty;
        frame.waits_ok = kind == Kind::Task;
        self.frames.push(frame);
        for (i, p) in params.iter().enumerate() {
            if p.is_self {
                self.frame_mut().self_var = p.var;
                if self.frame().self_ty.is_none() {
                    self.say(Diagnostic::new(code::SELF_OUTSIDE, p.at));
                }
                continue;
            }
            let ty = match &p.ty {
                Some(_) => self.declared(owner, &param_key(key, p, i)),
                None => Ty::Error,
            };
            if let Some(name) = &p.name {
                // A task runs on later frames: what it is given must outlive the statement that starts it.
                if kind == Kind::Task {
                    self.kept(&ty, &name.name, name.at, "Script.Keep.Task");
                }
                // A hook's parameters may be written (`wrap`, section 8); which hooks may is part (e)'s.
                let var = p.var || kind == Kind::Hook;
                let id = self.bind(name, ty, var);
                self.params.insert(id);
            }
        }
        let end = self.block(body, Want::Drop);
        self.returns(&ret, &end, body.end);
        self.frames.pop();
    }

    /// A module-level `let` or `var`: its value against its type, or its type found from its value (once).
    fn module_binding(&mut self, item: ItemRef) {
        let key = (item, "=".to_string());
        if self.c.inferred.contains_key(&key) || self.c.inferring.contains(&key) {
            return;
        }
        let ItemKind::Binding(b) = &self.item_of(item).kind else { return };
        self.c.inferring.insert(key.clone());
        let declared = self.c.out.types.get(&(item, String::new())).cloned();
        let mut w = Walk::new(&mut *self.c, item.module_ref());
        w.frames.push(Frame::new(Kind::Init, None));
        let ty = match (&declared, &b.value) {
            (Some(t), Some(v)) => {
                w.expr(v, Want::Ty(t));
                t.clone()
            }
            (Some(t), None) => t.clone(),
            (None, Some(v)) => {
                let t = w.expr(v, Want::Any);
                w.stored(t, v.at)
            }
            (None, None) => {
                w.say(Diagnostic::new(code::TYPE_NEEDS_WRITING, b.pattern.at).with("found", Arg::Text(pattern_text(&b.pattern))));
                Ty::Error
            }
        };
        let mut names = Vec::new();
        w.module_pattern(&b.pattern, &ty, item, &mut names);
        for (name, t) in &names {
            w.kept(t, name, b.pattern.at, "Script.Keep.Module");
        }
        w.finish();
        self.c.inferring.remove(&key);
        for (name, t) in names {
            self.c.inferred.insert((item, format!("={name}")), t);
        }
        self.c.inferred.insert(key, Ty::Unit);
    }

    /// A module-level pattern: names and tuples of names (anything else may not match).
    fn module_pattern(&mut self, p: &'a ast::Pattern, ty: &Ty, item: ItemRef, out: &mut Vec<(String, Ty)>) {
        match &p.kind {
            ast::PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => {
                self.named.insert(path[0].at, Resolved::Item(item));
                out.push((path[0].name.clone(), ty.clone()));
            }
            ast::PatternKind::Tuple(items) => {
                let tys: Vec<Ty> = match ty {
                    Ty::Tuple(ts) if ts.len() == items.len() => ts.clone(),
                    Ty::Error => vec![Ty::Error; items.len()],
                    other => {
                        let found = self.ty_text(other);
                        let expected = format!("({})", vec!["_"; items.len()].join(", "));
                        self.say(Diagnostic::new(code::TYPE_MISMATCH, p.at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                        vec![Ty::Error; items.len()]
                    }
                };
                for (sub, t) in items.iter().zip(tys.iter()) {
                    self.module_pattern(sub, t, item, out);
                }
            }
            _ => self.say(Diagnostic::new(code::REFUTABLE_PATTERN, p.at)),
        }
    }

    /// The type of a module-level name of a `let` or `var` (found from its value the first time).
    fn module_name_ty(&mut self, item: ItemRef, name: &str, at: Pos) -> Ty {
        let key = (item, format!("={name}"));
        if let Some(t) = self.c.inferred.get(&key) {
            return t.clone();
        }
        if self.c.inferring.contains(&(item, "=".to_string())) {
            self.say(Diagnostic::new(code::TYPE_NEEDS_WRITING, at).with("found", Arg::Text(name.to_string())));
            return Ty::Error;
        }
        self.module_binding(item);
        self.c.inferred.get(&key).cloned().unwrap_or(Ty::Error)
    }

    /// A class: its fields' values, functions, overrides and hooks, each with the class's members in reach.
    fn class(&mut self, me: ItemRef, cd: &'a ast::ClassDecl) {
        for (n, member) in cd.members.iter().enumerate() {
            let members = Some(Members::Mod(me));
            match member {
                ClassMember::Binding(b) => {
                    let mut idents = Vec::new();
                    super::pattern_names(&b.pattern, &mut idents);
                    for ident in idents {
                        let t = self.class_field_ty(me, &ident.name, ident.at);
                        self.kept(&t, &ident.name, ident.at, "Script.Keep.Field");
                    }
                    let _ = b;
                }
                ClassMember::Fn(f) => self.function(me, &format!(".{}", f.name.name), f, Some(Ty::Class(me)), members),
                ClassMember::Override(f) => self.override_fn(me, &format!(".{}", f.name.name), f),
                ClassMember::Hook(h) => self.hook(me, &format!(".hook{n}"), h, Some(me)),
            }
        }
    }

    /// `override fn` (docs/modding/language/language.md, section 5.5): its parameters and result are the virtual's it replaces (a type written must be
    /// the same; check.rs said when it replaces none); its body runs on the class's own objects as a wrap of the
    /// virtual's row, which out.hooks records for the back end (key `.<name>`), and `base.<name>(...)` runs the
    /// base's own.
    fn override_fn(&mut self, class: ItemRef, key: &str, f: &'a ast::FnDecl) {
        let arity = f.params.iter().filter(|p| !p.is_self).count();
        let row = self.class_base(class).and_then(|b| self.game().virtual_of(&b, &f.name.name, Some(arity)));
        let fnty = row.as_ref().and_then(|(_, sig)| self.game().hook_signature(sig).ok());
        if let (Some((_, sig)), None) = (&row, &fnty) {
            self.say(Diagnostic::new(code::GAME_NONE_CROSSING, f.name.at).with("found", Arg::Text(f.name.name.clone())).with("signatures", Arg::Text(sig.clone())));
        }
        let ret = match (&fnty, &f.ret) {
            (Some(t), _) => match &t.result {
                GameTy::Unit => None,
                GameTy::Place { of, .. } => Some(expr::from_game_place(of)),
                other => Some(expr::from_game_place(other)),
            },
            (None, Some(_)) => Some(self.declared(class, &format!("{key}->"))),
            (None, None) => None,
        };
        if let (Some(r), Some(t)) = (&f.ret, &fnty) {
            let written = self.declared(class, &format!("{key}->"));
            let want = ret.clone().unwrap_or(Ty::Unit);
            if t.result != GameTy::Unit || written != Ty::Unit {
                if !matches!(written, Ty::Error) && !matches!(want, Ty::Error) && !written_fits(&written, &want) {
                    let (fw, e) = (self.ty_text(&written), self.ty_text(&want));
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, r.at).with("found", Arg::Text(fw)).with("expected", Arg::Text(e)));
                }
            }
        }
        let params: Vec<&'a ast::Param> = f.params.iter().filter(|p| !p.is_self).collect();
        if let Some(t) = &fnty {
            if t.params.len() != params.len() {
                let d = Diagnostic::new(code::OVERRIDE_PARAMS, f.name.at).with("name", Arg::Text(f.name.name.clone())).with("found", Arg::Text(params.len().to_string())).with("expected", Arg::Text(t.params.len().to_string()));
                self.say(d);
            }
        }
        let mut frame = Frame::new(Kind::Fn, ret.clone());
        frame.self_ty = Some(Ty::Class(class));
        frame.members = Some(Members::Mod(class));
        let mut types = Vec::new();
        let mut binds = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            if p.is_self {
                continue;
            }
            let k = types.len();
            let g = fnty.as_ref().and_then(|t| t.params.get(k)).cloned();
            let (ty, var) = match &g {
                Some(g) => {
                    let (t, _, nullable) = call::game_param(g);
                    let t = if nullable { Ty::Opt(Box::new(t)) } else { t };
                    (t, !matches!(g, GameTy::Place { konst: true, .. }))
                }
                None => (if p.ty.is_some() { self.declared(class, &param_key(key, p, i)) } else { Ty::Error }, true),
            };
            if p.ty.is_some() && g.is_some() {
                let written = self.declared(class, &param_key(key, p, i));
                if !matches!(written, Ty::Error) && !matches!(ty, Ty::Error) && !written_fits(&written, &ty) {
                    let (fw, e) = (self.ty_text(&written), self.ty_text(&ty));
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, p.at).with("found", Arg::Text(fw)).with("expected", Arg::Text(e)));
                }
            }
            types.push(ty.clone());
            binds.push((p, ty, var));
        }
        frame.override_of = Some(OverrideCx { name: f.name.name.clone(), params: types, ret: ret.clone() });
        self.frames.push(frame);
        for (p, ty, var) in binds {
            if let Some(name) = &p.name {
                let id = self.bind(name, ty, var);
                self.params.insert(id);
            }
        }
        let end = self.block(&f.body, Want::Drop);
        self.returns(&ret, &end, f.body.end);
        self.frames.pop();
        if let (true, Some((name, signature))) = (self.record, row) {
            self.c.out.hooks.insert((class, key.to_string()), HookTarget { name, signature, overrides: Vec::new() });
            // Its result's type, the virtual's, for the back end (`return v`).
            if let Some(r) = ret {
                self.c.out.types.insert((class, format!("{key}->")), r);
            }
        }
    }

    /// A class field's type: as written, or found from its value (once); its value checked once either way.
    fn class_field_ty(&mut self, class: ItemRef, name: &str, at: Pos) -> Ty {
        let mark = (class, format!(".{name}="));
        let written = self.c.out.types.get(&(class, format!(".{name}"))).cloned();
        if self.c.inferred.contains_key(&mark) {
            return written.or_else(|| self.c.inferred.get(&(class, format!(".{name}"))).cloned()).unwrap_or(Ty::Error);
        }
        if self.c.inferring.contains(&mark) {
            self.say(Diagnostic::new(code::TYPE_NEEDS_WRITING, at).with("found", Arg::Text(name.to_string())));
            return Ty::Error;
        }
        let ItemKind::Class(cd) = &self.item_of(class).kind else { return Ty::Error };
        let Some(b) = cd.members.iter().find_map(|m| match m {
            ClassMember::Binding(b) => {
                let mut idents = Vec::new();
                super::pattern_names(&b.pattern, &mut idents);
                idents.iter().any(|i| i.name == name).then_some(b)
            }
            _ => None,
        }) else {
            return Ty::Error;
        };
        self.c.inferring.insert(mark.clone());
        let mut w = Walk::new(&mut *self.c, class.module_ref());
        w.frames.push(Frame::new(Kind::Init, None));
        let ty = match (&written, &b.value) {
            (Some(t), Some(v)) => {
                w.expr(v, Want::Ty(t));
                t.clone()
            }
            (Some(t), None) => t.clone(),
            (None, Some(v)) => {
                let t = w.expr(v, Want::Any);
                w.stored(t, v.at)
            }
            (None, None) => {
                w.say(Diagnostic::new(code::TYPE_NEEDS_WRITING, at).with("found", Arg::Text(name.to_string())));
                Ty::Error
            }
        };
        if !matches!(b.pattern.kind, ast::PatternKind::Path { .. }) {
            w.say(Diagnostic::new(code::REFUTABLE_PATTERN, b.pattern.at));
        } else {
            w.named.insert(at, Resolved::Member { owner: class, name: name.to_string() });
        }
        w.finish();
        self.c.inferring.remove(&mark);
        if written.is_none() {
            self.c.inferred.insert((class, format!(".{name}")), ty.clone());
            // The back end's type of it (the class's data).
            self.c.out.types.insert((class, format!(".{name}")), ty.clone());
        }
        self.c.inferred.insert(mark, Ty::Unit);
        ty
    }

    /// A hook (top-level, or in a class: `class_of`), part (e). Its head chooses the game function's row
    /// ([`Walk::hook_row`]); the row's facts say whether it has a `this` and what it is, its signature what the
    /// parameters and the result are. `self` is the class's object in a class, else a view of the record the function
    /// is a member of. A `return` in a hook has no value (its result is `result`, `skip` or `proceed()`).
    fn hook(&mut self, owner: ItemRef, key: &str, h: &'a ast::Hook, class_of: Option<ItemRef>) {
        let chosen = self.hook_row(owner, key, h, class_of);
        let at = h.target.at;
        let mut fnty: Option<crate::game::FnTy> = None;
        let mut result: Option<Result<Ty, &'static str>> = Some(Ok(Ty::Error));
        let mut facts: Option<luma_validate::names::Facts> = None;
        let mut tell = false;
        let mut name = h.target.text.split('(').next().unwrap_or("").to_string();
        if let Some((n, row, is_tell)) = &chosen {
            name = n.clone();
            tell = *is_tell;
            match self.game().hook_signature(row) {
                Ok(f) => {
                    // A parameter that does not cross is bound as `_` (or the list left out), never by a name.
                    for (i, why) in &f.opaque {
                        if h.params.as_ref().and_then(|ps| ps.get(*i)).is_some_and(|p| p.name.is_some()) {
                            let d = Diagnostic::new(code::GAME_PARAM_NOT_CROSSING, at).with("found", Arg::Text(n.clone())).with("param", Arg::Text((i + 1).to_string())).with("why", Arg::Phrase(crate::check::why_phrase(why)));
                            self.say(d);
                        }
                    }
                    result = match &f.result {
                        GameTy::Unit => None,
                        GameTy::Place { of, .. } => Some(Ok(expr::from_game_place(of))),
                        other => Some(Ok(expr::from_game_place(other))),
                    };
                    fnty = Some(f);
                }
                Err(crate::game::SigNotCrossing { param: Some(p), why }) => {
                    // The script names the parameters: each must cross; with no list it reads none of them.
                    if h.params.is_some() {
                        let d = Diagnostic::new(code::GAME_PARAM_NOT_CROSSING, at).with("found", Arg::Text(n.clone())).with("param", Arg::Text((p + 1).to_string())).with("why", Arg::Phrase(crate::check::why_phrase(&why)));
                        self.say(d);
                    }
                }
                Err(crate::game::SigNotCrossing { param: None, why }) if !matches!(why, crate::game::NotCrossing::Variadic | crate::game::NotCrossing::Unreadable(_)) => {
                    result = Some(Err(crate::check::why_phrase(&why)));
                }
                Err(_) => {
                    self.say(Diagnostic::new(code::GAME_NONE_CROSSING, at).with("found", Arg::Text(n.clone())).with("signatures", Arg::Text(row.clone())));
                }
            }
            facts = match self.game().facts.as_ref() {
                Some(ff) => {
                    let f = ff.get(n, row).cloned();
                    if f.is_none() && self.c.noted.insert(format!("{n}{row}")) {
                        self.say(Diagnostic::new(code::NOT_DESCRIBED, at).with("found", Arg::Text(format!("{n}{row}"))).with("what", Arg::Phrase("Script.What.FunctionKinds")));
                    }
                    f
                }
                None => {
                    if self.c.noted.insert(n.clone()) {
                        self.say(Diagnostic::new(code::NOT_DESCRIBED, at).with("found", Arg::Text(n.clone())).with("what", Arg::Phrase("Script.What.FunctionKinds")));
                    }
                    None
                }
            };
        }
        use luma_validate::names::FunctionKind as FK;
        let kind_of = facts.as_ref().map(|f| f.kind);
        let is_tell = tell || matches!(kind_of, Some(FK::Constructor | FK::Destructor));
        if is_tell {
            result = None;
            if matches!(h.kind, ast::HookKind::Wrap | ast::HookKind::Replace) {
                self.say(Diagnostic::new(code::HOOK_TELL, h.at).with("found", Arg::Text(name.clone())));
            }
        }
        // `every override of`: a virtual, and each override the facts list (directly or through another).
        let mut overrides = Vec::new();
        if let (true, Some(f), Some((n, row, _))) = (h.every_override, &facts, &chosen) {
            if !f.is_virtual {
                self.say(Diagnostic::new(code::HOOK_NOT_VIRTUAL, at).with("found", Arg::Text(n.clone())));
            } else {
                overrides = self.overrides_of(n, row);
            }
        }
        // `this`: the record the function is a member of, by the facts (else by the name's qualifier).
        let record = match (&facts, &chosen) {
            (Some(f), _) => match f.kind {
                FK::Method | FK::Constructor | FK::Destructor => f.owner.clone(),
                FK::Free | FK::Static => None,
            },
            (None, Some(_)) | (None, None) => self.hook_record(&h.target),
        };
        let has_this = match &facts {
            Some(f) => matches!(f.kind, FK::Method | FK::Constructor | FK::Destructor),
            None => record.is_some(),
        };
        let (self_ty, members) = match class_of {
            Some(c) => {
                // The class's hooks run on its objects only (the `is_a` test): `this` is one of them.
                let base = self.class_base(c);
                let of_class = has_this && matches!((&base, &record), (Some(b), Some(r)) if self.derives(b, r));
                if chosen.is_some() && !of_class && (facts.is_some() || record.is_some()) {
                    let class_name = match &self.item_of(c).kind {
                        ItemKind::Class(cd) => cd.name.name.clone(),
                        _ => String::new(),
                    };
                    self.say(Diagnostic::new(code::HOOK_NOT_OF_CLASS, at).with("found", Arg::Text(name.clone())).with("class", Arg::Text(class_name)));
                }
                (Some(Ty::Class(c)), Some(Members::Mod(c)))
            }
            None => match (&record, has_this) {
                (Some(r), true) => {
                    let konst = facts.as_ref().is_some_and(|f| f.konst);
                    (Some(Ty::Game(GameTy::View { record: r.clone(), konst })), Some(Members::Game(r.clone())))
                }
                _ => (None, None),
            },
        };
        let self_gone = match (kind_of, h.kind) {
            (Some(FK::Constructor), ast::HookKind::Before) => Some("Script.Self.NotYet"),
            (Some(FK::Destructor), ast::HookKind::After) => Some("Script.Self.Gone"),
            _ => None,
        };
        // Its result's name or type, against the row's.
        if let Some(ret) = &h.ret {
            let ret_at = match ret {
                ast::HookRet::Name(n) => n.at,
                ast::HookRet::Type(t) => t.at,
            };
            match (&result, ret) {
                (None, _) if chosen.is_some() => {
                    self.say(Diagnostic::new(code::HOOK_NO_RESULT, ret_at).with("found", Arg::Text(name.clone())));
                }
                (Some(Err(why)), _) => {
                    self.say(Diagnostic::new(code::GAME_RESULT_NOT_CROSSING, ret_at).with("found", Arg::Text(name.clone())).with("why", Arg::Phrase(why)));
                    result = Some(Ok(Ty::Error));
                }
                (Some(Ok(t)), ast::HookRet::Type(_)) => {
                    let written = self.declared(owner, &format!("{key}->"));
                    if !matches!(written, Ty::Error) && !matches!(t, Ty::Error) && !written_fits(&written, t) {
                        let (f, e) = (self.ty_text(&written), self.ty_text(t));
                        self.say(Diagnostic::new(code::TYPE_MISMATCH, ret_at).with("found", Arg::Text(f)).with("expected", Arg::Text(e)));
                        // One mistake said once: the body's `skip` and `result` are not checked against either type.
                        result = Some(Ok(Ty::Error));
                    }
                }
                _ => {}
            }
        }
        // A wrap or replace gives the result itself: its paths are walked for it (S0123).
        if matches!(h.kind, ast::HookKind::Wrap | ast::HookKind::Replace) && matches!(result, Some(Err(_))) {
            if let Some(Err(why)) = result {
                self.say(Diagnostic::new(code::GAME_RESULT_NOT_CROSSING, at).with("found", Arg::Text(name.clone())).with("why", Arg::Phrase(why)));
            }
            result = Some(Ok(Ty::Error));
        }
        let mut frame = Frame::new(Kind::Hook, None);
        frame.self_ty = self_ty;
        frame.members = members;
        frame.self_gone = self_gone;
        frame.path.given = !matches!(h.kind, ast::HookKind::Wrap | ast::HookKind::Replace) || result.is_none();
        frame.hook = Some(HookCx { kind: h.kind, name: name.clone(), result: result.clone(), not_yet_said: false });
        self.frames.push(frame);
        if let Some(params) = &h.params {
            let expected = fnty.as_ref().map(|f| f.params.len());
            if let Some(n) = expected.filter(|n| *n != params.len()) {
                let d = Diagnostic::new(code::HOOK_PARAMS, at).with("name", Arg::Text(name.clone())).with("found", Arg::Text(params.len().to_string())).with("expected", Arg::Text(n.to_string()));
                self.say(d);
            }
            for (i, p) in params.iter().enumerate() {
                let row_param = fnty.as_ref().and_then(|f| f.params.get(i)).cloned();
                let (ty, var) = match &row_param {
                    Some(g) => {
                        let (t, _, nullable) = call::game_param(g);
                        let t = if nullable { Ty::Opt(Box::new(t)) } else { t };
                        // A `const T &` is the game's, read only here (docs/modding/language/compiled-form.md); the others are the hook's.
                        (t, !matches!(g, GameTy::Place { konst: true, .. }))
                    }
                    None => (Ty::Error, true),
                };
                if p.ty.is_some() {
                    let written = self.declared(owner, &param_key(key, p, i));
                    if !matches!(written, Ty::Error) && !matches!(ty, Ty::Error) && !written_fits(&written, &ty) {
                        let (f, e) = (self.ty_text(&written), self.ty_text(&ty));
                        self.say(Diagnostic::new(code::TYPE_MISMATCH, p.at).with("found", Arg::Text(f)).with("expected", Arg::Text(e)));
                    }
                }
                if let Some(pname) = &p.name {
                    let id = self.bind(pname, ty, var);
                    self.params.insert(id);
                }
            }
        }
        if let Some(ast::HookRet::Name(n)) = &h.ret {
            let t = match &result {
                Some(Ok(t)) => t.clone(),
                _ => Ty::Error,
            };
            self.bind(n, t, false);
        }
        let end = self.block(&h.body, Want::Drop);
        if end != Ty::Never {
            self.hook_ends(h.body.end);
        }
        self.frames.pop();
        if let (true, Some((n, row, _))) = (self.record, chosen) {
            self.c.out.hooks.insert((owner, key.to_string()), HookTarget { name: n, signature: row, overrides });
        }
    }

    /// The row a hook's head names: its function by name (in a class, a bare name is a member function of the
    /// class's game base or of a base of it), then the row by a quoted signature, else among several by the written
    /// parameters' count and types. None when it names none (said). Gives the name, the signature, whether the row is a
    /// constructor's or destructor's.
    fn hook_row(&mut self, owner: ItemRef, key: &str, h: &'a ast::Hook, class_of: Option<ItemRef>) -> Option<(String, String, bool)> {
        let at = h.target.at;
        let (written_name, written_sig) = match h.target.text.find('(') {
            Some(i) if h.target.quoted => (h.target.text[..i].trim().to_string(), Some(h.target.text[i..].to_string())),
            _ => (h.target.text.clone(), None),
        };
        let name = match class_of {
            Some(c) if !written_name.contains("::") => match self.class_base(c).and_then(|b| self.game_method(&b, &written_name)) {
                Some(full) => full,
                None => written_name.clone(),
            },
            _ => written_name.clone(),
        };
        let Some(rows) = self.game().functions.get(&name) else {
            self.unknown_name(at, &name);
            return None;
        };
        let all: Vec<(&'a str, bool)> = rows.iter().map(|(s, tell)| (s.as_str(), *tell)).collect();
        let listed = |list: &[(&str, bool)]| list.iter().map(|(s, _)| *s).collect::<Vec<_>>().join("; ");
        let mut cands = all.clone();
        if let Some(w) = &written_sig {
            let wanted = crate::ctype::parse_signature(&format!("void{w}"));
            cands.retain(|(r, _)| match (&wanted, crate::ctype::parse_signature(r)) {
                (Ok(a), Ok(b)) => a.params == b.params && a.konst == b.konst,
                _ => false,
            });
            if cands.is_empty() {
                self.say(Diagnostic::new(code::GAME_NO_SIGNATURE, at).with("found", Arg::Text(format!("{name}{w}"))).with("signatures", Arg::Text(listed(&all))));
                return None;
            }
        }
        if let (Some(ps), true) = (&h.params, cands.len() > 1) {
            let count = |r: &str| crate::ctype::parse_signature(r).map(|s| s.params.len()).ok();
            let by_count: Vec<(&'a str, bool)> = cands.iter().copied().filter(|(r, _)| count(r) == Some(ps.len())).collect();
            if !by_count.is_empty() {
                cands = by_count;
            }
            if cands.len() > 1 && ps.iter().any(|p| p.ty.is_some()) {
                let written: Vec<Option<Ty>> = ps.iter().enumerate().map(|(i, p)| p.ty.as_ref().map(|_| self.declared(owner, &param_key(key, p, i)))).collect();
                let game = self.game();
                cands.retain(|(r, _)| match game.hook_signature(r) {
                    Ok(f) => f.params.len() == written.len() && f.params.iter().zip(written.iter()).all(|(g, w)| match w {
                        None | Some(Ty::Error) => true,
                        Some(w) => {
                            let (t, _, nullable) = call::game_param(g);
                            *w == t || (nullable && *w == Ty::Opt(Box::new(t)))
                        }
                    }),
                    Err(_) => false,
                });
            }
        }
        match cands.as_slice() {
            [(row, tell)] => Some((name, row.to_string(), *tell)),
            [] => {
                self.say(Diagnostic::new(code::GAME_NO_SIGNATURE, at).with("found", Arg::Text(name.clone())).with("signatures", Arg::Text(listed(&all))));
                None
            }
            many => {
                self.say(Diagnostic::new(code::GAME_AMBIGUOUS, at).with("found", Arg::Text(name.clone())).with("signatures", Arg::Text(listed(many))));
                None
            }
        }
    }

    /// Every row the function facts say overrides (name, signature), directly or through another override.
    fn overrides_of(&self, name: &str, signature: &str) -> Vec<(String, String)> {
        let Some(ff) = self.game().facts.as_ref() else { return Vec::new() };
        let mut found: Vec<(String, String)> = Vec::new();
        let mut todo = vec![(name.to_string(), signature.to_string())];
        while let Some((n, s)) = todo.pop() {
            for ((rn, rs), f) in &ff.rows {
                if f.overrides.iter().any(|o| o.name == n && o.signature == s) && !found.iter().any(|(a, b)| a == rn && b == rs) {
                    found.push((rn.clone(), rs.clone()));
                    todo.push((rn.clone(), rs.clone()));
                }
            }
        }
        found.sort();
        found
    }

    /// A hook's body ends here (its end, or `return`): a wrap's or replace's path gives the result (S0123).
    fn hook_ends(&mut self, at: Pos) {
        let frame = self.frame();
        let Some(h) = &frame.hook else { return };
        if !matches!(h.kind, ast::HookKind::Wrap | ast::HookKind::Replace) || frame.path.given {
            return;
        }
        let Some(Ok(t)) = &h.result else { return };
        if matches!(t, Ty::Error) {
            return;
        }
        let (found, expected) = (h.name.clone(), self.ty_text(t));
        self.say(Diagnostic::new(code::RESULT_NOT_GIVEN, at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
    }

    /// `result` read (part e): an after's, a wrap's or a replace's own body, of a function that returns something,
    /// where every path to it gave it (S0124).
    pub(super) fn result_read(&mut self, at: Pos) -> Ty {
        let t = self.result_place(at);
        let frame = self.frame();
        if matches!(t, Ty::Error) || frame.path.given || frame.hook.as_ref().is_some_and(|h| h.not_yet_said) {
            return t;
        }
        self.say(Diagnostic::new(code::RESULT_NOT_YET, at));
        if let Some(h) = self.frame_mut().hook.as_mut() {
            h.not_yet_said = true;
        }
        t
    }

    /// `result` as a place: its type, or Ty::Error with S0120, S0116 or the result's not crossing said.
    pub(super) fn result_place(&mut self, at: Pos) -> Ty {
        let Some(h) = self.frame().hook.clone() else {
            self.say(Diagnostic::new(code::RESULT_HERE, at).with("why", Arg::Phrase("Script.Result.NotHook")));
            return Ty::Error;
        };
        if h.kind == ast::HookKind::Before {
            self.say(Diagnostic::new(code::RESULT_HERE, at).with("why", Arg::Phrase("Script.Result.Before")));
            return Ty::Error;
        }
        match h.result {
            None => {
                self.say(Diagnostic::new(code::HOOK_NO_RESULT, at).with("found", Arg::Text(h.name)));
                Ty::Error
            }
            Some(Err(why)) => {
                self.say(Diagnostic::new(code::GAME_RESULT_NOT_CROSSING, at).with("found", Arg::Text(h.name)).with("why", Arg::Phrase(why)));
                if let Some(h) = self.frame_mut().hook.as_mut() {
                    h.result = Some(Ok(Ty::Error));
                }
                Ty::Error
            }
            Some(Ok(t)) => t,
        }
    }

    /// `proceed()` (part e): a wrap's own body, once on any path (S0121, S0122). It gives the result.
    pub(super) fn proceed_here(&mut self, at: Pos) -> Ty {
        match self.frame().hook.as_ref().map(|h| h.kind) {
            Some(ast::HookKind::Wrap) => {
                if let Some(first) = self.frame().path.proceeded {
                    self.say(Diagnostic::new(code::PROCEED_TWICE, at).with("line", Arg::Text(first.line.to_string())));
                }
                let path = &mut self.frame_mut().path;
                path.proceeded = path.proceeded.or(Some(at));
                path.given = true;
            }
            Some(ast::HookKind::Replace) => self.say(Diagnostic::new(code::PROCEED_HERE, at).with("why", Arg::Phrase("Script.Proceed.Replace"))),
            _ => self.say(Diagnostic::new(code::PROCEED_HERE, at).with("why", Arg::Phrase("Script.Proceed.NotWrap"))),
        }
        Ty::Unit
    }

    /// The `states` machines of a mod class, over the mod's modules: their items.
    fn machines_of(&self, class: ItemRef) -> Vec<ItemRef> {
        let mut out = Vec::new();
        for (k, source) in self.c.modules.iter().enumerate() {
            let m = ModRef::of_mod(k);
            for (i, item) in source.module.items.iter().enumerate() {
                if let ItemKind::States(s) = &item.kind {
                    if self.c.scope_type(m, &s.class.name) == Some(Ty::Class(class)) {
                        out.push(m.item(i));
                    }
                }
            }
        }
        out
    }

    fn machine_of(&self, item: ItemRef) -> Option<&'a ast::States> {
        match &self.item_of(item).kind {
            ItemKind::States(s) => Some(s),
            _ => None,
        }
    }

    /// `go S` or `go M.S` (11.2, part e): in a machine (its own state, or a machine of the same class by name), or in
    /// a function or hook of the class (the machine named).
    fn go(&mut self, at: Pos, path: &'a [ast::Ident]) {
        let path: Vec<&'a ast::Ident> = path.iter().collect();
        self.state_target(at, &path, "go");
    }

    /// `is_state(S)` or `is_state(M.S)`: true while the object's machine is in that state. Placed and resolved
    /// as `go` is; the argument is a state's name, never walked as a value.
    pub(super) fn is_state(&mut self, e: &'a Expr, args: &'a [ast::Arg]) -> Ty {
        self.res.insert(e.id, Resolved::Std("is_state"));
        if args.len() != 1 {
            self.say(Diagnostic::new(code::ARG_COUNT, e.at).with("name", Arg::Text("is_state".into())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text("1".into())));
            return Ty::Bool;
        }
        let a = &args[0];
        let path: Option<Vec<&'a ast::Ident>> = match &a.value.kind {
            _ if a.name.is_some() || a.var => None,
            ExprKind::Name { name, targs } if targs.is_empty() => Some(vec![name]),
            ExprKind::Member { base, name, optional: false, targs } if targs.is_empty() => match &base.kind {
                ExprKind::Name { name: m, targs: mt } if mt.is_empty() => Some(vec![m, name]),
                _ => None,
            },
            _ => None,
        };
        match path {
            Some(p) => self.state_target(e.at, &p, "is_state"),
            None => self.say(Diagnostic::new(code::STATE_ARG, a.value.at)),
        }
        Ty::Bool
    }

    /// `keep(value)` (reload G-RL5): an `on stop`'s own body; the value json data, kept across a reload of
    /// the mod's code. True when kept (false: over 1 MiB encoded, which lumart also writes to the mod log).
    pub(super) fn keep_call(&mut self, e: &'a Expr, args: &'a [ast::Arg]) -> Ty {
        if self.frame().event != Some("stop") {
            self.say(Diagnostic::new(code::KEEP_HERE, e.at).with("what", Arg::Text("keep".into())).with("why", Arg::Phrase("Script.Keep.Stop")));
        }
        if !self.std_args(e.at, "keep", args, &[1]) {
            return Ty::Bool;
        }
        self.res.insert(e.id, Resolved::Std("keep"));
        let written = matches!(args[0].value.kind, ExprKind::List(_) | ExprKind::Braces(_));
        let t = self.expr(&args[0].value, if written { Want::Ty(&Ty::Json) } else { Want::Any });
        if !self.json_data(&t) {
            let found = self.ty_text(&t);
            self.say(Diagnostic::new(code::NOT_JSON_DATA, args[0].value.at).with("found", Arg::Text(found)));
        }
        Ty::Bool
    }

    /// `kept<T>()`: an `on start`'s own body before any wait (the kept bytes last until `start` returns); `T`
    /// json data. The value the previous load kept, or `none` (no reload, nothing kept, or not a `T`).
    pub(super) fn kept_call(&mut self, e: &'a Expr, targs: &'a [ast::Type], args: &'a [ast::Arg]) -> Ty {
        if self.frame().event != Some("start") || !self.frame().path.waits.is_empty() {
            self.say(Diagnostic::new(code::KEEP_HERE, e.at).with("what", Arg::Text("kept".into())).with("why", Arg::Phrase("Script.Kept.Start")));
        }
        let target = match targs {
            [t] => {
                let m = self.m;
                Some(self.c.resolve(m, t))
            }
            _ => {
                self.say(Diagnostic::new(code::TYPE_ARG_COUNT, e.at).with("found", Arg::Text("kept".into())).with("expected", Arg::Text("1".into())));
                None
            }
        };
        if !self.std_args(e.at, "kept", args, &[0]) {
            return Ty::Error;
        }
        self.res.insert(e.id, Resolved::Std("kept"));
        match target {
            Some(t) if self.json_data(&t) => Ty::Opt(Box::new(t)),
            Some(t) => {
                let found = self.ty_text(&t);
                self.say(Diagnostic::new(code::NOT_JSON_DATA, e.at).with("found", Arg::Text(found)));
                Ty::Error
            }
            None => Ty::Error,
        }
    }

    /// The state a `go` or `is_state` names (11.2): in a machine, `S` of its own or `M.S`; in a function or hook of
    /// the class, `M.S`. Said where it cannot be or names no state (S0126, S0127, S0130); recorded as
    /// `Resolved::State` at the state's name.
    fn state_target(&mut self, at: Pos, path: &[&'a ast::Ident], what: &'static str) {
        let here = || Diagnostic::new(code::GO_HERE, at).with("what", Arg::Text(what.into()));
        let frame = self.frame();
        let (own, class) = match (frame.kind, frame.machine, &frame.self_ty) {
            (Kind::State, Some(mach), self_ty) => (Some(mach), match self_ty {
                Some(Ty::Class(c)) => Some(*c),
                _ => None,
            }),
            (Kind::Fn | Kind::Hook, _, Some(Ty::Class(c))) if frame.members == Some(Members::Mod(*c)) => (None, Some(*c)),
            _ => (None, None),
        };
        if own.is_none() && class.is_none() {
            self.say(here());
            return;
        }
        let machines: Vec<ItemRef> = match class {
            Some(c) => self.machines_of(c),
            None => own.into_iter().collect(),
        };
        let names = |w: &Self, list: &[ItemRef]| list.iter().filter_map(|m| w.machine_of(*m)).map(|s| s.name.name.clone()).collect::<Vec<_>>().join(", ");
        let (machine, state) = match path {
            [s] => match own {
                Some(m) => (m, s),
                None => {
                    let listed = names(self, &machines);
                    if machines.is_empty() {
                        self.say(here());
                    } else {
                        let d = Diagnostic::new(code::GO_NAME_MACHINE, s.at).with("what", Arg::Text(what.into()));
                        self.say(d.with("found", Arg::Text(s.name.clone())).with("machine", Arg::Text(listed)));
                    }
                    return;
                }
            },
            [m, s] => match machines.iter().find(|x| self.machine_of(**x).is_some_and(|st| st.name.name == m.name)) {
                Some(x) => (*x, s),
                None => {
                    let listed = names(self, &machines);
                    self.say(Diagnostic::new(code::GO_UNKNOWN, m.at).with("found", Arg::Text(m.name.clone())).with("machine", Arg::Text(listed)));
                    return;
                }
            },
            _ => {
                let written = path.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(".");
                let listed = names(self, &machines);
                self.say(Diagnostic::new(code::GO_UNKNOWN, at).with("found", Arg::Text(written)).with("machine", Arg::Text(listed)));
                return;
            }
        };
        let state: &ast::Ident = state;
        let Some(st) = self.machine_of(machine) else { return };
        if st.states.iter().any(|x| x.name.name == state.name) {
            self.named.insert(state.at, Resolved::State { machine, state: state.name.clone() });
        } else {
            let mname = st.name.name.clone();
            self.say(Diagnostic::new(code::GO_UNKNOWN, state.at).with("found", Arg::Text(state.name.clone())).with("machine", Arg::Text(mname)));
        }
    }

    /// The game record a hooked `Record::function` is a member of, when its qualifier names one.
    fn hook_record(&self, target: &ast::GameName) -> Option<String> {
        let name = target.text.split('(').next().unwrap_or("");
        let (qualifier, _) = name.rsplit_once("::")?;
        let types = self.game().types.as_ref()?;
        types.record(qualifier).map(|(row, _)| row.to_string())
    }

    fn on(&mut self, on: &'a ast::On) {
        let ret = match &on.event {
            ast::OnEvent::Layers(t) => {
                let m = self.m;
                let ty = self.c.resolve(m, t);
                // The kernel's layers call takes names: list<text> and nothing else.
                let want = Ty::List(Box::new(Ty::Text));
                if ty != Ty::Error && ty != want {
                    let found = self.ty_text(&ty);
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, t.at).with("found", Arg::Text(found)).with("expected", Arg::Text("list<text>".into())));
                }
                Some(ty)
            }
            _ => None,
        };
        let mut frame = Frame::new(Kind::Handler, ret.clone());
        // The compiler makes a task of a handler of start, a stage, an option or an event that waits; the others
        // run inside one call (stop, frame, load, draw, layers, step).
        frame.waits_ok = matches!(on.event, ast::OnEvent::Start | ast::OnEvent::Stage(_) | ast::OnEvent::Option(_) | ast::OnEvent::Event { .. });
        frame.event = match on.event {
            ast::OnEvent::Start => Some("start"),
            ast::OnEvent::Stop => Some("stop"),
            _ => None,
        };
        self.frames.push(frame);
        // `on option x`: an option of the mod's luma.json (docs/modding/language/language.md, section 10).
        if let ast::OnEvent::Option(key) = &on.event {
            if !self.c.facts.options.iter().any(|o| o.key == key.name) {
                self.say(Diagnostic::new(code::OPTION_NOT_IN_LUMA_JSON, key.at).with("found", Arg::Text(key.name.clone())));
            }
        }
        // `on draw <point>`: a case of the draw service's Point.
        if let ast::OnEvent::Draw(path) = &on.event {
            self.draw_point(path);
        }
        if let ast::OnEvent::Event { path, names, ty } = &on.event {
            let service = match (path.as_slice(), path.first().map(|p| self.name_kind(&p.name))) {
                ([_, kind], Some(expr::NameKind::Module(super::ModTarget::Service(s)))) => Some((s, kind)),
                _ => None,
            };
            let stopped = on.event.is_kit_stopped();
            match service {
                // A service's event (part f); svc.mods' STOPPED only with its record type written.
                Some((s, kind)) if !stopped => {
                    // A type written on its name is the event's record, or a mismatch.
                    if let (Some(t), Some(first)) = (ty, names.as_ref().and_then(|n| n.first())) {
                        let m = self.m;
                        let written = self.c.resolve(m, t);
                        let record = Ty::Service { service: s.clone(), name: service::event_name(&kind.name) };
                        if written != Ty::Error && written != record {
                            let (found, expected) = (self.ty_text(&written), self.ty_text(&record));
                            self.say(Diagnostic::new(code::TYPE_MISMATCH, first.at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                        }
                    }
                    self.service_on(&s, kind, names.as_deref())
                }
                // `on mods.stopped(m)`: a mod that called this library's exports was taken out (the loader's
                // dependent_gone); `m` is the number `caller()` gave inside its calls (lumakit rule 4).
                _ if stopped => {
                    self.frame_mut().waits_ok = false;
                    for n in names.iter().flatten() {
                        let id = self.bind(n, Ty::Int(Scalar::U64), false);
                        self.params.insert(id);
                    }
                }
                _ => {
                    for n in names.iter().flatten() {
                        self.bind(n, Ty::Error, false);
                    }
                }
            }
        }
        // `on step(now: time.Now)`: its parameter, of the type written (the service's struct, part f).
        if let ast::OnEvent::Step(Some(p)) = &on.event {
            if let (Some(n), Some(t)) = (&p.name, &p.ty) {
                let m = self.m;
                let ty = self.c.resolve(m, t);
                let id = self.bind(n, ty, false);
                self.params.insert(id);
            }
        }
        let end = self.block(&on.body, Want::Drop);
        self.returns(&ret, &end, on.body.end);
        self.frames.pop();
    }

    fn states(&mut self, me: ItemRef, s: &'a ast::States) {
        let class = match self.c.scope_type(self.m, &s.class.name) {
            Some(Ty::Class(c)) => Some(c),
            _ => None,
        };
        for state in &s.states {
            let mut frame = Frame::new(Kind::State, None);
            frame.self_ty = class.map(Ty::Class);
            frame.members = class.map(Members::Mod);
            frame.machine = Some(me);
            self.frames.push(frame);
            // `enter`, `update` and `exit` once a state (S0129).
            let mut seen: HashMap<&'static str, Pos> = HashMap::new();
            for part in &state.parts {
                let word = match part {
                    ast::StatePart::Enter(_) => Some("enter"),
                    ast::StatePart::Update(_) => Some("update"),
                    ast::StatePart::Exit(_) => Some("exit"),
                    ast::StatePart::Stmt(_) => None,
                };
                if let (Some(w), ast::StatePart::Enter(b) | ast::StatePart::Update(b) | ast::StatePart::Exit(b)) = (word, part) {
                    match seen.get(w) {
                        Some(first) => {
                            let d = Diagnostic::new(code::STATE_PART_TWICE, b.at).with("found", Arg::Text(w.into())).with("line", Arg::Text(first.line.to_string()));
                            self.say(d);
                        }
                        None => {
                            seen.insert(w, b.at);
                        }
                    }
                }
                match part {
                    ast::StatePart::Enter(b) => {
                        self.frame_mut().waits_ok = true;
                        self.block(b, Want::Drop);
                    }
                    // `update` runs each frame and `exit` as the state is left: both run inside one call.
                    ast::StatePart::Update(b) | ast::StatePart::Exit(b) => {
                        let since = self.next_tick();
                        let frame = self.frame_mut();
                        frame.waits_ok = false;
                        frame.later_from = Some(since);
                        self.block(b, Want::Drop);
                        self.frame_mut().later_from = None;
                    }
                    ast::StatePart::Stmt(st) => {
                        self.frame_mut().waits_ok = true;
                        self.stmt(st);
                    }
                }
            }
            self.frames.pop();
        }
    }

    // ------------------------------------------------------------ statements

    /// A block; with a value wanted, its last statement's value is the block's. `Never` when it always leaves.
    fn block(&mut self, b: &'a Block, want: Want<'_>) -> Ty {
        self.scope_push();
        let mut ty = Ty::Unit;
        let mut leaves = false;
        let n = b.stmts.len();
        for (k, s) in b.stmts.iter().enumerate() {
            if k + 1 == n && !matches!(want, Want::Drop) {
                if let StmtKind::Expr(e) = &s.kind {
                    ty = self.expr(e, want);
                    continue;
                }
            }
            if self.stmt(s) {
                leaves = true;
            }
        }
        self.scope_pop();
        if leaves && ty == Ty::Unit {
            Ty::Never
        } else {
            ty
        }
    }

    /// One statement; true when it always leaves (`return`, `break`, `continue`, `stop`, `skip`, `go`).
    fn stmt(&mut self, s: &'a Stmt) -> bool {
        match &s.kind {
            StmtKind::Binding(b) => {
                self.local_binding(b);
                false
            }
            StmtKind::Fn(f) => {
                self.local_fn(f);
                false
            }
            StmtKind::Return(v) => {
                self.leaves_defer(s.at, "return", false);
                self.hook_ends(s.at);
                let ret = self.frame().ret.clone();
                match (v, ret) {
                    (Some(v), None) => {
                        self.expr(v, Want::Any);
                        self.say(Diagnostic::new(code::RETURN_NO_VALUE, s.at));
                    }
                    (Some(v), Some(t)) => {
                        self.expr(v, Want::Ty(&t));
                    }
                    (None, Some(t)) if t != Ty::Unit && t != Ty::Error => {
                        let expected = self.ty_text(&t);
                        self.say(Diagnostic::new(code::RETURN_NEEDS_VALUE, s.at).with("expected", Arg::Text(expected)));
                    }
                    (None, _) => {}
                }
                true
            }
            StmtKind::Break | StmtKind::Continue => {
                let word = if matches!(s.kind, StmtKind::Break) { "break" } else { "continue" };
                if self.frame().loops == 0 {
                    self.say(Diagnostic::new(code::OUTSIDE_LOOP, s.at).with("found", Arg::Text(word.into())));
                } else {
                    self.leaves_defer(s.at, word, true);
                }
                self.loop_leave(matches!(s.kind, StmtKind::Break));
                true
            }
            StmtKind::Stop => {
                self.leaves_defer(s.at, "stop", false);
                // A task's, a machine's (11.1, 11.2), or a handler's the compiler makes a task of.
                let frame = self.frame();
                if !(matches!(frame.kind, Kind::Task | Kind::State) || (frame.kind == Kind::Handler && frame.waits_ok)) {
                    self.say(Diagnostic::new(code::STOP_HERE, s.at));
                }
                true
            }
            StmtKind::Go(path) => {
                self.leaves_defer(s.at, "go", false);
                self.go(s.at, path);
                true
            }
            StmtKind::Skip(v) => {
                self.leaves_defer(s.at, "skip", false);
                self.skip(s.at, v.as_ref());
                true
            }
            StmtKind::Defer(inner) => {
                let loops = self.frame().loops;
                self.frame_mut().defers.push(loops);
                self.stmt(inner);
                self.frame_mut().defers.pop();
                false
            }
            StmtKind::Log { fact, value } => {
                if let Some(f) = fact {
                    self.expr(f, Want::Ty(&Ty::Text));
                }
                let t = self.expr(value, Want::Any);
                self.shown(&t, value.at);
                false
            }
            StmtKind::Expect(e) => {
                self.expr(e, Want::Ty(&Ty::Bool));
                false
            }
            StmtKind::Fail(e) => {
                // It leaves the function as a script error does: nothing after it runs.
                self.leaves_defer(s.at, "fail", false);
                self.expr(e, Want::Ty(&Ty::Text));
                true
            }
            StmtKind::Guard { conds, otherwise } => {
                // The `else` first: the names the conditions bind are not in it. It must leave (docs/modding/language/language.md, section 4). It runs
                // after the conditions, so after a `wait` that is one of them; its own waits leave with it (A42D-3).
                let before = self.path_waits();
                if let Some(w) = conds.iter().map(|c| first_read(&c.value)).find(|x| matches!(x.kind, ExprKind::Wait { .. } | ExprKind::WaitUntil(_))) {
                    let t = self.next_tick();
                    self.frame_mut().path.waits.push((t, w.at));
                }
                if self.block(otherwise, Want::Drop) != Ty::Never {
                    self.say(Diagnostic::new(code::GUARD_STAYS, otherwise.at));
                }
                self.arm_waits(&before);
                self.conds(conds);
                false
            }
            StmtKind::While { cond, body } => {
                // The condition is read again each time round: it is inside the loop.
                self.loop_start();
                self.expr(cond, Want::Ty(&Ty::Bool));
                self.loop_entry();
                self.frame_mut().loops += 1;
                let leaves = self.block(body, Want::Drop) == Ty::Never;
                self.frame_mut().loops -= 1;
                let (broke, _) = self.loop_end(leaves);
                // `while true` that no `break` leaves never ends: what follows it is not reached.
                matches!(cond.kind, ExprKind::True) && !broke
            }
            StmtKind::For { pattern, iter, body } => {
                let t = self.expr(iter, Want::Any);
                let elem = self.element_of(&t, iter.at);
                self.scope_push();
                self.loop_start();
                self.loop_entry();
                self.irrefutable(pattern, &elem, false);
                // The pattern's first name, for a list made in the `for` itself (A42D-2).
                let mut names = Vec::new();
                super::pattern_names(pattern, &mut names);
                let first = names.first().and_then(|n| self.find_local(&n.name)).map(|(id, _)| id);
                self.frame_mut().loops += 1;
                let leaves = self.block(body, Want::Drop) == Ty::Never;
                self.frame_mut().loops -= 1;
                let (_, round_wait) = self.loop_end(leaves);
                self.scope_pop();
                // The loop goes on through what `iter` gave, a local's or one made here: a view of what holds the
                // views, read after a `wait` on a path back round (S0104).
                if let Some(w) = round_wait.filter(|_| holds_view(&t)) {
                    if let Some(id) = self.root_local(iter).or(first) {
                        let name = self.local(id).name.clone();
                        self.across(id, &name, iter.at, w);
                    }
                }
                false
            }
            StmtKind::Expr(e) => {
                self.whole_value(e);
                let t = self.expr(e, Want::Drop);
                // A `pending<()>` dropped loses nothing: a result-less task called and left to run.
                if matches!(&t, Ty::Pending(of) if **of != Ty::Unit) {
                    self.say(Diagnostic::new(code::PENDING_DROPPED, e.at));
                }
                t == Ty::Never
            }
            StmtKind::Assign { target, op, value } => {
                self.assign(target, *op, value);
                false
            }
        }
    }

    /// `skip` (section 8, part e): a `before`'s own body; with the hooked function's result when it has one, without
    /// one when it has none.
    fn skip(&mut self, at: Pos, v: Option<&'a Expr>) {
        let hook = self.frame().hook.clone().filter(|h| h.kind == ast::HookKind::Before);
        let Some(h) = hook else {
            self.say(Diagnostic::new(code::SKIP_OUTSIDE, at));
            if let Some(v) = v {
                self.expr(v, Want::Any);
            }
            return;
        };
        match (&h.result, v) {
            (None, Some(v)) => {
                self.expr(v, Want::Any);
                self.say(Diagnostic::new(code::SKIP_VALUE, v.at).with("found", Arg::Text(h.name)));
            }
            (Some(Ok(t)), Some(v)) => {
                self.expr(v, Want::Ty(t));
            }
            (Some(Ok(t)), None) if !matches!(t, Ty::Error) => {
                let expected = self.ty_text(t);
                self.say(Diagnostic::new(code::SKIP_NEEDS_VALUE, at).with("found", Arg::Text(h.name)).with("expected", Arg::Text(expected)));
            }
            (Some(Err(why)), v) => {
                if let Some(v) = v {
                    self.expr(v, Want::Any);
                }
                self.say(Diagnostic::new(code::GAME_RESULT_NOT_CROSSING, at).with("found", Arg::Text(h.name)).with("why", Arg::Phrase(why)));
                if let Some(h) = self.frame_mut().hook.as_mut() {
                    h.result = Some(Ok(Ty::Error));
                }
            }
            _ => {}
        }
    }

    /// What `for` gives each time round: a list's or set's values, a map's (key, value), a range's numbers, a game
    /// array's elements.
    fn element_of(&mut self, t: &Ty, at: Pos) -> Ty {
        match t {
            Ty::List(e) | Ty::Set(e) | Ty::Range(e) => (**e).clone(),
            Ty::Map(k, v) => Ty::Tuple(vec![(**k).clone(), (**v).clone()]),
            Ty::Game(GameTy::Array { of, extents }) => expr::array_element(of, extents),
            Ty::Error | Ty::Never => Ty::Error,
            other => {
                let found = self.ty_text(other);
                self.say(Diagnostic::new(code::NOT_ITERABLE, at).with("found", Arg::Text(found)));
                Ty::Error
            }
        }
    }

    /// `let` or `var` in a body.
    fn local_binding(&mut self, b: &'a ast::Binding) {
        let declared = b.ty.as_ref().map(|t| {
            let m = self.m;
            self.c.resolve(m, t)
        });
        if let Some(v) = &b.value {
            self.whole_value(v);
        }
        let ty = match (&declared, &b.value) {
            (Some(t), Some(v)) => {
                self.expr(v, Want::Ty(t));
                t.clone()
            }
            (Some(t), None) => t.clone(),
            (None, Some(v)) => {
                let t = self.expr(v, Want::Any);
                self.stored(t, v.at)
            }
            (None, None) => {
                self.say(Diagnostic::new(code::TYPE_NEEDS_WRITING, b.pattern.at).with("found", Arg::Text(pattern_text(&b.pattern))));
                Ty::Error
            }
        };
        self.irrefutable(&b.pattern, &ty, b.var);
    }

    /// Whether writing `target` goes through the view its local holds (`a.x`, `a[i]`), not into the local itself.
    fn local_view_place(&self, target: &Expr) -> bool {
        !matches!(target.kind, ExprKind::Name { .. })
    }

    /// The local an expression reads through (`xs`, `xs[0]`, `xs.f`), if it is one.
    fn root_local(&self, e: &Expr) -> Option<LocalId> {
        match &e.kind {
            ExprKind::Paren(x) | ExprKind::Member { base: x, .. } | ExprKind::Index { base: x, .. } => self.root_local(x),
            ExprKind::Name { .. } => match self.res.get(&e.id) {
                Some(Resolved::Local(id)) => Some(*id),
                _ => None,
            },
            _ => None,
        }
    }

    /// A type as a local holds it: a literal at its default; a view of a plain game struct as the script's own copy
    /// (`let s = view.mStates` copies it); a class's view stays a view (part d keeps it in its statement).
    fn stored(&mut self, t: Ty, at: Pos) -> Ty {
        match t {
            Ty::IntLit => Ty::Int(Scalar::I32),
            Ty::FloatLit => Ty::Float(Scalar::F32),
            Ty::Open => {
                self.say(Diagnostic::new(code::TYPE_UNKNOWN_HERE, at).with("found", Arg::Text("none".into())));
                Ty::Error
            }
            Ty::Never => Ty::Error,
            Ty::Game(GameTy::View { record, konst }) => match self.plain_struct(&record) {
                true => Ty::Game(GameTy::Struct(record)),
                false => Ty::Game(GameTy::View { record, konst }),
            },
            // `let b = a?.mBox`: the same copy, or none.
            Ty::Opt(inner) => match *inner {
                Ty::Game(GameTy::View { record, konst }) => match self.plain_struct(&record) {
                    true => Ty::Opt(Box::new(Ty::Game(GameTy::Struct(record)))),
                    false => Ty::Opt(Box::new(Ty::Game(GameTy::View { record, konst }))),
                },
                other => Ty::Opt(Box::new(other)),
            },
            other => other,
        }
    }

    /// A record with no vtable that crosses by value: a `let` of a view of one copies it.
    fn plain_struct(&self, record: &str) -> bool {
        let game = self.game();
        let poly = game.types.as_ref().and_then(|t| t.records.get(record)).is_none_or(|r| r.polymorphic);
        !poly && game.by_value(record).is_ok()
    }

    /// A pattern that must match (`let`, `for`): names and tuples of names; it binds them.
    fn irrefutable(&mut self, p: &'a ast::Pattern, ty: &Ty, var: bool) {
        match &p.kind {
            ast::PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => {
                self.bind(&path[0], ty.clone(), var);
            }
            ast::PatternKind::Tuple(items) => {
                let tys = self.tuple_parts(ty, items.len(), p.at);
                for (sub, t) in items.iter().zip(tys.iter()) {
                    self.irrefutable(sub, t, var);
                }
            }
            _ => {
                self.say(Diagnostic::new(code::REFUTABLE_PATTERN, p.at));
                // Its names still bind, so what follows is checked.
                let mut idents = Vec::new();
                pat::pattern_binds(p, &mut idents);
                for ident in idents {
                    self.bind(ident, Ty::Error, var);
                }
            }
        }
    }

    /// A tuple type's parts for a pattern of `n`; a mismatch is said once.
    fn tuple_parts(&mut self, ty: &Ty, n: usize, at: Pos) -> Vec<Ty> {
        match ty {
            Ty::Tuple(ts) if ts.len() == n => ts.clone(),
            Ty::Error | Ty::Never => vec![Ty::Error; n],
            other => {
                let found = self.ty_text(other);
                let expected = format!("({})", vec!["_"; n].join(", "));
                self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                vec![Ty::Error; n]
            }
        }
    }

    /// `fn` in a body: a local of a function type, which it sees from its own body too (recursion).
    fn local_fn(&mut self, f: &'a ast::FnDecl) {
        let m = self.m;
        let params: Vec<Ty> = f.params.iter().map(|p| p.ty.as_ref().map_or(Ty::Error, |t| self.c.resolve(m, t))).collect();
        let ret = f.ret.as_ref().map(|t| self.c.resolve(m, t));
        let fn_ty = Ty::Fn(params.clone(), Box::new(ret.clone().unwrap_or(Ty::Unit)));
        self.bind(&f.name, fn_ty, false);
        self.closure_body(&f.params, &params, ret, &f.body);
    }

    /// A closure's or a local function's body: its parameters, with the locals around it as copies (5.4).
    fn closure_body(&mut self, params: &'a [ast::Param], tys: &[Ty], ret: Option<Ty>, body: &'a Block) {
        let members = self.frame().members.clone();
        let self_ty = self.frame().self_ty.clone();
        let mut frame = Frame::new(Kind::Closure, ret.clone());
        frame.members = members;
        frame.self_ty = self_ty;
        frame.runs_now = std::mem::take(&mut self.next_runs_now);
        self.frames.push(frame);
        for (p, t) in params.iter().zip(tys.iter()) {
            if p.is_self {
                self.say(Diagnostic::new(code::SELF_OUTSIDE, p.at));
                continue;
            }
            if let Some(name) = &p.name {
                let id = self.bind(name, t.clone(), p.var);
                self.params.insert(id);
            }
        }
        let end = self.block(body, Want::Drop);
        self.returns(&ret, &end, body.end);
        self.frames.pop();
    }

    /// The conditions of `if`, `guard` and `while`'s `let`s, each in order; names bound go into the current scope.
    fn conds(&mut self, conds: &'a [ast::Cond]) {
        for c in conds {
            self.whole_value(&c.value);
            match &c.pattern {
                None => {
                    self.expr(&c.value, Want::Ty(&Ty::Bool));
                    // What it shows given holds for the conditions after it and the block (an `if`'s, or the rest of
                    // a `guard`'s).
                    let mut shown = Vec::new();
                    self.given_by(&c.value, true, &mut shown);
                    self.given.extend(shown);
                }
                Some(p) => {
                    let t = self.expr(&c.value, Want::Any);
                    match t {
                        Ty::Opt(inner) => {
                            let inner = self.stored(*inner, c.value.at);
                            self.pattern(p, &inner, false);
                        }
                        Ty::Error | Ty::Never => {
                            self.pattern(p, &Ty::Error, false);
                        }
                        other if pat::irrefutable_shape(p) => {
                            let found = self.ty_text(&other);
                            self.say(Diagnostic::new(code::UNWRAP_NOT_OPTIONAL, c.value.at).with("found", Arg::Text(found)));
                            self.pattern(p, &other, false);
                        }
                        other => {
                            self.pattern(p, &other, false);
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ assignment and places

    fn assign(&mut self, target: &'a Expr, op: AssignOp, value: &'a Expr) {
        if matches!(op, AssignOp::Set) {
            self.whole_value(value);
        }
        // `result = v` gives the hooked function's result (part e): written, not read.
        if matches!(op, AssignOp::Set) && matches!(target.kind, ExprKind::Result) {
            let t = self.result_place(target.at);
            self.tys.insert(target.id, t.clone());
            self.expr(value, Want::Ty(&t));
            if self.frame().hook.is_some() {
                self.frame_mut().path.given = true;
            }
            return;
        }
        let from = self.tick;
        let t = self.expr(target, Want::Any);
        if let Err(why) = self.place(target) {
            self.say(Diagnostic::new(code::NOT_A_PLACE, target.at).with("why", Arg::Phrase(why)));
        }
        let Some(bin) = assign_op(op) else {
            self.expr(value, Want::Ty(&t));
            // `a.x = wait p`: the place is written after the wait, through a view made before it (S0104).
            let waited = self.waited_from(from);
            if let (Some(w), Some(id)) = (waited, self.root_local(target)) {
                if holds_view(&self.local(id).ty) && self.local_view_place(target) {
                    let name = self.local(id).name.clone();
                    self.across(id, &name, target.at, w);
                }
            }
            return;
        };
        let rhs = expr::rhs_want(bin, &t);
        let v = match &rhs {
            Some(w) => self.expr(value, Want::Ty(w)),
            None => self.expr(value, Want::Any),
        };
        let result = self.binary_types(bin, &t, &v, value.at);
        if !self.fits(&result, &t) {
            let (found, expected) = (self.ty_text(&result), self.ty_text(&t));
            self.say(Diagnostic::new(code::TYPE_MISMATCH, value.at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
        }
    }

    /// Whether an expression is a place a script may write (5.1, 6.1, docs/modding/language/compiled-form.md), from what the walk found.
    fn place(&self, e: &Expr) -> Writable {
        match &e.kind {
            ExprKind::Paren(inner) => self.place(inner),
            ExprKind::Name { .. } => match self.res.get(&e.id) {
                Some(Resolved::Local(id)) => {
                    let captured = self.captured.contains(&e.id);
                    if captured {
                        Err("Script.Place.Captured")
                    } else if self.local(*id).var {
                        Ok(())
                    } else if self.is_param(*id) {
                        Err("Script.Place.Param")
                    } else {
                        Err("Script.Place.Let")
                    }
                }
                Some(Resolved::Item(item)) => match self.c.scope(item.module_ref()).names.values().find_map(|n| match n {
                    Name::Item { item: i, sort, .. } if *i == item.item => Some(*sort),
                    _ => None,
                }) {
                    Some(Sort::Var) => Ok(()),
                    Some(Sort::Option) => Err("Script.Place.Option"),
                    Some(Sort::Let) => Err("Script.Place.Let"),
                    _ => Err("Script.Place.Value"),
                },
                Some(Resolved::Member { .. }) => Ok(()),
                Some(Resolved::GameField { .. }) | Some(Resolved::GameGlobal(_)) => self.game_writable(e.id),
                Some(Resolved::GameConstant { .. }) => Err("Script.Place.ReadOnly"),
                None if self.tys.get(&e.id) == Some(&Ty::Error) => Ok(()),
                _ => Err("Script.Place.Value"),
            },
            ExprKind::Game(_) => match self.res.get(&e.id) {
                Some(Resolved::GameGlobal(_)) => self.game_writable(e.id),
                Some(Resolved::GameConstant { .. }) => Err("Script.Place.ReadOnly"),
                None => Ok(()),
                _ => Err("Script.Place.Value"),
            },
            ExprKind::SelfValue => Err("Script.Place.Value"),
            ExprKind::Result => Ok(()),
            ExprKind::Member { base, optional: false, .. } => match self.res.get(&e.id) {
                Some(Resolved::GameField { .. }) => self.game_writable(e.id),
                Some(Resolved::Member { .. }) => match self.tys.get(&base.id) {
                    // A class's object is the game's: its fields are written there.
                    Some(Ty::Class(_)) => Ok(()),
                    _ => self.value_place(base),
                },
                Some(Resolved::TupleIndex(_)) | Some(Resolved::Component(_)) => self.value_place(base),
                // A number field of a service's struct (not an event's record, the service's to fill): the header's
                // struct holds it as the script does.
                Some(Resolved::ServiceField { service, record, .. })
                    if self.service_struct_named(service, record) && matches!(self.tys.get(&e.id), Some(Ty::Int(_) | Ty::Float(_))) =>
                {
                    self.value_place(base)
                }
                None if self.tys.get(&e.id) == Some(&Ty::Error) => Ok(()),
                _ => Err("Script.Place.Value"),
            },
            ExprKind::Index { base, .. } => match self.tys.get(&base.id) {
                Some(Ty::Game(GameTy::Array { .. })) => self.game_writable(e.id),
                Some(Ty::List(_) | Ty::Map(..) | Ty::Bytes) => self.value_place(base),
                Some(Ty::Error) | None => Ok(()),
                _ => Err("Script.Place.Value"),
            },
            ExprKind::Unwrap(_) if self.game_places.contains(&e.id) => Ok(()),
            _ => Err("Script.Place.Value"),
        }
    }

    /// A part of a value (a struct's field, a list's element): writable when the value is, or `self` is `var self`.
    fn value_place(&self, base: &Expr) -> Writable {
        match &base.kind {
            ExprKind::SelfValue => match self.frames.iter().rev().find(|f| f.self_ty.is_some()) {
                Some(f) if f.self_var => Ok(()),
                // `self` in a class or a hook is the game's object, and its members are places there.
                Some(f) if matches!(f.self_ty, Some(Ty::Class(_) | Ty::Game(_))) => Ok(()),
                _ => Err("Script.Place.Param"),
            },
            _ => self.place(base),
        }
    }

    /// A game field, global or array element: read only when the game's type says so (docs/modding/language/compiled-form.md) or the view
    /// it is reached through is const.
    fn game_writable(&self, id: ExprId) -> Writable {
        match self.read_only.get(&id) {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }

    fn is_param(&self, id: LocalId) -> bool {
        self.params.contains(&id)
    }

    // ------------------------------------------------------------ text of types

    /// A type as a message names it (LumaScript's spelling; a game type by its row's name).
    pub(super) fn ty_text(&self, t: &Ty) -> String {
        match t {
            Ty::Bool => "bool".into(),
            Ty::Int(s) | Ty::Float(s) => scalar_text(*s).into(),
            Ty::Value(v) => value_text(*v).into(),
            Ty::Text => "text".into(),
            Ty::Bytes => "bytes".into(),
            Ty::List(e) => format!("list<{}>", self.ty_text(e)),
            Ty::Set(e) => format!("set<{}>", self.ty_text(e)),
            Ty::Map(k, v) => format!("map<{}, {}>", self.ty_text(k), self.ty_text(v)),
            Ty::Opt(e) => format!("{}?", self.ty_text(e)),
            Ty::Tuple(items) => format!("({})", items.iter().map(|i| self.ty_text(i)).collect::<Vec<_>>().join(", ")),
            Ty::Fn(ps, r) => {
                let ps = ps.iter().map(|p| self.ty_text(p)).collect::<Vec<_>>().join(", ");
                match &**r {
                    Ty::Unit => format!("fn({ps})"),
                    r => format!("fn({ps}) -> {}", self.ty_text(r)),
                }
            }
            Ty::Ref(of) => format!("ref<{}>", self.ty_text(of)),
            Ty::Pending(of) => format!("pending<{}>", self.ty_text(of)),
            Ty::Json => "json.Value".into(),
            Ty::Struct(i) | Ty::Enum(i) | Ty::Class(i) => self.c.item_name(*i),
            Ty::Game(g) => self.game_text(g),
            Ty::Service { service, name } => format!("{service}.{name}"),
            Ty::Unit => "()".into(),
            Ty::Range(of) => format!("{}..{}", self.ty_text(of), self.ty_text(of)),
            Ty::Task => "task".into(),
            Ty::Content(word) => word.clone(),
            Ty::Group { record, prefix, .. } => format!("{record}.{prefix}"),
            Ty::IntLit => "int".into(),
            Ty::FloatLit => "f32".into(),
            Ty::Open => "none".into(),
            Ty::Never | Ty::Error => "_".into(),
        }
    }

    fn game_text(&self, g: &GameTy) -> String {
        match g {
            GameTy::Unit => "()".into(),
            GameTy::Scalar(s) => scalar_text(*s).into(),
            GameTy::Value(v) => value_text(*v).into(),
            GameTy::Enum(n) | GameTy::Struct(n) => n.clone(),
            GameTy::View { record, .. } => record.clone(),
            GameTy::Text => "text".into(),
            GameTy::Place { of, .. } => self.game_text(of),
            GameTy::Opt(of) => format!("{}?", self.game_text(of)),
            GameTy::Bits { of, .. } => scalar_text(*of).into(),
            GameTy::Null => "none".into(),
            GameTy::Array { of, extents } => {
                format!("{}{}", self.game_text(of), extents.iter().map(|n| format!("[{n}]")).collect::<String>())
            }
            GameTy::List { of, fixed: None, .. } => format!("list<{}>", self.game_text(of)),
            GameTy::List { of, fixed: Some(n), .. } => format!("list<{}> of {n}", self.game_text(of)),
        }
    }
}

/// The key part (b) gave a parameter's type.
/// Whether a type the script wrote for a hook's or an override's parameter or result is the game's: the same, but
/// for a view's `const`, which a script cannot write, and a game struct named where the game passes a reference to
/// one (the binding keeps the game's type).
fn written_fits(written: &Ty, game: &Ty) -> bool {
    // A game struct written by name where the game passes it by reference: the parameter is that struct, read
    // through the game's reference.
    if let (Ty::Game(GameTy::Struct(a)), Ty::Game(GameTy::View { record, .. })) = (written, game) {
        return a == record;
    }
    unconst(written) == unconst(game)
}

fn unconst(t: &Ty) -> Ty {
    match t {
        Ty::Game(g) => Ty::Game(unconst_game(g)),
        Ty::Opt(of) => Ty::Opt(Box::new(unconst(of))),
        Ty::List(of) => Ty::List(Box::new(unconst(of))),
        other => other.clone(),
    }
}

fn unconst_game(g: &GameTy) -> GameTy {
    match g {
        GameTy::View { record, .. } => GameTy::View { record: record.clone(), konst: false },
        GameTy::Opt(of) => GameTy::Opt(Box::new(unconst_game(of))),
        other => other.clone(),
    }
}

fn param_key(key: &str, p: &ast::Param, i: usize) -> String {
    let pname = p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone());
    format!("{key}({pname})")
}

/// A pattern as written, for messages.
fn pattern_text(p: &ast::Pattern) -> String {
    match &p.kind {
        ast::PatternKind::Tuple(items) => format!("({})", items.iter().map(pattern_text).collect::<Vec<_>>().join(", ")),
        ast::PatternKind::Path { path, .. } => path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("."),
        ast::PatternKind::Literal(_) => "_".into(),
    }
}

fn assign_op(op: AssignOp) -> Option<ast::BinOp> {
    use ast::BinOp as B;
    Some(match op {
        AssignOp::Set => return None,
        AssignOp::Add => B::Add,
        AssignOp::Sub => B::Sub,
        AssignOp::Mul => B::Mul,
        AssignOp::Div => B::Div,
        AssignOp::Rem => B::Rem,
        AssignOp::And => B::BitAnd,
        AssignOp::Or => B::BitOr,
        AssignOp::Xor => B::BitXor,
        AssignOp::Shl => B::Shl,
        AssignOp::Shr => B::Shr,
    })
}

pub(super) fn scalar_text(s: Scalar) -> &'static str {
    match s {
        Scalar::Bool => "bool",
        Scalar::I8 => "i8",
        Scalar::I16 => "i16",
        Scalar::I32 => "i32",
        Scalar::I64 => "i64",
        Scalar::U8 => "u8",
        Scalar::U16 => "u16",
        Scalar::U32 => "u32",
        Scalar::U64 => "u64",
        Scalar::F32 => "f32",
        Scalar::F64 => "f64",
    }
}

pub(super) fn value_text(v: Value) -> &'static str {
    match v {
        Value::Vec2 => "vec2",
        Value::Vec3 => "vec3",
        Value::Vec4 => "vec4",
        Value::Quat => "quat",
        Value::Mat34 => "mat34",
        Value::Color => "color",
    }
}
