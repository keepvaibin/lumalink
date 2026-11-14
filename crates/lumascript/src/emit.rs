//! The back end (docs/modding/language/compiled-form.md, sections 2 to 8): a checked mod (part b to f of the checker, [`Checked`])
//! written as C17 that clang builds with lumart (`rt/`) and the game's `luma.h` into the mod's library.
//!
//! What it writes (docs/modding/language/compiled-form.md, section 1, step 2):
//! - `lum/<module>.c` per module, each statement under a `#line <n> "<module>.lum"` marker, so a debugger, a crash
//!   report and a profile show the script's line;
//! - `lum/lm.h`, what the modules share: the value types, the import slots, the functions;
//! - `lum/imports.c`: the import list, generated from what the script uses (a game function it calls is one
//!   `LUMA_FUNCTION` row, a function it hooks one `LUMA_HOOK_ON` row per kind), sorted;
//! - `lum/mod.c`: the hooks the kernel calls, `start` and `stop`, and `luma_mod`;
//! - `lumart.h` and `lumart.c`, the run-time support, copied as they are.
//!
//! The C follows one shape everywhere (docs/modding/language/compiled-form.md, section 6): a generated function that can fail returns `int32_t` (0 ok, 1 a script
//! error, already reported through lumart) and gives its value through a last pointer parameter; it keeps every local
//! at its top, zeroed; a failing step sets the status and jumps to the function's one way out, `lm_out`, which
//! releases what the function owns. Every value an expression makes goes into a temporary of its own, in the order
//! the script reads them, so the order of reads and of the errors they may raise is the script's. Control flow is
//! C's `if` and `for (;;)` only (never `switch`), so the script's `break` and `continue` are C's.
//!
//! A construct this back end does not write yet is S0800 at its place, naming it: never a guess and never silence.

mod cside;
mod expr;
mod flow;
mod game;
mod class;
mod states;
mod json;
mod library;
mod service;
mod closure;
mod task;

use expr::{binop_text, expr_text};

use crate::ast::{self, Expr, HookKind, ItemKind, Pos};
use crate::check::{Checked, ItemRef, Library, LocalId, Owner, Resolved, Source, Ty};
use crate::ctype::{self, CType};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::{Game, GameTy, Scalar, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

/// lumart's two files, written beside the generated C.
pub const LUMART_H: &str = include_str!("../rt/lumart.h");
pub const LUMART_C: &str = include_str!("../rt/lumart.c");

/// The flags of abi 1.1 every platform shares (lumalink-core's build uses these; the back end's tests compile with them).
pub const C_FLAGS: &[&str] = &[
    "-std=c17",
    "-O2",
    "-nostdlib",
    "-ffreestanding",
    "-fno-stack-protector",
    "-fvisibility=hidden",
    "-fno-builtin-malloc",
    "-fno-common",
    "-Wall",
    "-Werror=implicit-function-declaration",
    // A stack frame over 4 KiB does not link: a script runs on the game's stack, inside its hooks.
    "-Wframe-larger-than=4096",
    "-Werror=frame-larger-than",
    // Unwind tables (.pdata on Windows, .eh_frame on ELF, compact unwind on Mach-O): the game can walk a crash
    // through a mod's frames. -ffreestanding alone gives none.
    "-fasynchronous-unwind-tables",
];

/// The generated files, by their path under the build's work folder.
#[derive(Clone, Debug, Default)]
pub struct Emitted {
    pub files: Vec<(String, String)>,
    /// The import rows, as written (name, signature, kind), for the build report.
    pub imports: Vec<(String, String, &'static str)>,
    /// Each game call passing made text to a `const char *` (docs/modding/language/compiled-form.md: the build report lists them, since
    /// each distinct text stays in memory until the mod is unloaded): source file, line, function.
    pub kept: Vec<(String, u32, String)>,
    /// The interface file `<mod id>.lumi` (docs/modding/language/compiled-form.md), when [`emit_with`] was given the
    /// modules' texts.
    pub lumi: Option<String>,
    /// The functions exported for other mods: (name, signature), sorted by name.
    pub exports: Vec<(String, String)>,
}

/// Writes the C of a checked mod. `checked` must have no error (notes are fine); every module of `sources` is the
/// mod's own, in the checker's order.
/// The checker's results with every content type but `nerve` (C2: `model`, `effect`, `sound`, `layout`, `font`,
/// `movie`) as `text`: a content value is its name at run time, so the back end writes it as any text.
fn content_as_text(checked: &Checked) -> Checked {
    fn text(t: &Ty) -> Ty {
        let b = |x: &Ty| Box::new(text(x));
        match t {
            Ty::Content(w) if w != "nerve" => Ty::Text,
            Ty::List(x) => Ty::List(b(x)),
            Ty::Set(x) => Ty::Set(b(x)),
            Ty::Opt(x) => Ty::Opt(b(x)),
            Ty::Ref(x) => Ty::Ref(b(x)),
            Ty::Pending(x) => Ty::Pending(b(x)),
            Ty::Range(x) => Ty::Range(b(x)),
            Ty::Map(k, v) => Ty::Map(b(k), b(v)),
            Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(text).collect()),
            Ty::Fn(ps, r) => Ty::Fn(ps.iter().map(text).collect(), b(r)),
            other => other.clone(),
        }
    }
    let mut c = checked.clone();
    for t in c.types.values_mut() {
        *t = text(t);
    }
    for t in c.exprs.values_mut() {
        *t = text(t);
    }
    for l in &mut c.locals {
        l.ty = text(&l.ty);
    }
    c
}

pub fn emit(mod_id: &str, sources: &[Source], checked: &Checked, game: &Game) -> Result<Emitted, Vec<(String, Diagnostic)>> {
    emit_with(mod_id, "", sources, &[], &[], checked, game)
}

/// As [`emit`], for a mod that may `use` library mods (`libraries`, as the checker had them) and whose interface file
/// is written from `texts` (each module's source text, in the order of `sources`; none: no interface file).
#[allow(clippy::too_many_arguments)]
pub fn emit_with(
    mod_id: &str,
    version: &str,
    sources: &[Source],
    texts: &[String],
    libraries: &[Library],
    checked: &Checked,
    game: &Game,
) -> Result<Emitted, Vec<(String, Diagnostic)>> {
    // C2: a content value (`model`, `sound`...) is its name, a text, from here on.
    let erased = content_as_text(checked);
    let checked = &erased;
    let mut g = Gen {
        mod_id: mod_id.to_string(),
        own_models: checked.own_models.clone(),
        libraries,
        mod_fns: BTreeMap::new(),
        svc_slots: BTreeMap::new(),
        svc_type_headers: std::collections::BTreeSet::new(),
        svc_events: BTreeMap::new(),
        lib_exports: Vec::new(),
        in_fns: HashMap::new(),
        lumi_sigs: BTreeMap::new(),
        export_code: String::new(),
        sources,
        checked,
        game,
        diags: Vec::new(),
        fn_slots: BTreeMap::new(),
        method_slots: BTreeMap::new(),
        fields: BTreeMap::new(),
        game_globals: BTreeMap::new(),
        classes: BTreeMap::new(),
        sizes: BTreeMap::new(),
        descs: HashMap::new(),
        closures: String::new(),
        closure_n: 0,
        fn_values: HashSet::new(),
        kept: Vec::new(),
        literals: HashMap::new(),
        literal_defs: Vec::new(),
        counted_globals: Vec::new(),
        inside: BTreeMap::new(),
        groups: BTreeMap::new(),
        protos: Vec::new(),
        module_code: Vec::new(),
        starts: Vec::new(),
        class_defines: Vec::new(),
        frames: Vec::new(),
        stages: Vec::new(),
        options: Vec::new(),
        option_ons: Vec::new(),
        load: None,
        layers: None,
        draws: Vec::new(),
        step: None,
        tasks_used: false,
        start_ret: false,
        c_headers: std::collections::BTreeSet::new(),
        c_protos: std::collections::BTreeSet::new(),
        exports: Vec::new(),
        stops: Vec::new(),
        gones: Vec::new(),
        inits: Vec::new(),
        globals: Vec::new(),
        typedefs: Vec::new(),
        type_names: HashMap::new(),
        defining: HashSet::new(),
        hooks_written: 0,
        json_readers: HashMap::new(),
        writers: HashMap::new(),
        writer_inline: false,
        pre_read: None,
        reader_inline: false,
        json_code: String::new(),
    };
    LOST.with(|l| l.borrow_mut().clear());
    COUNTED.with(|c| {
        let mut c = c.borrow_mut();
        c.clear();
        c.insert("lumart_text".to_string());
        c.insert("lumart_list".to_string());
        c.insert("lumart_map".to_string());
        c.insert("lumart_sorter".to_string());
        c.insert("lumart_bytes".to_string());
        c.insert("lumart_fn".to_string());
    });
    for m in 0..sources.len() {
        let code = g.module(m);
        g.module_code.push(code);
    }
    // A statement dropped with nothing said on its line: said here, so the build stops.
    for (m, at) in LOST.with(|l| std::mem::take(&mut *l.borrow_mut())) {
        let path = &sources[m].path;
        if !g.diags.iter().any(|(p, d)| p == path && d.at.line == at.line) {
            g.say(m, at, "this construct");
        }
    }
    if !g.diags.is_empty() {
        return Err(g.diags);
    }
    // The standard module (last, when given) is every mod's own: no part of an interface.
    let own = sources.iter().filter(|s| s.path != crate::check::STD_PATH).count();
    let lumi = (!texts.is_empty() && texts.len() == own).then(|| {
        let modules: Vec<(&str, &str, &ast::Module)> = sources.iter().zip(texts).map(|(s, t)| (s.path.as_str(), t.as_str(), &s.module)).collect();
        crate::lumi::write(mod_id, version, &modules, &|m, f| g.lumi_sigs.get(&(m, f.to_string())).cloned())
    });
    let mut out = g.files(mod_id);
    out.lumi = lumi;
    Ok(out)
}

/// One hook body: its C name and the place of its head, for the reports a hook makes about itself (S0901).
#[derive(Clone, Debug)]
struct Body {
    c_name: String,
    file: String,
    line: u32,
}

/// What a hook group needs to write its wrapper: the hooked function's result in C (None: no result).
#[derive(Clone, Debug)]
struct Group {
    result_c: Option<String>,
    bodies: Vec<Body>,
}

struct Gen<'a> {
    mod_id: String,
    /// The mod's own models ([`Checked::own_models`]): `model "x"` of one is "Luma/<mod id>/x".
    own_models: std::collections::BTreeSet<String>,
    /// The library mods the mod uses, as the checker had them (`Owner::Library` indexes them).
    libraries: &'a [Library],
    /// Each library function called: ("<mod id>/<module>.<fn>", signature) to its slot (`lm_mf_<n>`, kind 6).
    mod_fns: BTreeMap<(String, String), String>,
    /// Each service called: its name to its kind 5 slot, table type, header and version (emit/service.rs).
    svc_slots: BTreeMap<String, service::SvcSlot>,
    /// Headers of services whose types this mod writes without calling them (a library's struct holding a
    /// `draw.view.Camera`): included in lm.h, no import row.
    svc_type_headers: std::collections::BTreeSet<String>,
    /// What a request's event needs, made once each (emit/service.rs): its record's fill function by service and kind,
    /// a waited request's box, frame and step by the value's C type.
    svc_events: BTreeMap<String, String>,
    /// The mod's exported functions: (name, signature, wrapper), in the order written (sorted when listed).
    lib_exports: Vec<(String, String, String)>,
    /// Each type's copy into this mod's memory (library.rs `in_fn`), by the type: `lm_in_<n>`.
    in_fns: HashMap<String, String>,
    /// Each exported function's signature by (module, name), for the interface file.
    lumi_sigs: BTreeMap<(usize, String), String>,
    /// The export wrappers' C (lum/exports.c).
    export_code: String,
    sources: &'a [Source],
    checked: &'a Checked,
    game: &'a Game,
    diags: Vec<(String, Diagnostic)>,
    /// Each game function called: (name, signature) to its slot's C name.
    fn_slots: BTreeMap<(String, String), String>,
    /// Each virtual called on an object (docs/modding/language/objects.md): (row name, signature) to its slot (`lm_vm_<n>`, kind 8).
    method_slots: BTreeMap<(String, String), String>,
    /// Each game field used: ("<record>.<field>", its type text) to its offset slot (`lm_off_<n>`, kind 1).
    fields: BTreeMap<(String, String), String>,
    /// Each game global used: (name, type text) to its address slot (`lm_gl_<n>`, kind 7).
    game_globals: BTreeMap<(String, String), String>,
    /// Each game class named (is_a, as_class): its record to its number slot (`lm_class_<n>`, kind 3).
    classes: BTreeMap<String, String>,
    /// Each game record whose size a place needs (an array's element): its row name to its size slot (`lm_size_<n>`,
    /// kind 2).
    sizes: BTreeMap<String, String>,
    kept: Vec<(String, u32, String)>,
    /// Each element C type's descriptor in lm.h (lumart_desc): its name.
    descs: HashMap<String, String>,
    /// Function values' C (closure.rs): each closure's function and block, and each mod function's value wrapper,
    /// written into the module being written.
    closures: String,
    closure_n: u32,
    fn_values: HashSet<String>,
    /// The module being written's text literals: bytes to their static block (`lm_s<module>_<n>`), and their C.
    literals: HashMap<Vec<u8>, String>,
    literal_defs: Vec<String>,
    /// Module values of counted types, released when the mod stops: (global, C type).
    counted_globals: Vec<(String, String)>,
    /// Each game function hooked: (name, signature) to its re-entry flag (docs/modding/language/compiled-form.md, section 4).
    inside: BTreeMap<(String, String), String>,
    /// Hooks by (function, signature, kind), each group's bodies in the order written (modules in order, items in
    /// order): one `LumaHook` per group (docs/modding/language/compiled-form.md, section 4, order within one mod).
    groups: BTreeMap<(String, String, u32), Group>,
    /// Declarations of every generated function, for `lm.h`.
    protos: Vec<String>,
    module_code: Vec<String>,
    /// `on start` and `on stop` handlers' C names, in module order.
    starts: Vec<String>,
    /// Each class's define function (class.rs), called by the mod's start before the modules' initialisers.
    class_defines: Vec<String>,
    /// The `on frame` bodies, in module and source order: run by the mod's one frame handler (time's `on_frame`).
    frames: Vec<String>,
    /// `on stage "<name>"` handlers (section 10) with their stage's name, in module and source order: the frame
    /// handler runs each on the first frame a scene of that stage plays after it loaded.
    stages: Vec<(String, String)>,
    /// Each `option` of the mod: (key, its global, C type, type), read at start and on its OPTION event.
    options: Vec<(String, String, String, Ty)>,
    /// `on option <key>` handlers: (key, C name), in module and source order.
    option_ons: Vec<(String, String)>,
    /// The mod's one `on load` body (section 10): run by start after the modules' initialisers, before `on start`.
    load: Option<String>,
    /// The mod's one `on layers` body: run by LumaModInfo's `layers`, before start.
    layers: Option<String>,
    /// `on draw <point>` bodies: (the point's number, C name), in module and source order.
    draws: Vec<(i64, String)>,
    /// The mod's one `on step` body: run by its step handler (time's `on_step`).
    step: Option<String>,
    /// A task or a waiting handler was written: the frame handler runs lumart's tasks (11.1).
    tasks_used: bool,
    /// The next mod call is `start`'s: it gives the task's handle ([`Gen::start_expr`]).
    start_ret: bool,
    /// `use c` headers (paths from `source/`), included by lm.h (docs/modding/language/language.md, section 13).
    c_headers: std::collections::BTreeSet<String>,
    /// `extern c fn` prototypes, in lm.h.
    c_protos: std::collections::BTreeSet<String>,
    /// `export c fn` prototypes, in lum/lm_exports.h for the mod's C.
    exports: Vec<String>,
    stops: Vec<String>,
    /// `on mods.stopped(m)` handlers, called by the mod's dependent_gone (docs/modding/lumalink/loader.md).
    gones: Vec<String>,
    /// Each module's initialiser of its `let`s and `var`s, in module order.
    inits: Vec<String>,
    /// Module values, declared in `lm.h`.
    globals: Vec<String>,
    /// The C types of the mod's values (tuples, options, structs, enums), each after the types it holds.
    typedefs: Vec<String>,
    /// A type's key (its debug form) to its C name.
    type_names: HashMap<String, String>,
    /// Structs and enums being defined (a value cannot hold itself; the checker refuses it, this only stops a loop).
    defining: HashSet<ItemRef>,
    hooks_written: usize,
    /// json.decode's reader of each type (its debug form) to its C name (`lm_jd_<n>`).
    json_readers: HashMap<String, String>,
    /// value_text's writer of each struct or enum with values, as text or json, to its C name (`lm_tw_` / `lm_jw_`).
    writers: HashMap<String, String>,
    /// The next value_text writes its struct or enum in place (the writer function's own body); any other calls it.
    writer_inline: bool,
    /// `a?.m()` on a builtin method: `a`'s expression and its value, read once already (the method reads it as its
    /// receiver, of the option's inner type).
    pre_read: Option<(u32, String)>,
    /// The same for json_read and a reader function's body.
    reader_inline: bool,
    /// Readers and writers made while a module is written, put in that module's file.
    json_code: String,
}

/// A function being written.
struct Fx {
    /// The closures being written in place, innermost last: where `return` puts the value (none for a closure giving
    /// nothing), the label after the body, the defer scopes outside the closure.
    inline_ret: Vec<(Option<(String, Ty)>, String, usize)>,
    /// The checks a container place made (an index in bounds, a key found, the container made this variable's own),
    /// as their C lines: made again after the value is read and before the store, since the value may have changed the
    /// container or taken a count of it ([`Fx::recheck_since`]).
    rechecks: Vec<Vec<String>>,
    /// Set by a write (assign_at, place) for the one game_place_of call that follows: a field of a game struct copy
    /// is then reached through the copy made this variable's own (copy on write), not a counted read of it.
    writing: bool,
    /// Counted temporaries made in the statement being written, released at its end ([`Fx::release_since`]).
    live: Vec<(String, String)>,
    /// `<module>.lum`, the file of every `#line` marker and report.
    file: String,
    module: usize,
    decls: Vec<String>,
    lines: Vec<String>,
    /// Text builders to free on the way out.
    builders: Vec<String>,
    tmp: u32,
    depth: usize,
    /// Locals whose C place is not `l<id>` (a hook's parameters, an after's result, a `var` parameter).
    places: HashMap<LocalId, String>,
    /// Locals declared at the top already (parameters are declared by the head).
    declared: HashSet<LocalId>,
    hook: Option<HookFx>,
    /// A function's result: its C type and type (None: it gives none).
    ret: Option<(String, Ty)>,
    /// In a struct's function: `self` as C.
    self_c: Option<String>,
    /// The defers of each open block: (flag, statement), in the order reached.
    defer_scopes: Vec<Vec<(String, ast::Stmt)>>,
    /// Every defer of the function, for its way out.
    all_defers: Vec<(String, ast::Stmt)>,
    /// For each open loop: the number of open blocks outside it.
    loops: Vec<usize>,
    /// Lines after `lm_out`: the defers still set.
    tail: Vec<String>,
    /// A task's body (or an `on start`, which becomes one when it waits): `wait` and `stop` may be written.
    task: bool,
    /// A task with a result (11.1a): `return v` keeps `v` (its C type, its type) through `lm_resv`.
    task_ret: Option<(String, Ty)>,
    /// A writer or reader function of a type: its reports name the call's place, given as `file`, `line`, `src`.
    site: bool,
    /// The function writes in the scratch (docs/modding/language/language.md, section 6.4): it takes the scratch's mark `lm_sm` at its start and gives
    /// back to it on its way out (and at a task's park), so a script error between a use and its release loses nothing.
    scratch: bool,
    /// The waits written, each `lm_w<k>` (task.rs).
    waits: u32,
    /// An override's body (class.rs): the class whose data `lm_d` is, `self`'s.
    class_data: Option<ItemRef>,
    /// An override's body giving a value: the call's result C type, set by `return v` after `*lm_ret`.
    override_rc: Option<String>,
    /// An override's parameters in order: the local naming each (none: `_`), its slot in the call, its game type.
    override_params: Vec<(Option<LocalId>, String, GameTy)>,
    /// A machine's own block (states.rs): the machine and its C (`lm_m`), where `go` and `stop` act.
    machine: Option<(ItemRef, String)>,
    /// Temporaries and builders whose statement has ended, free to be used again, by their declared type (`ct`,
    /// `ct[n]`, `lumart_sb`): a function declares as many as one statement needs at once, not one per use, so its C
    /// frame stays within lumart's 4 KiB.
    free: HashMap<String, Vec<String>>,
    /// Every temporary and builder made, (name, declared type), in order: a statement's own go to `free` at its end.
    made: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct HookFx {
    kind: HookKind,
    /// The result's C type (None: the function gives none).
    result_c: Option<String>,
    /// The `const char *` parameters read as `text?`: (the local, its value as read, the
    /// argument's index, the parameter's name, the C type); written back to the call before the game's function runs
    /// when changed ([`Fx::text_args_back`]).
    texts: Vec<(String, String, usize, String, String)>,
    /// The `T *` parameters to a number, vector or enum read as `T?` ([`place_param`]): (the local, the pointer, the
    /// pointee's C type, const); each value written back through its pointer ([`Fx::place_args_back`]).
    outs: Vec<(String, String, String, bool)>,
    /// The hooked function's name and the hook's line (the kept text's report).
    name: String,
    line: u32,
}

/// An expression that did not write: its statement is dropped. Made by a function that has said S0800 at its place,
/// or by [`lost`], which notes the statement being written so that [`emit`] says S0800 there when nothing else was
/// said on its line: a dropped statement never builds silently.
struct NotWritten;

thread_local! {
    /// The statement or item being written: (module, place).
    static AT: std::cell::Cell<(usize, Pos)> = const { std::cell::Cell::new((0, Pos { line: 1, col: 1 })) };
    /// The places of statements dropped by [`lost`].
    static LOST: std::cell::RefCell<Vec<(usize, Pos)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The C types that hold counted values (text, and every aggregate holding one): each has `lm_ret_<T>`,
    /// `lm_rel_<T>` and `lm_cp_<T>` (lumart.h for `lumart_text`, lm.h for the rest).
    static COUNTED: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
}

/// Whether C type `ct` holds a counted value (its stores retain, its variables are released on the way out).
/// A `T *` parameter to a number, vector or enum as a hook or override has it (GameTy::Opt of a place): the pointee's C
/// type and whether it is const.
fn place_param(g: &GameTy, c: &CType) -> Option<(String, bool)> {
    let (GameTy::Opt(x), CType::Ptr { .. }) = (g, c) else { return None };
    let GameTy::Place { of, konst } = &**x else { return None };
    let et = match &**of {
        GameTy::Scalar(s) => scalar_c(*s),
        GameTy::Value(v) => value_c(*v),
        GameTy::Enum(_) => "int32_t",
        _ => return None,
    };
    Some((et.to_string(), *konst))
}

fn counted(ct: &str) -> bool {
    COUNTED.with(|c| c.borrow().contains(ct))
}

fn mark_counted(ct: &str) {
    COUNTED.with(|c| c.borrow_mut().insert(ct.to_string()));
}

/// `place = value` for a value of C type `ct`: a counted one is copied with its count taken (`lm_cp_<T>`, which
/// retains the new value before it releases the old), anything else assigned.
fn assign_line(place: &str, value: &str, ct: &str) -> String {
    if counted(ct) {
        format!("lm_cp_{ct}(&{place}, {value});")
    } else {
        format!("{place} = {value};")
    }
}

/// A construct the back end has no C for, found where no position was at hand: noted at the statement being written.
fn lost() -> NotWritten {
    LOST.with(|l| l.borrow_mut().push(AT.with(std::cell::Cell::get)));
    NotWritten
}

/// Marks the statement or item being written (for [`lost`]).
fn writing(module: usize, at: Pos) {
    AT.with(|a| a.set((module, at)));
}

type Ex = Result<String, NotWritten>;

fn scalar_c(s: Scalar) -> &'static str {
    match s {
        Scalar::Bool => "uint8_t",
        Scalar::I8 => "int8_t",
        Scalar::I16 => "int16_t",
        Scalar::I32 => "int32_t",
        Scalar::I64 => "int64_t",
        Scalar::U8 => "uint8_t",
        Scalar::U16 => "uint16_t",
        Scalar::U32 => "uint32_t",
        Scalar::U64 => "uint64_t",
        Scalar::F32 => "float",
        Scalar::F64 => "double",
    }
}

fn scalar_name(s: Scalar) -> &'static str {
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

fn signed(s: Scalar) -> bool {
    matches!(s, Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::I64)
}

fn value_c(v: Value) -> &'static str {
    match v {
        Value::Vec2 => "lm_vec2",
        Value::Vec3 => "lm_vec3",
        Value::Vec4 => "lm_vec4",
        Value::Quat => "lm_quat",
        Value::Mat34 => "lm_mat34",
        Value::Color => "lm_color",
    }
}

fn value_name(v: Value) -> &'static str {
    match v {
        Value::Vec2 => "vec2",
        Value::Vec3 => "vec3",
        Value::Vec4 => "vec4",
        Value::Quat => "quat",
        Value::Mat34 => "mat34",
        Value::Color => "color",
    }
}

/// The components of a vector-like value, by name.
fn components(v: Value) -> &'static [&'static str] {
    match v {
        Value::Vec2 => &["x", "y"],
        Value::Vec3 => &["x", "y", "z"],
        Value::Vec4 | Value::Quat => &["x", "y", "z", "w"],
        Value::Color => &["r", "g", "b", "a"],
        Value::Mat34 => &[],
    }
}

/// Whether a type is a game view: a pointer in C, `none` a null one.
fn is_view(t: &Ty) -> bool {
    matches!(t, Ty::Game(GameTy::View { .. }))
}

/// A C string literal of `s`, bytes outside printable ASCII as octal escapes (never a hex escape, which would eat a
/// following hex digit).
pub fn c_string(s: &str) -> String {
    c_bytes(s.as_bytes())
}

/// A C string literal of these bytes.
pub fn c_bytes(s: &[u8]) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for &b in s {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'?' => out.push_str("\\?"), // no trigraphs
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push('"');
    out
}

/// A string literal's text with its escapes read. The checker has refused any escape it does not take.
pub fn unescape(raw: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'0' => out.push(0),
            b'x' if i + 2 <= b.len() => {
                if let Ok(v) = u8::from_str_radix(&raw[i..i + 2], 16) {
                    out.push(v);
                }
                i += 2;
            }
            b'u' if i < b.len() && b[i] == b'{' => {
                let end = raw[i..].find('}').map_or(b.len(), |e| i + e);
                if let Some(ch) = u32::from_str_radix(&raw[i + 1..end], 16).ok().and_then(char::from_u32) {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                i = end + 1;
            }
            other => out.push(other), // \\ \" \' \{ \}
        }
    }
    out
}

/// A module's path as a C name part: `ui/hud` is `ui_hud`.
fn module_c(path: &str) -> String {
    path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

impl<'a> Gen<'a> {
    fn say(&mut self, module: usize, at: Pos, what: &str) {
        let d = Diagnostic::new(code::BACKEND_NOT_YET, at).with("what", Arg::Text(what.to_string()));
        self.diags.push((self.sources[module].path.clone(), d));
    }

    fn ty(&self, m: usize, e: &Expr) -> Ty {
        self.checked.exprs.get(&(m, e.id)).cloned().unwrap_or(Ty::Error)
    }

    fn resolved(&self, m: usize, e: &Expr) -> Option<&'a Resolved> {
        self.checked.resolved.get(&(m, e.id))
    }

    fn item(&self, item: ItemRef) -> &'a ast::Item {
        let modules = match item.owner {
            Owner::Mod => self.sources,
            Owner::Library(l) => &self.libraries[l].modules[..],
        };
        &modules[item.module].module.items[item.item]
    }

    /// The C name of an item of the mod: `lm_<module>__<name>` (docs/modding/language/compiled-form.md, section 2); of a library's (its types, as this mod's C
    /// holds them): `lml<n>_<module>__<name>`.
    fn item_c(&self, item: ItemRef, name: &str) -> String {
        match item.owner {
            Owner::Mod => format!("lm_{}__{}", module_c(&self.sources[item.module].path), name),
            Owner::Library(l) => format!("lml{l}_{}__{}", module_c(&self.libraries[l].modules[item.module].path), name),
        }
    }

    fn declared_ty(&self, item: ItemRef, key: &str) -> Ty {
        self.checked.types.get(&(item, key.to_string())).cloned().unwrap_or(Ty::Error)
    }

    /// The C type of a struct's field or a case's (`key` ".f" or "Case.f").
    fn field_c(&mut self, item: ItemRef, key: &str) -> Result<String, NotWritten> {
        let t = self.declared_ty(item, key);
        self.c_type(&t).ok_or_else(lost)
    }

    // ------------------------------------------------------------ types (docs/modding/language/compiled-form.md, section 5)

    /// The C type of a LumaScript type, or None when the back end does not hold it yet. `Unit` and `Never` have none.
    fn c_type(&mut self, t: &Ty) -> Option<String> {
        Some(match t {
            Ty::Text => "lumart_text".into(),
            // A json.Value: its canonical JSON text.
            Ty::Json => "lumart_text".into(),
            // Counted blocks; their elements' C types are made where an element is read or written (a struct may
            // hold a list of itself).
            Ty::List(_) => "lumart_list".into(),
            Ty::Map(..) | Ty::Set(_) => "lumart_map".into(),
            Ty::Bool => "uint8_t".into(),
            Ty::Int(s) | Ty::Float(s) => scalar_c(*s).into(),
            Ty::Value(v) => value_c(*v).into(),
            Ty::Game(GameTy::Enum(_)) => "int32_t".into(),
            Ty::Game(GameTy::View { .. }) => "void*".into(),
            // A mod's class's object: the game object it is (docs/modding/language/classes.md).
            Ty::Class(_) => "void*".into(),
            // A plain game record by value: the script's copy, a counted block of its size row's bytes, shared
            // until written as bytes are.
            Ty::Game(GameTy::Struct(_)) => "lumart_bytes".into(),
            // A handle (docs/modding/language/compiled-form.md, section 7.3): a slot of the mod's table and its generation; copied as it is, never counted.
            Ty::Ref(_) => "lumart_ref".into(),
            Ty::Bytes => "lumart_bytes".into(),
            Ty::Task => "lumart_task".into(),
            Ty::Opt(x) if is_view(x) => "void*".into(),
            Ty::Opt(x) => {
                let inner = self.c_type(x)?;
                let known = self.type_names.contains_key(&format!("{t:?}"));
                let name = self.typedef(t, |name| format!("typedef struct {name} {{\n    uint8_t some;\n    {inner} value;\n}} {name};"));
                if !known {
                    self.counted_helpers(&name, &[(Some("p->some".to_string()), vec![("value".to_string(), inner)])]);
                }
                name
            }
            Ty::Tuple(xs) => {
                let mut fields = Vec::new();
                let mut parts = Vec::new();
                for (i, x) in xs.iter().enumerate() {
                    let ct = self.c_type(x)?;
                    fields.push(format!("    {ct} _{i};"));
                    parts.push((format!("_{i}"), ct));
                }
                let body = fields.join("\n");
                let known = self.type_names.contains_key(&format!("{t:?}"));
                let name = self.typedef(t, |name| format!("typedef struct {name} {{\n{body}\n}} {name};"));
                if !known {
                    self.counted_helpers(&name, &[(None, parts)]);
                }
                name
            }
            Ty::Struct(item) => return self.struct_type(*item),
            Ty::Enum(item) => return self.enum_type(*item),
            Ty::Service { service, name } => return self.service_type_c(service, name),
            // A function value (docs/modding/language/language.md, section 5.4; closure.rs): a counted block holding its C function and captures.
            Ty::Fn(..) => "lumart_fn".into(),
            // A task's result to come (11.1a): its handle and its maker's operations, holding nothing counted.
            Ty::Pending(_) => "lumart_pending".into(),
            _ => return None,
        })
    }

    /// For an aggregate `name` holding counted values: `lm_ret_`, `lm_rel_` and `lm_cp_<name>`, written after its
    /// typedef (each part's helpers come before, its type having been made first). `groups`: (a condition on `p`
    /// under which the parts are there, the parts as (member path, C type)). Nothing for one holding none.
    fn counted_helpers(&mut self, name: &str, groups: &[(Option<String>, Vec<(String, String)>)]) {
        let mut ret = String::new();
        let mut rel = String::new();
        for (cond, parts) in groups {
            let parts: Vec<&(String, String)> = parts.iter().filter(|(_, ct)| counted(ct)).collect();
            if parts.is_empty() {
                continue;
            }
            let (open, close) = match cond {
                Some(c) => (format!("    if ({c}) {{\n"), "    }\n".to_string()),
                None => (String::new(), String::new()),
            };
            let pad = if cond.is_some() { "        " } else { "    " };
            ret.push_str(&open);
            rel.push_str(&open);
            for (path, ct) in parts {
                let _ = writeln!(ret, "{pad}lm_ret_{ct}(&p->{path});");
                let _ = writeln!(rel, "{pad}lm_rel_{ct}(&p->{path});");
            }
            ret.push_str(&close);
            rel.push_str(&close);
        }
        if ret.is_empty() {
            return;
        }
        mark_counted(name);
        self.typedefs.push(format!(
            "static inline void lm_ret_{name}(const {name}* p) {{\n{ret}}}\n\
             static inline void lm_rel_{name}({name}* p) {{\n{rel}    *p = ({name}){{0}};\n}}\n\
             static inline void lm_cp_{name}({name}* d, {name} s) {{\n    lm_ret_{name}(&s);\n    {name} old = *d;\n    *d = s;\n    lm_rel_{name}(&old);\n}}"
        ));
    }

    /// A tuple's or an option's C type, defined once: `lm_t<n>`.
    fn typedef(&mut self, t: &Ty, def: impl FnOnce(&str) -> String) -> String {
        let key = format!("{t:?}");
        if let Some(n) = self.type_names.get(&key) {
            return n.clone();
        }
        let name = format!("lm_t{}", self.type_names.len());
        self.typedefs.push(def(&name));
        self.type_names.insert(key, name.clone());
        name
    }

    fn struct_type(&mut self, item: ItemRef) -> Option<String> {
        let ItemKind::Struct(s) = &self.item(item).kind else { return None };
        let name = self.item_c(item, &s.name.name);
        let key = format!("struct {item:?}");
        if self.type_names.contains_key(&key) {
            return Some(name);
        }
        if !self.defining.insert(item) {
            return None;
        }
        let mut fields = Vec::new();
        let mut parts = Vec::new();
        for f in &s.fields {
            let t = self.declared_ty(item, &format!(".{}", f.name.name));
            let ct = self.c_type(&t)?;
            fields.push(format!("    {ct} {};", f.name.name));
            parts.push((f.name.name.clone(), ct));
        }
        if fields.is_empty() {
            fields.push("    uint8_t lm_empty;".into());
        }
        self.defining.remove(&item);
        self.typedefs.push(format!("typedef struct {name} {{\n{}\n}} {name};", fields.join("\n")));
        self.type_names.insert(key, name.clone());
        self.counted_helpers(&name, &[(None, parts)]);
        Some(name)
    }

    /// An enum: its case as `tag` (the case's place, from 0), the fields of the case it holds in `u.<Case>`.
    fn enum_type(&mut self, item: ItemRef) -> Option<String> {
        let ItemKind::Enum(e) = &self.item(item).kind else { return None };
        let name = self.item_c(item, &e.name.name);
        let key = format!("enum {item:?}");
        if self.type_names.contains_key(&key) {
            return Some(name);
        }
        if !self.defining.insert(item) {
            return None;
        }
        let mut cases = Vec::new();
        let mut groups = Vec::new();
        for (k, c) in e.cases.iter().enumerate() {
            if c.fields.is_empty() {
                continue;
            }
            let mut fields = Vec::new();
            let mut parts = Vec::new();
            for f in &c.fields {
                let t = self.declared_ty(item, &format!("{}.{}", c.name.name, f.name.name));
                let ct = self.c_type(&t)?;
                fields.push(format!("{ct} {};", f.name.name));
                parts.push((format!("u.{}.{}", c.name.name, f.name.name), ct));
            }
            cases.push(format!("        struct {{ {} }} {};", fields.join(" "), c.name.name));
            groups.push((Some(format!("p->tag == {k}u")), parts));
        }
        self.defining.remove(&item);
        let union = if cases.is_empty() { String::new() } else { format!("\n    union {{\n{}\n    }} u;", cases.join("\n")) };
        self.typedefs.push(format!("typedef struct {name} {{\n    uint32_t tag;{union}\n}} {name};"));
        self.type_names.insert(key, name.clone());
        self.counted_helpers(&name, &groups);
        Some(name)
    }

    /// The number of an enum's case (its place in the declaration).
    fn case_tag(&self, item: ItemRef, case: &str) -> Option<usize> {
        let ItemKind::Enum(e) = &self.item(item).kind else { return None };
        e.cases.iter().position(|c| c.name.name == case)
    }

    fn slot(&mut self, name: &str, signature: &str) -> String {
        let n = self.fn_slots.len();
        self.fn_slots.entry((name.to_string(), signature.to_string())).or_insert_with(|| format!("lm_fn_{n}")).clone()
    }

    /// The kind 8 slot of virtual `name` (`signature`): LumaCore.method's function, which runs the object's own class's
    /// function (docs/modding/language/objects.md).
    fn method_slot(&mut self, name: &str, signature: &str) -> String {
        let n = self.method_slots.len();
        self.method_slots.entry((name.to_string(), signature.to_string())).or_insert_with(|| format!("lm_vm_{n}")).clone()
    }

    fn inside_flag(&mut self, name: &str, signature: &str) -> String {
        let n = self.inside.len();
        self.inside.entry((name.to_string(), signature.to_string())).or_insert_with(|| format!("lm_inside_{n}")).clone()
    }

    // ------------------------------------------------------------ modules

    fn module(&mut self, m: usize) -> String {
        let src = &self.sources[m].module;
        let file = format!("{}.lum", self.sources[m].path);
        let mut out = String::new();
        self.literals.clear();
        self.literal_defs.clear();
        let mut init = Fx::new(&file, m);
        for (i, item) in src.items.iter().enumerate() {
            let me = ItemRef { owner: Owner::Mod, module: m, item: i };
            writing(m, item.at);
            match &item.kind {
                ItemKind::Use(ast::Use::C(header)) => self.use_c(header),
                ItemKind::Use(_) => {}
                ItemKind::Hook(h) => {
                    if h.every_override {
                        self.say(m, h.at, "hook ... every override of");
                        continue;
                    }
                    if let Some(text) = self.hook(m, me, h, &file) {
                        out.push_str(&text);
                    }
                }
                ItemKind::On(on) => match on.event {
                    ast::OnEvent::Start | ast::OnEvent::Stop | ast::OnEvent::Frame => {
                        let start = matches!(on.event, ast::OnEvent::Start);
                        let frame = matches!(on.event, ast::OnEvent::Frame);
                        let kind = if start { "on_start" } else if frame { "on_frame" } else { "on_stop" };
                        let name = format!("lm_{kind}_{m}_{}", self.frames.len() + self.starts.len());
                        let mut fx = Fx::new(&file, m);
                        // `on start` may wait: written as a task's body, a task when it does.
                        fx.task = start;
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        if fx.waits > 0 {
                            let text = self.start_task_handler(&fx, &name, on.at);
                            out.push_str(&text);
                        } else {
                            out.push_str(&fx.function(&format!("int32_t {name}(void)"), on.at));
                        }
                        self.protos.push(format!("int32_t {name}(void);"));
                        if start {
                            self.starts.push(name);
                        } else if frame {
                            self.frames.push(name);
                        } else {
                            self.stops.push(name);
                        }
                    }
                    ast::OnEvent::Event { ref names, .. } if on.event.is_kit_stopped() => {
                        let name = format!("lm_on_gone_{m}_{}", self.gones.len());
                        let mut fx = Fx::new(&file, m);
                        if let Some(n) = names.as_ref().and_then(|n| n.first()) {
                            if let Some(Resolved::Local(id)) = self.checked.named.get(&(m, n.at)) {
                                fx.places.insert(*id, "lm_gone".into());
                                fx.declared.insert(*id);
                            }
                        }
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        out.push_str(&fx.function(&format!("int32_t {name}(uint64_t lm_gone)"), on.at));
                        self.protos.push(format!("int32_t {name}(uint64_t lm_gone);"));
                        self.gones.push(name);
                    }
                    // `on stage "<name>"` (section 10): a stage became ready. Written as `on start` is (a task when it waits),
                    // run by the frame handler on the first frame time says PLAY after a frame it said BOOT or LOADING and
                    // the scenario's opening is over, when the game's current stage has that name.
                    ast::OnEvent::Stage(ref stage) => {
                        if stage.interpolates() {
                            self.say(m, on.at, "on stage with a name made while running");
                            continue;
                        }
                        let text: String = stage.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect();
                        let name = format!("lm_on_stage_{m}_{}", self.stages.len());
                        let mut fx = Fx::new(&file, m);
                        fx.task = true;
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        if fx.waits > 0 {
                            let code = self.start_task_handler(&fx, &name, on.at);
                            out.push_str(&code);
                        } else {
                            out.push_str(&fx.function(&format!("int32_t {name}(void)"), on.at));
                        }
                        self.protos.push(format!("int32_t {name}(void);"));
                        self.slot("MR::getCurrentStageName", "const char *()");
                        // Ready as the test harness takes it: the scenario's opening (its starter and opening camera)
                        // is over, so a handler that plays a scene or takes the camera does not stall the opening.
                        self.slot("MR::isExecScenarioStarter", "bool()");
                        self.slot("MR::isStageStateScenarioOpeningCamera", "bool()");
                        self.stages.push((name, text));
                    }
                    // `on step` / `on step(now: time.Now)`: the mod's one, on time's on_step; never a task (the
                    // checker refused a wait). Its `now` is the step's own record, read through the pointer given.
                    ast::OnEvent::Step(ref param) => {
                        let name = format!("lm_on_step_{m}");
                        // The time service's header (LumaTimeNow) for lm.h, and its import row.
                        if self.service_slot("time").is_none() {
                            self.say(m, on.at, "on step without the time service's header");
                            continue;
                        }
                        let mut fx = Fx::new(&file, m);
                        if let Some(n) = param.as_ref().and_then(|p| p.name.as_ref()) {
                            if let Some(Resolved::Local(id)) = self.checked.named.get(&(m, n.at)) {
                                fx.places.insert(*id, "(*lm_now)".into());
                                fx.declared.insert(*id);
                            }
                        }
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        out.push_str(&fx.function(&format!("int32_t {name}(const LumaTimeNow* lm_now)"), on.at));
                        self.protos.push(format!("int32_t {name}(const LumaTimeNow* lm_now);"));
                        self.step = Some(name);
                    }
                    // `on option <key>` (section 10): the option changed on the Mods page. Written as `on stage` is (a
                    // task when it waits), run by the mod's options handler after the option's value is read again.
                    ast::OnEvent::Option(ref key) => {
                        if self.service_slot("mods").is_none() {
                            self.say(m, on.at, "on option without the mods service's header");
                            continue;
                        }
                        let name = format!("lm_on_option_{m}_{}", self.option_ons.len());
                        let mut fx = Fx::new(&file, m);
                        fx.task = true;
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        if fx.waits > 0 {
                            let code = self.start_task_handler(&fx, &name, on.at);
                            out.push_str(&code);
                        } else {
                            out.push_str(&fx.function(&format!("int32_t {name}(void)"), on.at));
                        }
                        self.protos.push(format!("int32_t {name}(void);"));
                        self.option_ons.push((key.name.clone(), name));
                    }
                    // `on load` (section 10): the mod's one, run by start after the modules' initialisers and before
                    // `on start`, the first of the mod's code, so before any stage that loads after the mod starts;
                    // never a task.
                    ast::OnEvent::Load => {
                        let name = format!("lm_on_load_{m}");
                        let mut fx = Fx::new(&file, m);
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        out.push_str(&fx.function(&format!("int32_t {name}(void)"), on.at));
                        self.protos.push(format!("int32_t {name}(void);"));
                        self.load = Some(name);
                    }
                    // `on draw <point>`: a body at that point of every frame (draw's `at`, one function a point a
                    // mod: the mod's bodies at one point run from it in order). Never a task.
                    ast::OnEvent::Draw(ref path) => {
                        let Some(Resolved::ServiceCase { number, .. }) = path.last().and_then(|c| self.checked.named.get(&(m, c.at))).cloned() else {
                            self.say(m, on.at, "on draw at this point");
                            continue;
                        };
                        if self.service_slot("draw").is_none() {
                            self.say(m, on.at, "on draw without the draw service's header");
                            continue;
                        }
                        let name = format!("lm_on_draw_{m}_{}", self.draws.len());
                        let mut fx = Fx::new(&file, m);
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        out.push_str(&fx.function(&format!("int32_t {name}(void)"), on.at));
                        self.protos.push(format!("int32_t {name}(void);"));
                        self.draws.push((number, name));
                    }
                    // `on layers -> list<text>`: the mod's one, the kernel's `layers` call before content loads;
                    // its names the `when.code` layers it turns on. Never a task.
                    ast::OnEvent::Layers(_) => {
                        let t = Ty::List(Box::new(Ty::Text));
                        let Some(ct) = self.c_type(&t) else {
                            self.say(m, on.at, "on layers");
                            continue;
                        };
                        let name = format!("lm_on_layers_{m}");
                        let mut fx = Fx::new(&file, m);
                        fx.ret = Some((ct.clone(), t));
                        self.block(&mut fx, &on.body);
                        self.out_defers(&mut fx);
                        let head = format!("int32_t {name}({ct}* lm_ret)");
                        out.push_str(&fx.function(&head, on.at));
                        self.protos.push(format!("{head};"));
                        self.layers = Some(name);
                    }
                    _ => self.say(m, on.at, &format!("on {}", on_name(&on.event))),
                },
                ItemKind::Binding(b) => {
                    // A module `let` or `var`: a global of the mod, set by the module's initialiser in order.
                    let ast::PatternKind::Path { path, args: None, fields: None } = &b.pattern.kind else {
                        self.say(m, b.at, "a module-level pattern");
                        continue;
                    };
                    let name = &path[0];
                    let t = self.checked.types.get(&(me, String::new())).cloned().or_else(|| b.value.as_ref().map(|v| self.ty(m, v))).unwrap_or(Ty::Error);
                    let Some(ct) = self.c_type(&t) else {
                        let what = format!("a module value of type {}", self.ty_text(&t));
                        self.say(m, b.at, &what);
                        continue;
                    };
                    let g = global_name(m, &name.name);
                    if counted(&ct) {
                        self.counted_globals.push((g.clone(), ct.clone()));
                    }
                    self.globals.push(format!("extern {ct} {g};"));
                    let _ = writeln!(out, "{ct} {g};");
                    if let Some(v) = &b.value {
                        init.line_marker(b.at);
                        if let Ok(x) = self.expr(&mut init, v) {
                            let from = self.ty(m, v);
                            if let Ok(x) = self.coerce(&mut init, &x, &from, &t, v.at) {
                                init.set(&g, &x, &ct);
                            }
                        }
                    }
                }
                ItemKind::Fn(f) => {
                    let c_name = self.item_c(me, &f.name.name);
                    if let Some(text) = self.function(m, me, "", &c_name, f, &file) {
                        out.push_str(&text);
                        // Every `pub fn` is the mod's for other mods too.
                        if item.public {
                            self.export(m, me, "", f, &c_name, false);
                        }
                    }
                }
                ItemKind::Struct(s) => {
                    for f in &s.fns {
                        let c_name = self.struct_fn_c(me, &f.name.name);
                        let key = format!(".{}", f.name.name);
                        if let Some(text) = self.function(m, me, &key, &c_name, f, &file) {
                            out.push_str(&text);
                            // A pub struct's functions are the mod's for other mods too (`<module>.<S>.<fn>`).
                            if item.public {
                                self.export(m, me, &key, f, &c_name, false);
                            }
                        }
                    }
                    // The defaults it makes for other mods (a function value, a list with elements).
                    if item.public {
                        for fd in &s.fields {
                            let Some(d) = &fd.default else { continue };
                            let ft = self.declared_ty(me, &format!(".{}", fd.name.name));
                            if !library::made_by_library(d, &ft) {
                                continue;
                            }
                            if let Some((text, c_name)) = self.default_maker(m, me, &s.name.name, &fd.name.name, d, &ft, &file) {
                                out.push_str(&text);
                                self.export_default(m, &s.name.name, &fd.name.name, &ft, &c_name);
                            }
                        }
                    }
                    if s.c {
                        self.say(m, s.name.at, "c struct");
                    }
                    if self.struct_type(me).is_none() {
                        self.say(m, s.name.at, "a struct holding a value of this kind");
                    }
                }
                ItemKind::Enum(e) => {
                    if self.enum_type(me).is_none() {
                        self.say(m, e.name.at, "an enum holding a value of this kind");
                    }
                }
                // `option <key>: <type>`: a global of the mod, read by start and again on the option's OPTION
                // event; a key option is its input action's handle.
                ItemKind::Option { key, .. } => {
                    let t = self.checked.types.get(&(me, String::new())).cloned().unwrap_or(Ty::Error);
                    let key_option = matches!(&t, Ty::Service { service, name } if service == "input" && name == "Action");
                    let ct = match self.c_type(&t) {
                        Some(ct) if matches!(t, Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Text) || key_option => ct,
                        _ => {
                            let what = format!("an option of type {}", self.ty_text(&t));
                            self.say(m, item.at, &what);
                            continue;
                        }
                    };
                    if self.service_slot("mods").is_none() || (key_option && self.service_slot("input").is_none()) {
                        self.say(m, item.at, "an option without the mods (or input) service's header");
                        continue;
                    }
                    let g = format!("lm_opt_{}", key.name);
                    if counted(&ct) {
                        self.counted_globals.push((g.clone(), ct.clone()));
                    }
                    self.globals.push(format!("extern {ct} {g};"));
                    let _ = writeln!(out, "{ct} {g};");
                    self.options.push((key.name.clone(), g, ct, t));
                }
                ItemKind::Class(c) => {
                    if let Some(text) = self.class_item(m, me, c, &file) {
                        out.push_str(&text);
                    }
                }
                ItemKind::Task { name, params, body, ret } => {
                    let f = ast::FnDecl { name: name.clone(), params: params.clone(), ret: None, body: body.clone() };
                    if let Some(text) = self.task(m, me, &f, &file) {
                        out.push_str(&text);
                        // A `pub task` is the mod's for other mods too: its call gives a pending (11.1a), of `()`
                        // without a result.
                        let _ = ret;
                        if item.public {
                            let c_name = self.task_c(me);
                            self.export(m, me, "", &f, &c_name, true);
                        }
                    }
                }
                // Written with its class (class.rs, states.rs), whose data holds it.
                ItemKind::States(_) => {}
                // A test runs under `lumalink test` (4.8), never in the mod's library.
                ItemKind::Test { .. } => {}
                ItemKind::Extern { name, params, ret } => self.extern_fn(m, me, name, params, ret.is_some()),
                ItemKind::Export { name, params, ret, body } => {
                    let f = ast::FnDecl { name: name.clone(), params: params.clone(), ret: ret.clone(), body: body.clone() };
                    if let Some(text) = self.export_fn(m, me, &f, &file) {
                        out.push_str(&text);
                    }
                }
            }
        }
        out.push_str(&std::mem::take(&mut self.json_code));
        out.push_str(&std::mem::take(&mut self.closures));
        let init_name = format!("lm_init_{m}");
        out.push_str(&init.function(&format!("int32_t {init_name}(void)"), Pos { line: 1, col: 1 }));
        self.protos.push(format!("int32_t {init_name}(void);"));
        self.inits.push(init_name);
        // The module's text literals, before the code that reads them.
        let mut head = String::new();
        let _ = writeln!(head, "/* Generated by LumaLink from source/{file}. */\n#include \"lm.h\"\n");
        for d in &self.literal_defs {
            let _ = writeln!(head, "{d}");
        }
        if !self.literal_defs.is_empty() {
            head.push('\n');
        }
        head + &out
    }

    /// A text literal of the module being written: a static block counted LUMART_FOREVER in the library's image, one
    /// per distinct bytes; its value as C (null for the empty text).
    fn text_literal(&mut self, m: usize, bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return "((lumart_text)0)".into();
        }
        if let Some(n) = self.literals.get(bytes) {
            return format!("LUMART_LIT({n})");
        }
        let n = format!("lm_s{m}_{}", self.literals.len());
        self.literal_defs.push(format!(
            "static const struct {{ uint32_t rc; uint32_t reserved; uint64_t len; char b[{}]; }} {n} = {{LUMART_FOREVER, 0, {}u, {}}};",
            bytes.len() + 1,
            bytes.len(),
            c_bytes(bytes)
        ));
        self.literals.insert(bytes.to_vec(), n.clone());
        format!("LUMART_LIT({n})")
    }

    // ------------------------------------------------------------ functions (docs/modding/language/compiled-form.md, sections 2, 6)

    /// The head of a mod's function `f` (`lm_<module>__f`): its parameters (a `var` one by its address) and, when it
    /// gives a value, `lm_ret`. None when a parameter's or the result's type has no C yet.
    fn fn_head(&mut self, item: ItemRef, key: &str, name: &str, f: &ast::FnDecl) -> Option<(String, Vec<(LocalId, bool)>, Option<(String, Ty)>, Option<String>)> {
        let m = item.module;
        let mut params = Vec::new();
        let mut locals = Vec::new();
        let mut self_c = None;
        for (i, p) in f.params.iter().enumerate() {
            if p.is_self {
                // A struct's `self`: its value, or its place's address for `var self`.
                let ct = self.c_type(&Ty::Struct(item))?;
                params.push(if p.var { format!("{ct}* lm_self") } else { format!("{ct} lm_self") });
                self_c = Some(if p.var { "(*lm_self)".to_string() } else { "lm_self".to_string() });
                continue;
            }
            let pname = p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone());
            let t = self.declared_ty(item, &format!("{key}({pname})"));
            let ct = self.c_type(&t)?;
            let id = match p.name.as_ref().and_then(|n| self.checked.named.get(&(m, n.at))) {
                Some(Resolved::Local(id)) => *id,
                _ => return None,
            };
            params.push(if p.var { format!("{ct}* l{id}") } else { format!("{ct} l{id}") });
            locals.push((id, p.var));
        }
        let ret = match self.checked.types.get(&(item, format!("{key}->"))) {
            Some(Ty::Unit) | None => None,
            Some(t) => {
                let t = t.clone();
                let ct = self.c_type(&t)?;
                params.push(format!("{ct}* lm_ret"));
                Some((ct, t))
            }
        };
        let head = format!("int32_t {name}({})", if params.is_empty() { "void".to_string() } else { params.join(", ") });
        Some((head, locals, ret, self_c))
    }

    /// A pub struct field's default as a function of its own (`<S>__<field>__default`, the value through `lm_ret`), for
    /// its export to other mods; None after a report when its expression is not written.
    #[allow(clippy::too_many_arguments)]
    fn default_maker(&mut self, m: usize, me: ItemRef, st: &str, field: &str, d: &Expr, t: &Ty, file: &str) -> Option<(String, String)> {
        let ct = self.c_type(t)?;
        let c_name = format!("{}__{field}__default", self.item_c(me, st));
        let mut fx = Fx::new(file, m);
        let v = self.expr(&mut fx, d).ok()?;
        let from = self.ty(m, d);
        let v = self.coerce(&mut fx, &v, &from, t, d.at).ok()?;
        fx.set("(*lm_ret)", &v, &ct);
        let head = format!("int32_t {c_name}({ct}* lm_ret)");
        self.protos.push(format!("{head};"));
        Some((fx.function(&head, d.at), c_name))
    }

    /// The C name of struct `S`'s function `f` in module `m`: `lm_m__S__f` (docs/modding/language/compiled-form.md, section 2's form; `__` never occurs in a name).
    fn struct_fn_c(&self, item: ItemRef, f: &str) -> String {
        let ItemKind::Struct(s) = &self.item(item).kind else { return String::new() };
        format!("{}__{f}", self.item_c(item, &s.name.name))
    }

    fn function(&mut self, m: usize, me: ItemRef, key: &str, c_name: &str, f: &ast::FnDecl, file: &str) -> Option<String> {
        let Some((head, params, ret, self_c)) = self.fn_head(me, key, c_name, f) else {
            self.say(m, f.name.at, "a function with a parameter or result of this type");
            return None;
        };
        let mut fx = Fx::new(file, m);
        for (id, var) in params {
            fx.declared.insert(id);
            if var {
                fx.places.insert(id, format!("(*l{id})"));
            }
        }
        fx.ret = ret;
        fx.self_c = self_c;
        self.block(&mut fx, &f.body);
        self.protos.push(format!("{head};"));
        self.out_defers(&mut fx);
        Some(fx.function(&head, f.name.at))
    }

    // ------------------------------------------------------------ hooks (docs/modding/language/compiled-form.md, section 4)

    /// One hook item: its body function, added to its (function, kind) group.
    fn hook(&mut self, m: usize, me: ItemRef, h: &ast::Hook, file: &str) -> Option<String> {
        let target = self.checked.hooks.get(&(me, String::new()))?.clone();
        let fnty = match self.game.hook_signature(&target.signature) {
            Ok(f) => f,
            Err(_) => {
                self.say(m, h.at, "a hook whose signature does not cross");
                return None;
            }
        };
        let sig = ctype::parse_signature(&target.signature).ok()?;
        let result_c = match &fnty.result {
            GameTy::Unit => None,
            GameTy::Scalar(s) => Some(scalar_c(*s).to_string()),
            GameTy::Value(v) => Some(value_c(*v).to_string()),
            GameTy::Enum(_) => Some("int32_t".to_string()),
            GameTy::Opt(_) if fnty.text16_result => Some("const uint16_t*".to_string()),
            GameTy::View { .. } | GameTy::Opt(_) => Some("void*".to_string()),
            other => {
                self.say(m, h.at, &format!("a hook on a function returning {other:?}"));
                return None;
            }
        };
        let kind: u32 = match h.kind {
            HookKind::Before => 0,
            HookKind::After => 1,
            HookKind::Wrap => 2,
            HookKind::Replace => 3,
        };
        self.inside_flag(&target.name, &target.signature);
        let key = (target.name.clone(), target.signature.clone(), kind);
        // docs/modding/language/compiled-form.md, section 2: `lm_hook_<n>`, n counting the mod's hooks in written order (modules in order).
        let c_name = format!("lm_hook_{}", self.hooks_written);
        self.hooks_written += 1;
        let mut fx = Fx::new(file, m);
        fx.hook = Some(HookFx { kind: h.kind, result_c: result_c.clone(), texts: Vec::new(), outs: Vec::new(), name: h.target.text.clone(), line: h.at.line });
        // The parameters: each a place in the call's arguments (docs/modding/language/compiled-form.md, section 4, addition 3).
        if let Some(params) = &h.params {
            for (i, p) in params.iter().enumerate() {
                let Some(name) = &p.name else { continue };
                let Some(Resolved::Local(id)) = self.checked.named.get(&(m, name.at)) else { continue };
                let (Some(g), Some(c)) = (fnty.params.get(i), sig.params.get(fnty.c_index(i))) else { continue };
                // A `const char *`: the script's `text?`, read from the game's bytes at the hook's start; a text
                // written to it goes back to the call as its kept copy (docs/modding/language/compiled-form.md) before the game's function.
                if let (GameTy::Opt(x), CType::Ptr { .. }) = (g, c) {
                    if **x == GameTy::Text {
                        let n = fnty.c_index(i) + 1;
                        let ct = self.c_type(&Ty::Opt(Box::new(Ty::Text)))?;
                        let (v, was, read) = (format!("l{id}"), format!("lm_ta{n}"), format!("lm_tw{n}"));
                        fx.decl(format!("{ct} {v};"));
                        fx.decl(format!("{ct} {was};"));
                        fx.decl(format!("const char* {read};"));
                        fx.emit(format!("{read} = *(const char**)call->args[{n}];"));
                        fx.emit(format!("if ({read} != 0) {{"));
                        fx.depth += 1;
                        fx.emit(format!("{v}.some = 1;"));
                        fx.fail_if(&format!("lumart_text_from_game(&{v}.value, {read}, {}, {}u) != 0", c_string(file), name.at.line));
                        fx.depth -= 1;
                        fx.emit("}".into());
                        fx.emit(format!("lm_cp_{ct}(&{was}, {v});"));
                        fx.declared.insert(*id);
                        if let Some(hk) = fx.hook.as_mut() {
                            hk.texts.push((v, was, n, name.name.clone(), ct));
                        }
                        continue;
                    }
                }
                if self.bind_place_param(&mut fx, *id, g, c, fnty.c_index(i) + 1)? {
                    continue;
                }
                match self.hook_param(g, c, fnty.c_index(i) + 1) {
                    Some(place) => {
                        fx.places.insert(*id, place);
                        fx.declared.insert(*id);
                    }
                    None => {
                        self.say(m, name.at, &format!("a hook parameter of type {g:?}"));
                        return None;
                    }
                }
            }
        }
        // An after's result name: the result as the game made it, read at the hook's start.
        if let (Some(ast::HookRet::Name(n)), Some(rc)) = (&h.ret, &result_c) {
            if let Some(Resolved::Local(id)) = self.checked.named.get(&(m, n.at)) {
                let v = format!("l{id}");
                fx.decl(format!("{rc} {v};"));
                fx.declared.insert(*id);
                fx.emit(format!("{v} = LUMA_RESULT(call, {rc});"));
                fx.places.insert(*id, v);
            }
        }
        self.block(&mut fx, &h.body);
        let head = match h.kind {
            HookKind::Before => format!("int32_t {c_name}(LumaCall* call, uint8_t* lm_skip)"),
            HookKind::After => format!("int32_t {c_name}(LumaCall* call)"),
            HookKind::Wrap | HookKind::Replace => format!("int32_t {c_name}(LumaCall* call, uint8_t* lm_proceeded, int32_t (*lm_next)(LumaCall*))"),
        };
        self.out_defers(&mut fx);
        fx.place_args_tail();
        // A before hook's changed texts go to the call on every way out but a script error's (the game's function
        // runs after it, a skip aside).
        if h.kind == HookKind::Before && fx.hook.as_ref().is_some_and(|hk| !hk.texts.is_empty()) {
            let lines = std::mem::take(&mut fx.lines);
            let depth = std::mem::replace(&mut fx.depth, 1);
            fx.emit("if (lm_st == 0) {".into());
            fx.depth += 1;
            fx.text_args_back(true);
            fx.depth -= 1;
            fx.emit("}".into());
            let back = std::mem::replace(&mut fx.lines, lines);
            fx.tail.extend(back);
            fx.depth = depth;
        }
        // A text written back is kept (only a before's or a wrap's reaches the call).
        if matches!(h.kind, HookKind::Before | HookKind::Wrap) {
            for (_, _, _, p, _) in fx.hook.as_ref().map(|hk| hk.texts.clone()).unwrap_or_default() {
                self.kept.push((file.to_string(), h.at.line, p));
            }
        }
        let text = fx.function(&head, h.at);
        self.protos.push(format!("{head};"));
        let group = self.groups.entry(key).or_insert_with(|| Group { result_c: result_c.clone(), bodies: Vec::new() });
        group.bodies.push(Body { c_name, file: file.to_string(), line: h.at.line });
        Some(text)
    }

    /// A hook's or override's `T *` parameter to a number, vector or enum (a nullable place, docs/modding/language/compiled-form.md, section 5: args[n] holds the
    /// pointer): the local of the script's `T?`, read through the pointer at the start (none for null); its value
    /// goes back through the pointer before the game's code runs on (proceed, base) and on the way out. false: `g` is
    /// no such parameter.
    fn bind_place_param(&mut self, fx: &mut Fx, id: LocalId, g: &GameTy, c: &CType, n: usize) -> Option<bool> {
        let Some((et, konst)) = place_param(g, c) else { return Some(false) };
        let ct = self.c_type(&self.checked.locals[id as usize].ty)?;
        let (v, p) = (format!("l{id}"), format!("lm_pp{n}"));
        fx.decl(format!("{ct} {v};"));
        fx.decl(format!("void* {p};"));
        fx.emit(format!("{p} = *(void**)call->args[{n}];"));
        fx.emit(format!("if ({p} != 0) {{ {v}.some = 1; {v}.value = *(const {et}*){p}; }}"));
        fx.declared.insert(id);
        if let Some(hk) = fx.hook.as_mut() {
            hk.outs.push((v, p, et, konst));
        }
        Some(true)
    }

    /// The C place of a hooked function's parameter `n` (from 1): `args[n]` is the address of the value's copy for a
    /// value parameter, the object's address for a reference, the pointer's own slot for a pointer (docs/modding/language/compiled-form.md).
    fn hook_param(&self, g: &GameTy, c: &CType, n: usize) -> Option<String> {
        let direct = |t: &str| format!("(*({t}*)call->args[{n}])");
        match (g, c) {
            (GameTy::Scalar(s), _) => Some(direct(scalar_c(*s))),
            (GameTy::Value(v), _) => Some(direct(value_c(*v))),
            (GameTy::Enum(_), _) => Some(direct("int32_t")),
            (GameTy::Place { of, .. }, CType::Ref(_) | CType::RRef(_)) => match of.as_ref() {
                GameTy::Scalar(s) => Some(direct(scalar_c(*s))),
                GameTy::Value(v) => Some(direct(value_c(*v))),
                GameTy::Enum(_) => Some(direct("int32_t")),
                _ => None,
            },
            // A pointer parameter: args[n] points at the pointer.
            (GameTy::View { .. } | GameTy::Opt(_), CType::Ptr { .. }) => Some(format!("(*(void**)call->args[{n}])")),
            // A reference to an object: args[n] is the object's address.
            (GameTy::View { .. }, CType::Ref(_) | CType::RRef(_)) => Some(format!("call->args[{n}]")),
            _ => None,
        }
    }

    fn ty_text(&self, t: &Ty) -> String {
        match t {
            Ty::Bool => "bool".into(),
            Ty::Int(s) | Ty::Float(s) => scalar_name(*s).into(),
            Ty::Value(v) => value_name(*v).into(),
            Ty::Text => "text".into(),
            Ty::Bytes => "bytes".into(),
            Ty::List(x) => format!("list<{}>", self.ty_text(x)),
            Ty::Map(k, v) => format!("map<{}, {}>", self.ty_text(k), self.ty_text(v)),
            Ty::Set(x) => format!("set<{}>", self.ty_text(x)),
            Ty::Opt(x) => format!("{}?", self.ty_text(x)),
            Ty::Tuple(xs) => format!("({})", xs.iter().map(|x| self.ty_text(x)).collect::<Vec<_>>().join(", ")),
            Ty::Struct(i) | Ty::Enum(i) | Ty::Class(i) => match &self.item(*i).kind {
                ItemKind::Struct(s) => s.name.name.clone(),
                ItemKind::Enum(e) => e.name.name.clone(),
                ItemKind::Class(c) => c.name.name.clone(),
                _ => "?".into(),
            },
            Ty::Game(GameTy::View { record, .. }) | Ty::Game(GameTy::Struct(record)) => record.clone(),
            Ty::Json => "json.Value".into(),
            Ty::Ref(x) => format!("ref<{}>", self.ty_text(x)),
            Ty::Fn(..) => "fn".into(),
            other => format!("{other:?}"),
        }
    }

    // ------------------------------------------------------------ the files

    fn files(self, mod_id: &str) -> Emitted {
        let mut files = Vec::new();
        let mut imports = Vec::new();
        // lm.h
        let mut h = String::new();
        h.push_str("/* Generated by LumaLink: what the modules of this mod share. */\n#ifndef LM_H\n#define LM_H\n#include \"lumart.h\"\n\n");
        let (svc_h, svc_c, svc_rows) = self.service_files();
        h.push_str(&svc_h);
        // The mod's C (docs/modding/language/language.md, section 13): its headers, then the script's declarations of its functions, which must agree.
        for header in &self.c_headers {
            let _ = writeln!(h, "#include \"{header}\"");
        }
        for p in &self.c_protos {
            let _ = writeln!(h, "{p}");
        }
        if !self.exports.is_empty() {
            h.push_str("#include \"lm_exports.h\"\n");
        }
        if !self.c_headers.is_empty() || !self.c_protos.is_empty() || !self.exports.is_empty() {
            h.push('\n');
        }
        for t in &self.typedefs {
            let _ = writeln!(h, "{t}\n");
        }
        for ((name, sig), slot) in &self.fn_slots {
            let _ = writeln!(h, "extern const LumaFunction* {slot}; /* {name} {sig} */");
        }
        for ((name, sig), slot) in &self.method_slots {
            let _ = writeln!(h, "extern const LumaFunction* {slot}; /* virtual {name} {sig} */");
        }
        for ((name, sig), slot) in &self.mod_fns {
            let _ = writeln!(h, "extern const void* {slot}; /* {name} {sig} */");
        }
        for ((name, ty), slot) in &self.fields {
            let _ = writeln!(h, "extern int32_t {slot}; /* {name}: {ty} */");
        }
        for ((name, ty), slot) in &self.game_globals {
            let _ = writeln!(h, "extern void* {slot}; /* {name}: {ty} */");
        }
        for (name, slot) in &self.classes {
            let _ = writeln!(h, "extern int32_t {slot}; /* class {name} */");
        }
        for (name, slot) in &self.sizes {
            let _ = writeln!(h, "extern int32_t {slot}; /* size of {name} */");
        }
        for ((name, sig), flag) in &self.inside {
            let _ = writeln!(h, "extern uint8_t {flag}; /* this mod is inside its own hook on {name} {sig} */");
        }
        for g in &self.globals {
            let _ = writeln!(h, "{g}");
        }
        for b in &self.protos {
            let _ = writeln!(h, "{b}");
        }
        // The hooks the kernel calls, one per (function, kind): defined in mod.c, named in imports.c.
        for gi in 0..self.groups.len() {
            let _ = writeln!(h, "int32_t lm_hook_group_{gi}(LumaCall* call, void* user);");
        }
        h.push_str("\n#endif\n");
        files.push(("lum/lm.h".to_string(), h));
        if !self.exports.is_empty() {
            let mut x = String::new();
            let _ = writeln!(x, "/* Generated by LumaLink: the mod {mod_id}'s `export c fn`s, for its own C (hidden: never the game's). */\n#ifndef LM_EXPORTS_H\n#define LM_EXPORTS_H\n#include <stdbool.h>\n#include <stdint.h>\n#ifdef __cplusplus\nextern \"C\" {{\n#endif");
            for p in &self.exports {
                let _ = writeln!(x, "{p}");
            }
            x.push_str("#ifdef __cplusplus\n}\n#endif\n#endif\n");
            files.push(("lum/lm_exports.h".to_string(), x));
        }
        for (m, code) in self.module_code.iter().enumerate() {
            // The standard module's file has a name no `source/` path gives as a C name of LumaLink's.
            let path = &self.sources[m].path;
            let name = if path == crate::check::STD_PATH { "lm_std" } else { path.as_str() };
            files.push((format!("lum/{name}.c"), code.clone()));
        }
        // mod.c: slots, flags, the hook wrappers, start, stop, luma_mod.
        let mut c = String::new();
        let _ = writeln!(c, "/* Generated by LumaLink: the mod {mod_id}'s hooks, start and stop. */\n#include \"lm.h\"\n");
        for slot in self.fn_slots.values().chain(self.method_slots.values()) {
            let _ = writeln!(c, "const LumaFunction* {slot};");
        }
        for slot in self.mod_fns.values() {
            let _ = writeln!(c, "const void* {slot};");
        }
        c.push_str(&svc_c);
        for slot in self.fields.values() {
            let _ = writeln!(c, "int32_t {slot};");
        }
        for slot in self.game_globals.values() {
            let _ = writeln!(c, "void* {slot};");
        }
        for slot in self.classes.values() {
            let _ = writeln!(c, "int32_t {slot};");
        }
        for slot in self.sizes.values() {
            let _ = writeln!(c, "int32_t {slot};");
        }
        for flag in self.inside.values() {
            let _ = writeln!(c, "uint8_t {flag};");
        }
        let mut rows: Vec<(String, String, u32, String)> = Vec::new();
        for (gi, ((name, sig, kind), group)) in self.groups.iter().enumerate() {
            let flag = &self.inside[&(name.clone(), sig.clone())];
            let wrapper = format!("lm_hook_group_{gi}");
            c.push_str(&group_wrapper(&wrapper, name, *kind, group, flag));
            rows.push((name.clone(), sig.clone(), *kind, wrapper));
        }
        // The rows sorted (name, signature, kind): docs/modding/language/compiled-form.md, section 3, the same list for the same script.
        let mut table: Vec<(String, String, u32, String)> = Vec::new();
        for ((name, sig), slot) in &self.fn_slots {
            table.push((name.clone(), sig.clone(), 100, format!("LUMA_FUNCTION({slot}, {}, {})", c_string(name), c_string(sig))));
            imports.push((name.clone(), sig.clone(), "function"));
        }
        for ((name, sig), slot) in &self.method_slots {
            table.push((name.clone(), sig.clone(), 108, format!("LUMA_METHOD({slot}, {}, {})", c_string(name), c_string(sig))));
            imports.push((name.clone(), sig.clone(), "method"));
        }
        for ((name, sig), slot) in &self.mod_fns {
            table.push((name.clone(), sig.clone(), 106, format!("LUMA_MOD_FUNCTION({slot}, {}, {})", c_string(name), c_string(sig))));
            imports.push((name.clone(), sig.clone(), "mod function"));
        }
        for (name, version, kind, row) in svc_rows {
            imports.push((name.clone(), version.clone(), "service"));
            table.push((name, version, kind, row));
        }
        for ((name, ty), slot) in &self.fields {
            table.push((name.clone(), ty.clone(), 101, format!("LUMA_FIELD({slot}, {}, {})", c_string(name), c_string(ty))));
            imports.push((name.clone(), ty.clone(), "field"));
        }
        for ((name, ty), slot) in &self.game_globals {
            table.push((name.clone(), ty.clone(), 107, format!("LUMA_GLOBAL({slot}, {}, {})", c_string(name), c_string(ty))));
            imports.push((name.clone(), ty.clone(), "global"));
        }
        for (name, slot) in &self.classes {
            table.push((name.clone(), String::new(), 103, format!("LUMA_CLASS({slot}, {})", c_string(name))));
            imports.push((name.clone(), String::new(), "class"));
        }
        for (name, slot) in &self.sizes {
            table.push((name.clone(), String::new(), 102, format!("LUMA_SIZE({slot}, {})", c_string(name))));
            imports.push((name.clone(), String::new(), "size"));
        }
        for (name, sig, kind, wrapper) in rows {
            let k = ["LUMA_BEFORE", "LUMA_AFTER", "LUMA_WRAP", "LUMA_REPLACE"][kind as usize];
            table.push((name.clone(), sig.clone(), kind, format!("LUMA_HOOK_ON({}, {}, {k}, {wrapper})", c_string(&name), c_string(&sig))));
            imports.push((name, sig, ["before", "after", "wrap", "replace"][kind as usize]));
        }
        table.sort_by(|a, b| (&a.0, &a.1, a.2).cmp(&(&b.0, &b.1, b.2)));
        imports.sort();
        // imports.c (docs/modding/language/compiled-form.md, section 3): the list alone, hidden like every symbol but luma_mod.
        let mut list = String::new();
        let _ = writeln!(list, "/* Generated by LumaLink: the mod {mod_id}'s import list, sorted (name, signature, kind). */\n#include \"lm.h\"\n");
        if !table.is_empty() {
            let _ = writeln!(list, "const LumaImport lm_imports[{}] = {{", table.len());
            for (.., row) in &table {
                let _ = writeln!(list, "    {row},");
            }
            list.push_str("};\n");
            let _ = writeln!(c, "extern const LumaImport lm_imports[{}];", table.len());
        }
        // `on frame` (docs/modding/services/time.md, section 9): one handler for the mod, each body in order; a script error ends that body's
        // run for the frame (reported by lumart), the next body still runs.
        let framed = !self.frames.is_empty() || self.tasks_used || !self.stages.is_empty();
        let timed = framed || self.step.is_some();
        if timed {
            c.push_str("#include <luma_time.h>\n\nstatic const LumaTimeService* lm_time;\n");
        }
        // `on step`: the mod's one body, once per world step; a script error ends that step's run.
        if let Some(s) = &self.step {
            let _ = write!(c, "\nstatic void lm_step(void* user, const LumaTimeNow* now) {{\n    (void)user;\n    (void){s}(now);\n}}\n");
        }
        if !self.stages.is_empty() {
            // Whether a frame since the last stage's start said BOOT or LOADING: the next PLAY frame is a stage's first.
            c.push_str("\nstatic uint8_t lm_stage_loaded = 1;\n");
            // The stage's name against a handler's (a mod links no C library).
            c.push_str("\nstatic int lm_stage_is(const char* a, const char* b) {\n    while (*a != 0 && *a == *b) {\n        ++a;\n        ++b;\n    }\n    return *a == *b;\n}\n");
        }
        if framed {
            c.push_str("\nstatic void lm_frame(void* user, const LumaTimeNow* now) {\n    (void)user;\n    (void)now;\n");
            // The tasks after `on frame`, against this frame's time read once; the ones it (or an
            // `on stage`) starts run from the next frame, so the pass counts them before it.
            if self.tasks_used {
                c.push_str("    const uint32_t lm_n = lumart_tasks_pass(now->frame, now->step);\n");
            }
            if !self.stages.is_empty() {
                let slot = self.fn_slots[&("MR::getCurrentStageName".to_string(), "const char *()".to_string())].clone();
                let starter = self.fn_slots[&("MR::isExecScenarioStarter".to_string(), "bool()".to_string())].clone();
                let opening = self.fn_slots[&("MR::isStageStateScenarioOpeningCamera".to_string(), "bool()".to_string())].clone();
                // A stage's first PLAY frame after BOOT or LOADING on which its opening is over.
                c.push_str("    if (now->state == LUMA_TIME_BOOT || now->state == LUMA_TIME_LOADING) {\n        lm_stage_loaded = 1;\n    } else if (now->state == LUMA_TIME_PLAY && lm_stage_loaded) {\n");
                let _ = writeln!(
                    c,
                    "        void* lm_args[1] = {{0}};\n        uint8_t lm_starting = 0;\n        uint8_t lm_opening = 0;\n        {starter}->invoke(lm_args, (void*)&lm_starting);\n        {opening}->invoke(lm_args, (void*)&lm_opening);\n        if (lm_starting == 0 && lm_opening == 0) {{\n            lm_stage_loaded = 0;\n            const char* lm_stage = 0;\n            {slot}->invoke(lm_args, (void*)&lm_stage);"
                );
                for (h, stage) in &self.stages {
                    let _ = writeln!(c, "            if (lm_stage != 0 && lm_stage_is(lm_stage, {})) (void){h}();", c_string(stage));
                }
                c.push_str("        }\n    }\n");
            }
            for f in &self.frames {
                let _ = writeln!(c, "    (void){f}();");
            }
            if self.tasks_used {
                c.push_str("    lumart_tasks_run(lm_n);\n");
            }
            c.push_str("}\n");
        }
        // Options: each a global, read at start and again on its OPTION event through mods 1.0 `option` (text:
        // a number's shortest decimal, a toggle's "1" or "0", a choice's or text's own); a key option is its input
        // action's handle (input 1.0 `action`).
        let mods_slot = self.svc_slots.get("mods").map(|s| s.slot.clone()).unwrap_or_default();
        let input_slot = self.svc_slots.get("input").map(|s| s.slot.clone()).unwrap_or_default();
        let watched = !self.options.is_empty() || !self.option_ons.is_empty();
        if self.options.iter().any(|o| !matches!(o.3, Ty::Service { .. })) {
            let _ = write!(
                c,
                "\nstatic int32_t lm_option_text(const char* key, lumart_text* out) {{\n    char lm_b[128];\n    char* lm_p = lm_b;\n    const int64_t lm_n = {mods_slot}->option(luma_rt_self, key, 0, 0);\n    if (lm_n < 0) return 1;\n    const uint64_t lm_size = (uint64_t)lm_n + 1u;\n    if (lm_size > sizeof lm_b) {{\n        lm_p = (char*)lumart_alloc(lm_size);\n        if (lm_p == 0) return 1;\n    }}\n    int32_t lm_st = 1;\n    if ({mods_slot}->option(luma_rt_self, key, lm_p, lm_size) >= 0) lm_st = lumart_text_from_game(out, lm_p, \"luma.json\", 0u);\n    if (lm_p != lm_b) lumart_free(lm_p, lm_size);\n    return lm_st;\n}}\n"
            );
        }
        for (i, (key, g, ct, t)) in self.options.iter().enumerate() {
            let k = c_string(key);
            let _ = write!(c, "\nstatic void lm_opt_read_{i}(void) {{\n");
            if matches!(t, Ty::Service { .. }) {
                let _ = writeln!(c, "    (void){input_slot}->action(luma_rt_self, {k}, (LumaInputAction*)(void*)&{g});");
            } else {
                let set = match t {
                    Ty::Text => format!("lm_cp_{ct}(&{g}, lm_t);"),
                    Ty::Bool => format!("{g} = ({ct})(lm_t != 0 && lm_t->len == 1u && lm_t->b[0] == '1');"),
                    Ty::Int(_) => format!("int64_t lm_v;\n        if (lm_t != 0 && lumart_text_parse_i64(lm_t, &lm_v)) {g} = ({ct})lm_v;"),
                    _ => format!("double lm_v;\n        if (lm_t != 0 && lumart_text_parse_f64(lm_t, &lm_v)) {g} = ({ct})lm_v;"),
                };
                let _ = write!(c, "    lumart_text lm_t = 0;\n    if (lm_option_text({k}, &lm_t) == 0) {{\n        {set}\n    }}\n    lm_rel_lumart_text(&lm_t);\n");
            }
            c.push_str("}\n");
        }
        if watched {
            // One OPTION event an option changed, its value its place in luma.json: read again, then its handlers.
            let mut by: BTreeMap<u32, (Vec<usize>, Vec<&str>)> = BTreeMap::new();
            for (i, o) in self.options.iter().enumerate() {
                if let Some(&n) = self.checked.option_index.get(&o.0) {
                    by.entry(n).or_default().0.push(i);
                }
            }
            for (key, h) in &self.option_ons {
                if let Some(&n) = self.checked.option_index.get(key) {
                    by.entry(n).or_default().1.push(h);
                }
            }
            c.push_str("\nstatic void lm_options(void* user, const LumaEvent* event) {\n    (void)user;\n    if (event->kind != LUMA_MODS_EVENT_OPTION) return;\n    switch (event->value) {\n");
            for (n, (reads, hs)) in &by {
                let _ = writeln!(c, "    case {n}u:");
                for i in reads {
                    let _ = writeln!(c, "        lm_opt_read_{i}();");
                }
                for h in hs {
                    let _ = writeln!(c, "        (void){h}();");
                }
                c.push_str("        break;\n");
            }
            c.push_str("    default:\n        break;\n    }\n}\n");
        }
        // `on draw` bodies by point, each point's in order from its one function.
        let draw_slot = self.svc_slots.get("draw").map(|s| s.slot.clone()).unwrap_or_default();
        let mut points: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for (n, h) in &self.draws {
            points.entry(*n).or_default().push(h.clone());
        }
        for (n, hs) in &points {
            let _ = write!(c, "\nstatic void lm_draw_{n}(void* user, const LumaDrawScreen* screen) {{\n    (void)user;\n    (void)screen;\n");
            for h in hs {
                let _ = writeln!(c, "    (void){h}();");
            }
            c.push_str("}\n");
        }
        // What the mod's code reads made first, once a load (`lm_prepare`): its options, its classes (a module's
        // initialiser may make one of their objects), its modules' values. Start runs it; with `on layers` the kernel's
        // layers call, which comes before start, runs it first.
        let mut prepare = String::new();
        for i in 0..self.options.len() {
            let _ = writeln!(prepare, "    lm_opt_read_{i}();");
        }
        for d in &self.class_defines {
            let _ = writeln!(prepare, "    if ({d}() != 0) return 1;");
        }
        for i in &self.inits {
            let _ = writeln!(prepare, "    if ({i}() != 0) return 1;");
        }
        if let Some(l) = &self.layers {
            let _ = write!(c, "\nstatic uint8_t lm_prepared = 0;\n\nstatic int32_t lm_prepare(void) {{\n    if (lm_prepared) return 0;\n    lm_prepared = 1;\n{prepare}    return 0;\n}}\n");
            // The names, each with its NUL, into buf when they fit; the bytes they need. A script error: none chosen.
            let _ = write!(
                c,
                "\nstatic int64_t lm_layers(const LumaCore* core, const LumaMod* mod, char* buf, uint64_t size) {{\n    (void)core;\n    (void)mod;\n    if (lm_prepare() != 0) return 0;\n    lumart_list lm_l = 0;\n    if ({l}(&lm_l) != 0) {{\n        lm_rel_lumart_list(&lm_l);\n        return 0;\n    }}\n    uint64_t lm_need = 0;\n    for (uint64_t i = 0; i < lumart_list_n(lm_l); i++) {{\n        const lumart_text t = *(const lumart_text*)lumart_list_at(lm_l, i);\n        lm_need += (t != 0 ? t->len : 0u) + 1u;\n    }}\n    if (buf != 0 && lm_need <= size) {{\n        uint64_t at = 0;\n        for (uint64_t i = 0; i < lumart_list_n(lm_l); i++) {{\n            const lumart_text t = *(const lumart_text*)lumart_list_at(lm_l, i);\n            const uint64_t n = t != 0 ? t->len : 0u;\n            for (uint64_t k = 0; k < n; k++) buf[at + k] = t->b[k];\n            buf[at + n] = 0;\n            at += n + 1u;\n        }}\n    }}\n    lm_rel_lumart_list(&lm_l);\n    return (int64_t)lm_need;\n}}\n"
            );
        }
        c.push_str("\nstatic int32_t lm_start(const LumaCore* core, const LumaMod* mod) {\n    (void)core;\n    (void)mod;\n");
        if timed {
            // No time service (or not granted): the mod cannot run its frames or steps, so it does not start (said by
            // the core).
            c.push_str("    lm_time = (const LumaTimeService*)core->service(mod, LUMA_TIME_NAME, LUMA_TIME_MAJOR, LUMA_TIME_MINOR);\n    if (lm_time == 0) return 1;\n");
        }
        if framed {
            c.push_str("    if (lm_time->on_frame(mod, lm_frame, 0) != 0) return 1;\n");
        }
        if self.step.is_some() {
            c.push_str("    if (lm_time->on_step(mod, lm_step, 0) != 0) return 1;\n");
        }
        if self.tasks_used {
            // The waits of `on start` and of tasks it starts count from the frame and step of now.
            c.push_str("    {\n        LumaTimeNow now;\n        __builtin_memset(&now, 0, sizeof now);\n        now.size = sizeof now;\n        if (lm_time->now(mod, &now) == 0) {\n            lumart_now_frame = now.frame;\n            lumart_now_step = now.step;\n        }\n    }\n");
        }
        if self.layers.is_some() {
            c.push_str("    if (lm_prepare() != 0) return 1;\n");
        } else {
            c.push_str(&prepare);
        }
        if watched {
            let _ = writeln!(c, "    if ({mods_slot}->watch_options(mod, lm_options, 0) != 0) return 1;");
        }
        if let Some(l) = &self.load {
            let _ = writeln!(c, "    if ({l}() != 0) return 1;");
        }
        // An optional draw (`use optional svc.draw`) not given: no point is drawn at.
        let draw_given = if self.optional_services().contains("draw") { format!("{draw_slot} != 0 && ") } else { String::new() };
        for n in points.keys() {
            let _ = writeln!(c, "    if ({draw_given}{draw_slot}->at(mod, (LumaDrawPoint){n}, lm_draw_{n}, 0) != 0) return 1;");
        }
        for s in &self.starts {
            let _ = writeln!(c, "    if ({s}() != 0) return 1;");
        }
        c.push_str("    return 0;\n}\n\nstatic void lm_stop(void) {\n");
        if framed {
            c.push_str("    if (lm_time != 0) (void)lm_time->on_frame(luma_rt_self, 0, 0);\n");
        }
        if self.step.is_some() {
            c.push_str("    if (lm_time != 0) (void)lm_time->on_step(luma_rt_self, 0, 0);\n");
        }
        if watched {
            let _ = writeln!(c, "    (void){mods_slot}->watch_options(luma_rt_self, 0, 0);");
        }
        for n in points.keys() {
            let only = if draw_given.is_empty() { String::new() } else { format!("if ({draw_slot} != 0) ") };
            let _ = writeln!(c, "    {only}(void){draw_slot}->at(luma_rt_self, (LumaDrawPoint){n}, 0, 0);");
        }
        if self.tasks_used {
            // Every task stopped: its defers run and its counts given back, before `on stop`.
            c.push_str("    lumart_tasks_stop_all();\n");
        }
        // `on stop` per module, the last module's first.
        for s in self.stops.iter().rev() {
            let _ = writeln!(c, "    (void){s}();");
        }
        for (g, ct) in &self.counted_globals {
            let _ = writeln!(c, "    lm_rel_{ct}(&{g});");
        }
        if self.layers.is_some() {
            // A start after this stop, in the same load, makes the mod's values again.
            c.push_str("    lm_prepared = 0;\n");
        }
        c.push_str("    lumart_scratch_free_all();\n}\n\n");
        // A mod that called this one's exports is gone; each `on mods.stopped` gets its number.
        // Only a mod with exports is needed by others, so only its table names the function.
        let gone_used = !self.gones.is_empty() && !self.lib_exports.is_empty();
        if gone_used {
            c.push_str("static void lm_dependent_gone(const LumaMod* dependent) {\n");
            for g in &self.gones {
                let _ = writeln!(c, "    (void){g}((uint64_t)(uintptr_t)dependent);");
            }
            c.push_str("}\n\n");
        }
        let gone = if gone_used { "lm_dependent_gone" } else { "0" };
        // The mod's finish: luma.h's (its exits and destructors), then the runtime gives back what it took
        // (lumart_end), after the game's grace, so no hook of the mod's is running.
        const LM_FINISH: &str = "static void lm_finish(void) {\n    luma_finish();\n    lumart_end();\n}\n";
        let mut exports: Vec<(String, String, String)> = self.lib_exports.clone();
        exports.sort();
        // By its name: a luma.h without the field (before the kernel's layers call) is an error, not a dropped value.
        let layers_c = ".layers = lm_layers";
        // The runtime layout in the mod's record when the SDK's LumaModInfo has the field (Game::mod_info_layout).
        let layout_c = if self.game.mod_info_layout { format!(", .runtime_layout = {}", crate::lumi::RUNTIME_LAYOUT) } else { String::new() };
        if self.game.mod_info_layout && exports.is_empty() && self.layers.is_none() {
            let (list_c, count) = if table.is_empty() { ("0".to_string(), "0".to_string()) } else { ("lm_imports".to_string(), table.len().to_string()) };
            let _ = writeln!(
                c,
                "LUMA_RUNTIME\n{LM_FINISH}LUMA_C LUMA_EXPORT const LumaModInfo luma_mod = {{sizeof(LumaModInfo), LUMA_API, {list_c}, {count}, lm_start, lm_stop, luma_begin, lm_finish, 0, 0, {gone}{layout_c}}};"
            );
        } else if exports.is_empty() && self.layers.is_some() {
            let (list_c, count) = if table.is_empty() { ("0".to_string(), "0".to_string()) } else { ("lm_imports".to_string(), table.len().to_string()) };
            let _ = writeln!(
                c,
                "LUMA_RUNTIME\n{LM_FINISH}LUMA_C LUMA_EXPORT const LumaModInfo luma_mod = {{sizeof(LumaModInfo), LUMA_API, {list_c}, {count}, lm_start, lm_stop, luma_begin, lm_finish, 0, 0, {gone}, {layers_c}{layout_c}}};"
            );
        } else if !exports.is_empty() {
            // A library mod (docs/modding/lumalink/loader.md): its exports sorted by name, for the kernel's binary search.
            let _ = writeln!(c, "static const LumaExport lm_exports[{}] = {{", exports.len());
            for (name, sig, wrapper) in &exports {
                let _ = writeln!(c, "    LUMA_EXPORT_FUNCTION({}, {}, {wrapper}),", c_string(name), c_string(sig));
            }
            c.push_str("};\n\n");
            let (list_c, count) = if table.is_empty() { ("0".to_string(), "0".to_string()) } else { ("lm_imports".to_string(), table.len().to_string()) };
            let _ = writeln!(
                c,
                "LUMA_RUNTIME\n{LM_FINISH}LUMA_C LUMA_EXPORT const LumaModInfo luma_mod = {{sizeof(LumaModInfo), LUMA_API, {list_c}, {count}, lm_start, lm_stop, luma_begin, lm_finish, lm_exports, {}, {gone}{}{layout_c}}};",
                exports.len(),
                if self.layers.is_some() { format!(", {layers_c}") } else { String::new() }
            );
            files.push(("lum/exports.c".to_string(), format!("/* Generated by LumaLink: the mod {mod_id}'s functions for other mods. */\n#include \"lm.h\"\n\n{}", self.export_code)));
        } else {
            // As LUMA_MOD and LUMA_MOD_NO_IMPORTS write it, with the mod's own finish; the fields after are zero.
            let (list_c, count) = if table.is_empty() { ("0", "0") } else { ("lm_imports", "sizeof(lm_imports) / sizeof(lm_imports[0])") };
            let _ = writeln!(
                c,
                "LUMA_RUNTIME\n{LM_FINISH}LUMA_C LUMA_EXPORT const LumaModInfo luma_mod = {{sizeof(LumaModInfo), LUMA_API, {list_c}, {count}, lm_start, lm_stop, luma_begin, lm_finish}};"
            );
        }
        files.push(("lum/imports.c".to_string(), list));
        files.push(("lum/mod.c".to_string(), c));
        files.push(("lum/lumart.h".to_string(), LUMART_H.to_string()));
        files.push(("lum/lumart.c".to_string(), LUMART_C.to_string()));
        let mut kept = self.kept;
        kept.dedup();
        let exports = exports.into_iter().map(|(n, s, _)| (n, s)).collect();
        Emitted { files, imports, kept, lumi: None, exports }
    }
}

/// The `LumaHook` the kernel calls for one (function, kind): the game-thread test, the re-entry flag, and each body in
/// the order written (docs/modding/language/compiled-form.md, section 4).
fn group_wrapper(wrapper: &str, function: &str, kind: u32, group: &Group, flag: &str) -> String {
    let mut c = String::new();
    let first = &group.bodies[0];
    let thread = format!("if (!lumart_on_game_thread({}, {}, {})) return 0;", c_string(&first.file), first.line, c_string(function));
    match kind {
        0 => {
            let _ = writeln!(c, "\nint32_t {wrapper}(LumaCall* call, void* user) {{\n    (void)user;\n    {thread}\n    if ({flag}) return 0;\n    {flag} = 1;\n    uint8_t skip = 0;");
            for b in &group.bodies {
                // A body that ends in a script error does not skip (section 8); the next body still runs.
                let _ = writeln!(c, "    if ({}(call, &skip) == 0 && skip) {{\n        {flag} = 0;\n        return 1;\n    }}\n    skip = 0;", b.c_name);
            }
            let _ = writeln!(c, "    {flag} = 0;\n    return 0;\n}}");
        }
        1 => {
            let _ = writeln!(c, "\nint32_t {wrapper}(LumaCall* call, void* user) {{\n    (void)user;\n    {thread}\n    if ({flag}) return 0;\n    {flag} = 1;");
            for b in &group.bodies {
                match &group.result_c {
                    // An after that ends in a script error leaves the result as it was before it (section 8).
                    Some(rc) => {
                        let _ = writeln!(c, "    {{\n        {rc} was = LUMA_RESULT(call, {rc});\n        if ({}(call) != 0) LUMA_RESULT(call, {rc}) = was;\n    }}", b.c_name);
                    }
                    None => {
                        let _ = writeln!(c, "    (void){}(call);", b.c_name);
                    }
                }
            }
            let _ = writeln!(c, "    {flag} = 0;\n    return 0;\n}}");
        }
        _ => {
            // Wraps of one mod on one function are one wrap: body k's proceed() runs body k+1, the last one's the
            // kernel's proceed with the flag down, so code the game runs inside re-enters every mod's hooks.
            let n = group.bodies.len();
            let _ = writeln!(c, "\nstatic int32_t {wrapper}_proceed_{n}(LumaCall* call) {{\n    {flag} = 0;\n    luma_rt_core->proceed(call);\n    {flag} = 1;\n    return 0;\n}}");
            for (k, b) in group.bodies.iter().enumerate().rev() {
                let next = format!("{wrapper}_proceed_{}", k + 1);
                let replace = kind == 3;
                let _ = writeln!(c, "\nstatic int32_t {wrapper}_proceed_{k}(LumaCall* call) {{\n    uint8_t proceeded = 0;\n    if ({}(call, &proceeded, {next}) != 0 && !proceeded) {{", b.c_name);
                // A wrap that ends in a script error before proceeding proceeds; a replace gives the game's result
                // by the kernel's proceed.
                if replace {
                    let _ = writeln!(c, "        {flag} = 0;\n        luma_rt_core->proceed(call);\n        {flag} = 1;");
                } else {
                    let _ = writeln!(c, "        (void){next}(call);");
                }
                let _ = writeln!(c, "    }}\n    return 0;\n}}");
            }
            let _ = writeln!(c, "\nint32_t {wrapper}(LumaCall* call, void* user) {{\n    (void)user;\n    if (!lumart_on_game_thread({}, {}, {})) {{\n        luma_rt_core->proceed(call);\n        return 0;\n    }}\n    if ({flag}) {{\n        luma_rt_core->proceed(call);\n        return 0;\n    }}\n    {flag} = 1;\n    (void){wrapper}_proceed_0(call);\n    {flag} = 0;\n    return 0;\n}}", c_string(&first.file), first.line, c_string(function));
        }
    }
    c
}

impl Fx {
    fn new(file: &str, module: usize) -> Fx {
        Fx { inline_ret: Vec::new(), rechecks: Vec::new(), writing: false, live: Vec::new(), file: file.to_string(), module, decls: Vec::new(), lines: Vec::new(), builders: Vec::new(), tmp: 0, depth: 1, places: HashMap::new(), declared: HashSet::new(), hook: None, ret: None, self_c: None, defer_scopes: Vec::new(), all_defers: Vec::new(), loops: Vec::new(), tail: Vec::new(), task: false, task_ret: None, site: false, scratch: false, waits: 0, class_data: None, override_rc: None, override_params: Vec::new(), machine: None, free: HashMap::new(), made: Vec::new() }
    }

    /// A builder in the scratch (docs/modding/language/language.md, section 6.4, docs/modding/language/compiled-form.md, section 7.2) and the mark taken before it: the caller gives back with
    /// `lumart_scratch_release(mark)` once the text is used (copied, logged, or after the game call it went to).
    fn scratch_builder(&mut self) -> (String, String) {
        self.scratch = true;
        let mark = self.take("uint64_t");
        self.emit(format!("{mark} = lumart_scratch_mark();"));
        let b = self.take("lumart_scratch_sb");
        self.emit(format!("lumart_sb_scratch(&{b});"));
        (b, mark)
    }

    fn emit(&mut self, line: String) {
        let indent = "    ".repeat(self.depth);
        self.lines.push(format!("{indent}{line}"));
    }

    fn decl(&mut self, d: String) {
        self.decls.push(format!("    {d}"));
    }

    fn line_marker(&mut self, at: Pos) {
        self.lines.push(format!("#line {} {}", at.line, c_string(&self.file)));
    }

    /// A new temporary of C type `ct`, set to `value` (a counted value's count taken: the temporary owns one).
    fn temp(&mut self, ct: &str, value: &str) -> String {
        let t = self.take(ct);
        self.set(&t, value, ct);
        t
    }

    /// `place = value` (see [`assign_line`]).
    fn set(&mut self, place: &str, value: &str, ct: &str) {
        let line = assign_line(place, value, ct);
        self.emit(line);
    }

    /// A new temporary of C type `ct`, zero where it is made (each time the code runs: in a loop too). A counted one is
    /// zero already: its statement's end gave its count back ([`Fx::release_since`] zeroes the slot).
    fn fresh(&mut self, ct: &str) -> String {
        let t = self.take(ct);
        if !counted(ct) {
            self.emit(format!("{t} = ({ct}){{0}};"));
        }
        t
    }

    /// A temporary of C type `ct` for this statement: one whose statement has ended, or a new one declared at the top.
    fn take(&mut self, ct: &str) -> String {
        let t = match self.free.get_mut(ct).and_then(Vec::pop) {
            Some(t) => t,
            None => {
                let t = format!("t{}", self.tmp);
                self.tmp += 1;
                self.decl(format!("{ct} {t};"));
                t
            }
        };
        self.made.push((t.clone(), ct.to_string()));
        if counted(ct) {
            self.live.push((t.clone(), ct.to_string()));
        }
        t
    }

    /// A temporary that lives to the function's end (a defer's flag), zero from the function's top: never used by
    /// another statement.
    fn fresh_kept(&mut self, ct: &str) -> String {
        let t = format!("t{}", self.tmp);
        self.tmp += 1;
        self.decl(format!("{ct} {t};"));
        t
    }

    /// An array temporary `ct name[n]` for this statement (a game call's argument list).
    fn array(&mut self, ct: &str, n: usize) -> String {
        let key = format!("{ct}[{n}]");
        let t = match self.free.get_mut(&key).and_then(Vec::pop) {
            Some(t) => t,
            None => {
                let t = format!("t{}", self.tmp);
                self.tmp += 1;
                self.decl(format!("{ct} {t}[{n}];"));
                t
            }
        };
        self.made.push((t.clone(), key));
        t
    }

    /// The temporaries and builders made since `mark` (a statement's start) free for the statements after it.
    fn recycle_since(&mut self, mark: usize) {
        for (t, key) in self.made.drain(mark..) {
            self.free.entry(key).or_default().push(t);
        }
    }

    /// The container checks recorded since `mark` made again here, and dropped from the record.
    fn recheck_since(&mut self, mark: usize) {
        let again: Vec<String> = self.rechecks.drain(mark..).flatten().collect();
        self.lines.extend(again);
    }

    /// Gives back the counts of the counted temporaries made since `mark` (a statement's end): a temporary held past
    /// its statement would make the next write to a container copy it (copy on write sees two owners).
    fn release_since(&mut self, mark: usize) {
        let gone: Vec<(String, String)> = self.live.drain(mark..).collect();
        for (t, ct) in gone {
            self.emit(format!("lm_rel_{ct}(&{t});"));
        }
    }

    /// An integer as decimal text, for a report's argument (a buffer of the function's own).
    fn temp_text_int(&mut self, v: &str, s: Scalar) -> String {
        let buf = self.builder();
        if signed(s) {
            self.emit(format!("lumart_sb_i64(&{buf}, (int64_t){v});"));
        } else {
            self.emit(format!("lumart_sb_u64(&{buf}, (uint64_t){v});"));
        }
        format!("({buf}.p != 0 ? {buf}.p : \"?\")")
    }

    fn temp_text_float(&mut self, v: &str) -> String {
        let buf = self.builder();
        self.emit(format!("lumart_sb_f64(&{buf}, (double){v});"));
        format!("({buf}.p != 0 ? {buf}.p : \"?\")")
    }

    /// A text builder, emptied where it is made, freed on the way out.
    fn builder(&mut self) -> String {
        let b = match self.free.get_mut("lumart_sb").and_then(Vec::pop) {
            Some(b) => b,
            None => {
                let b = format!("t{}", self.tmp);
                self.tmp += 1;
                self.builders.push(b.clone());
                b
            }
        };
        self.made.push((b.clone(), "lumart_sb".to_string()));
        self.emit(format!("lumart_sb_free(&{b});"));
        b
    }

    /// The values of a hook's or override's pointer parameters ([`HookFx::outs`]) written through their pointers (a
    /// null pointer, a none or a const pointer writes nothing).
    fn place_args_back(&mut self) {
        let Some(hk) = self.hook.clone() else { return };
        for (v, p, et, konst) in &hk.outs {
            if !konst {
                self.emit(format!("if ({p} != 0 && {v}.some) *({et}*){p} = {v}.value;"));
            }
        }
    }

    /// After the game's code ran on (proceed, base): each pointer parameter's value read again.
    fn place_args_in(&mut self) {
        let Some(hk) = self.hook.clone() else { return };
        for (v, p, et, _) in &hk.outs {
            self.emit(format!("if ({p} != 0) {{ {v}.some = 1; {v}.value = *(const {et}*){p}; }}"));
        }
    }

    /// The pointer parameters' values go back on every way out but a script error's.
    fn place_args_tail(&mut self) {
        if !self.hook.as_ref().is_some_and(|hk| hk.outs.iter().any(|o| !o.3)) {
            return;
        }
        let lines = std::mem::take(&mut self.lines);
        let depth = std::mem::replace(&mut self.depth, 1);
        self.emit("if (lm_st == 0) {".into());
        self.depth += 1;
        self.place_args_back();
        self.depth -= 1;
        self.emit("}".into());
        let back = std::mem::replace(&mut self.lines, lines);
        self.tail.extend(back);
        self.depth = depth;
    }

    /// When `cond` holds (and has reported), the function ends with a script error.
    fn fail_if(&mut self, cond: &str) {
        self.emit(format!("if ({cond}) {{ lm_st = 1; goto lm_out; }}"));
    }

    /// A hook's `const char *` parameters the body changed, written to the call: each text's kept copy (one per
    /// distinct text, docs/modding/language/compiled-form.md: the game may keep the name), null for none; a NUL inside is S0918. In the way
    /// out (`tail`) a failure only sets the status.
    fn text_args_back(&mut self, tail: bool) {
        let Some(hk) = self.hook.clone() else { return };
        for (v, was, n, p, ct) in &hk.texts {
            self.emit(format!("if (!({v}.some == {was}.some && (!{v}.some || lumart_text_eq({v}.value, {was}.value)))) {{"));
            self.depth += 1;
            self.emit("const char* lm_k = 0;".into());
            let keep = format!("lumart_text_keep({v}.value, {}, {}u, {}, {})", c_string(&self.file), hk.line, c_string(&hk.name), c_string(p));
            self.emit(format!("if ({v}.some) lm_k = {keep};"));
            if tail {
                self.emit(format!("if ({v}.some && lm_k == 0) lm_st = 1; else *(const char**)call->args[{n}] = lm_k;"));
            } else {
                self.fail_if(&format!("{v}.some && lm_k == 0"));
                self.emit(format!("*(const char**)call->args[{n}] = lm_k;"));
            }
            self.emit(format!("lm_cp_{ct}(&{was}, {v});"));
            self.depth -= 1;
            self.emit("}".into());
        }
    }

    fn local(&self, id: LocalId) -> String {
        self.places.get(&id).cloned().unwrap_or_else(|| format!("l{id}"))
    }

    /// The function: its head, its locals zeroed at the top (docs/modding/language/compiled-form.md, section 6), its statements, its one way out.
    fn function(&self, head: &str, at: Pos) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "#line {} {}", at.line, c_string(&self.file));
        let _ = writeln!(out, "{head} {{\n    int32_t lm_st = 0;");
        for d in &self.decls {
            // Each local zeroed: `= {0}` gives every type, scalar or struct, its zero.
            let d = d.trim_end_matches(';');
            let _ = writeln!(out, "{d} = {{0}};");
        }
        // Builders are not in `decls` (a task's frame copies them in only once a wait has kept them).
        for b in &self.builders {
            let _ = writeln!(out, "    lumart_sb {b};\n    lumart_sb_init(&{b});");
        }
        if self.scratch {
            out.push_str("    const uint64_t lm_sm = lumart_scratch_mark();\n");
        }
        if self.hook.is_some() {
            out.push_str("    (void)call;\n");
        }
        if matches!(self.hook.as_ref().map(|h| h.kind), Some(HookKind::Wrap | HookKind::Replace)) {
            out.push_str("    (void)lm_proceeded;\n    (void)lm_next;\n");
        }
        if matches!(self.hook.as_ref().map(|h| h.kind), Some(HookKind::Before)) {
            out.push_str("    (void)lm_skip;\n");
        }
        for l in &self.lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str("    goto lm_out;\nlm_out:\n");
        out.push_str(&self.way_out());
        out
    }

    /// What follows `lm_out`: the defers still set, every count given back, the builders freed, the status returned.
    fn way_out(&self) -> String {
        let mut out = String::new();
        for l in &self.tail {
            out.push_str(l);
            out.push('\n');
        }
        // Every variable holding a counted value gives its count back (after the defers, which may read them).
        for d in &self.decls {
            let d = d.trim().trim_end_matches(';');
            if let Some((ct, name)) = d.rsplit_once(' ') {
                if counted(ct) {
                    let _ = writeln!(out, "    lm_rel_{ct}(&{name});");
                }
            }
        }
        for b in &self.builders {
            let _ = writeln!(out, "    lumart_sb_free(&{b});");
        }
        if self.scratch {
            out.push_str("    lumart_scratch_release(lm_sm);\n");
        }
        out.push_str("    return lm_st;\n}\n\n");
        out
    }
}

fn global_name(module: usize, name: &str) -> String {
    format!("lm_g{module}_{name}")
}

fn limit_c(s: Scalar, max: bool) -> String {
    match (s, max) {
        (Scalar::I8, false) => "((int8_t)-128)".into(),
        (Scalar::I8, true) => "((int8_t)127)".into(),
        (Scalar::I16, false) => "((int16_t)-32768)".into(),
        (Scalar::I16, true) => "((int16_t)32767)".into(),
        (Scalar::I32, false) => "((int32_t)(-2147483647 - 1))".into(),
        (Scalar::I32, true) => "((int32_t)2147483647)".into(),
        (Scalar::I64, false) => "((int64_t)(-9223372036854775807LL - 1))".into(),
        (Scalar::I64, true) => "((int64_t)9223372036854775807LL)".into(),
        (Scalar::U8, false) | (Scalar::U16, false) | (Scalar::U32, false) | (Scalar::U64, false) => format!("(({})0)", scalar_c(s)),
        (Scalar::U8, true) => "((uint8_t)255)".into(),
        (Scalar::U16, true) => "((uint16_t)65535)".into(),
        (Scalar::U32, true) => "((uint32_t)4294967295u)".into(),
        (Scalar::U64, true) => "((uint64_t)18446744073709551615ull)".into(),
        (_, _) => "0".into(),
    }
}

/// The C test that integer `v` of type `a` fits type `b`, or None when every value does.
fn int_fits(a: Scalar, b: Scalar, v: &str) -> Option<String> {
    let bits = |s: Scalar| match s {
        Scalar::I8 | Scalar::U8 => 8,
        Scalar::I16 | Scalar::U16 => 16,
        Scalar::I32 | Scalar::U32 => 32,
        _ => 64,
    };
    let (sa, sb) = (signed(a), signed(b));
    let (wa, wb) = (bits(a), bits(b));
    let fits_all = (sa == sb && wb >= wa) || (!sa && sb && wb > wa);
    if fits_all {
        return None;
    }
    // Compare in the widest form that holds both: signed values through int64_t, unsigned through uint64_t.
    let lo = limit_c(b, false);
    let hi = limit_c(b, true);
    Some(match (sa, sb) {
        (true, true) => format!("(int64_t){v} >= (int64_t){lo} && (int64_t){v} <= (int64_t){hi}"),
        (true, false) => format!("(int64_t){v} >= 0 && (uint64_t)(int64_t){v} <= (uint64_t){hi}"),
        (false, _) => format!("(uint64_t){v} <= (uint64_t){hi}"),
    })
}

/// The open bounds a float must lie within to convert to integer type `s` (cut toward zero).
fn float_bounds(s: Scalar) -> (String, String) {
    let (lo, hi): (f64, f64) = match s {
        Scalar::I8 => (-129.0, 128.0),
        Scalar::I16 => (-32769.0, 32768.0),
        Scalar::I32 => (-2147483649.0, 2147483648.0),
        Scalar::I64 => (-9223372036854777856.0, 9223372036854775808.0),
        Scalar::U8 => (-1.0, 256.0),
        Scalar::U16 => (-1.0, 65536.0),
        Scalar::U32 => (-1.0, 4294967296.0),
        _ => (-1.0, 18446744073709551616.0),
    };
    (f64_c(lo), f64_c(hi))
}

/// An f32 as a C literal that reads back exactly (Rust's shortest form, `f` suffixed).
fn f32_c(v: f32) -> String {
    if v.is_nan() || v.is_infinite() {
        return "0.0f".into();
    }
    let s = format!("{v:?}");
    let s = if s.contains(['.', 'e', 'E']) { s } else { format!("{s}.0") };
    format!("{s}f")
}

fn f64_c(v: f64) -> String {
    let s = format!("{v:?}");
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.0")
    }
}

fn on_name(e: &ast::OnEvent) -> &'static str {
    match e {
        ast::OnEvent::Start => "start",
        ast::OnEvent::Stop => "stop",
        ast::OnEvent::Frame => "frame",
        ast::OnEvent::Load => "load",
        ast::OnEvent::Step(_) => "step",
        ast::OnEvent::Option(_) => "option",
        ast::OnEvent::Stage(_) => "stage",
        ast::OnEvent::Draw(_) => "draw",
        ast::OnEvent::Layers(_) => "layers",
        ast::OnEvent::Event { .. } => "a service event",
    }
}
