//! The game's tables by name (docs/modding/language/numbers.md: `set_table`, `table`) and the downcast `as_class<T>`
//! (docs/modding/language/objects.md), against a small game: the C each writes, and each refusal by its code.

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::{code, Arg};
use lumascript::emit;
use lumascript::game::{Game, GameTable};
use std::collections::HashMap;

const GAME: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 8, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "LiveActor", "size": 64, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "class", "name": "Kuribo", "size": 96, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "LiveActor", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "class", "name": "Gravity", "size": 16, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "struct", "name": "Box", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "mW", "type": "s32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mH", "type": "f32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Consts", "size": 72, "align": 8, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "mJump", "type": "f32", "offset": 0, "count": 3, "extents": null, "access": "public", "bits": null},
      {"name": "mGrid", "type": "s16", "offset": 12, "count": 6, "extents": [2, 3], "access": "public", "bits": null},
      {"name": "mOn", "type": "bool", "offset": 24, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mMode", "type": "Mode", "offset": 28, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mFlags", "type": "u32 : 3", "offset": 32, "count": 1, "extents": null, "access": "public", "bits": {"width": 3, "offset": 256}},
      {"name": "mName", "type": "const char *", "offset": 40, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mBox", "type": "Box", "offset": 48, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mTint", "type": "Color8", "offset": 56, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mUp", "type": "Vec3f", "offset": 60, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Color8", "size": 4, "align": 1, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "r", "type": "u8", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "g", "type": "u8", "offset": 1, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "b", "type": "u8", "offset": 2, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "a", "type": "u8", "offset": 3, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Vec3<f32>", "size": 12, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "x", "type": "f32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "y", "type": "f32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "z", "type": "f32", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "alias", "name": "Vec3f", "of": "Vec3<f32>", "record": "Vec3<f32>"},
    {"kind": "alias", "name": "s16", "of": "short", "record": null},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "alias", "name": "u8", "of": "unsigned char", "record": null},
    {"kind": "alias", "name": "u32", "of": "unsigned int", "record": null},
    {"kind": "alias", "name": "f32", "of": "float", "record": null},
    {"kind": "enum", "name": "Mode", "underlying": "int", "scoped": true, "values": [{"name": "Walk", "value": 0}, {"name": "Run", "value": 2}]}
  ],
  "globals": [],
  "left_out": []
}"#;

const FUNCTIONS: &[(&str, &str)] = &[("MR::actor", "LiveActor *()"), ("MR::gravity", "Gravity *()")];

fn game() -> Game {
    let mut functions: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (name, sig) in FUNCTIONS {
        functions.entry(name.to_string()).or_default().push((sig.to_string(), false));
    }
    let mut game = Game::new(functions, Some(Types::from_json(GAME.as_bytes()).unwrap_or_else(|e| panic!("{e}"))));
    let table = |ty: &str, count: u64, size: u64| GameTable { ty: ty.to_string(), count, size };
    game.tables = Some(HashMap::from([
        ("T.cpp#cConsts".to_string(), table("Consts", 1, 72)),
        ("T.cpp#cBoxes".to_string(), table("Box", 4, 32)),
        ("T.cpp#cSpeeds".to_string(), table("float", 3, 12)),
        ("T.cpp#cNames".to_string(), table("const char *", 2, 16)),
        ("T.cpp#cModes".to_string(), table("Mode", 2, 8)),
        ("T.cpp#cUps".to_string(), table("Vec3f", 2, 24)),
    ]));
    game
}

/// What the checker says of a program: (code, line, its arguments as `name=value`).
fn said(text: &str) -> Vec<(u16, u32, Vec<String>)> {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        checked
            .diags
            .iter()
            .filter(|(_, d)| !lumascript::diag::is_note(d.code))
            .map(|(_, d)| {
                let args = d.args.iter().map(|(n, a)| match a {
                    Arg::Text(t) => format!("{n}={t}"),
                    Arg::Phrase(p) => format!("{n}={p}"),
                });
                (d.code, d.at.line, args.collect())
            })
            .collect()
    })
    .unwrap()
}

/// The generated C of the program's module; the checker says nothing. Compiled when LUMA_SDK names luma.h's folder.
fn build(text: &str) -> String {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("tables", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        compiles(&out.files);
        out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap()
    })
    .unwrap()
}

fn compiles(files: &[(String, String)]) {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("the generated C is not compiled: LUMA_SDK is not set");
        return;
    };
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-tables-{}-{}", std::process::id(), files.len() ^ files.iter().map(|f| f.1.len()).sum::<usize>()));
    let own = dir.join("source");
    std::fs::create_dir_all(&own).unwrap();
    for (rel, text) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    for (i, (rel, text)) in files.iter().enumerate().filter(|(_, (rel, _))| rel.ends_with(".c")) {
        let out = std::process::Command::new(&clang)
            .current_dir(&dir)
            .arg("--target=x86_64-pc-windows-msvc")
            .args(emit::C_FLAGS)
            .arg("-I").arg(dir.join("lum")).arg("-I").arg(&sdk).arg("-I").arg(&own)
            .arg("-c").arg(rel).arg("-o").arg(dir.join(format!("{i}.o")))
            .output()
            .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
        assert!(out.status.success(), "{rel} does not compile:\n{}\n{text}", String::from_utf8_lossy(&out.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// set_table writes the place its steps name, at the offset the types file gives: a record table's array field by a
/// literal index (mJump[2]: 8), a two-extent field (mGrid[1][2]: 12 + 3*2*1 + 2*2 = 22), a bool, an enum, a colour
/// (four u8), a vector, an array table's record field, and an index computed at run time (checked, S0902). Each
/// value takes the place's type; the bytes are as the game holds them.
#[test]
fn set_table_writes_each_place_by_its_offset() {
    let c = build(
        "on start {
    let i = 3
    let a = set_table(\"T.cpp#cConsts\", \"mJump\", 2, 30.5)
    let b = set_table(\"T.cpp#cConsts\", \"mGrid\", 1, 2, -7)
    let d = set_table(\"T.cpp#cConsts\", \"mOn\", true)
    let e = set_table(\"T.cpp#cConsts\", \"mMode\", Mode::Run)
    let f = set_table(\"T.cpp#cConsts\", \"mTint\", color(1.0, 0.5, 0.0))
    let g = set_table(\"T.cpp#cUps\", 1, vec3(0.0, 1.0, 0.0))
    let h = set_table(\"T.cpp#cBoxes\", i, \"mH\", 2.0)
    let k = set_table(\"T.cpp#cConsts\", \"mBox\", \"mW\", 9)
    log \"{a} {b} {d} {e} {f} {g} {h} {k}\"
}
",
    );
    for want in [
        "lumart_set_table(\"T.cpp#cConsts\", ",
        "(uint32_t)(8ull)",
        "(uint32_t)(22ull)",
        "(uint32_t)(24ull)",
        "(uint32_t)(28ull)",
        "(uint32_t)(56ull)",
        "(uint32_t)(12ull)",
        "(uint32_t)(48ull)",
        "(uint32_t)(4ull + (uint64_t)",
        " * 8ull)",
        ">= 4ull) && lumart_index_error(",
        "lumart_unit_byte(",
        ", 4u, &",
        ", 12u, &",
        ", 1u, &",
        ", 2u, &",
    ] {
        assert!(c.contains(want), "no {want:?} in:\n{c}");
    }
}

/// table reads a place back as `T?`: none when the game has no such table; an enum as its storage, a colour from
/// its four bytes.
#[test]
fn table_reads_a_place_as_an_option() {
    let c = build(
        "on start {
    let jump: f32? = table(\"T.cpp#cConsts\", \"mJump\", 0)
    let mode: Mode? = table(\"T.cpp#cModes\", 1)
    let tint: color? = table(\"T.cpp#cConsts\", \"mTint\")
    let speed = table(\"T.cpp#cSpeeds\", 2) ?? 0.0
    let run = mode == Mode::Run
    log \"{jump ?? 0.0} {run} {(tint ?? color(0.0, 0.0, 0.0)).r} {speed}\"
}
",
    );
    for want in ["lumart_table(\"T.cpp#cConsts\", ", "lumart_table(\"T.cpp#cModes\", ", "(uint32_t)(4ull)", "(uint32_t)(8ull)", "(float)("] {
        assert!(c.contains(want), "no {want:?} in:\n{c}");
    }
}

/// Each refusal by its code: a table the game does not list (S0150), a field its record does not have (S0051), a value
/// of another type (S0049), a pointer, a bit-field, a name that is no text literal, a step that does not fit, a
/// place left short (S0151, with why), a literal index outside (S0152).
#[test]
fn set_table_refuses_what_it_cannot_write() {
    let cases: &[(&str, u16, &str)] = &[
        ("set_table(\"T.cpp#cNone\", 1.0)", code::TABLE_UNKNOWN, "found=T.cpp#cNone"),
        ("set_table(\"T.cpp#cConsts\", \"mJumps\", 0, 1.0)", code::NO_MEMBER, "name=mJumps"),
        ("set_table(\"T.cpp#cConsts\", \"mJump\", 0, \"high\")", code::TYPE_MISMATCH, "expected=f32"),
        ("set_table(\"T.cpp#cConsts\", \"mOn\", 1)", code::TYPE_MISMATCH, "expected=bool"),
        ("set_table(\"T.cpp#cConsts\", \"mName\", \"x\")", code::TABLE_PLACE, "why=Script.Table.Pointer"),
        ("set_table(\"T.cpp#cNames\", 0, \"x\")", code::TABLE_PLACE, "why=Script.Table.Pointer"),
        ("set_table(\"T.cpp#cConsts\", \"mFlags\", 1)", code::TABLE_PLACE, "why=Script.Table.BitField"),
        ("set_table(\"T.cpp#c{1}\", \"mOn\", true)", code::TABLE_PLACE, "why=Script.Table.NotLiteral"),
        ("set_table(\"T.cpp#cConsts\", \"mJump\", 1.0)", code::TABLE_PLACE, "why=Script.Table.NeedsIndex"),
        ("set_table(\"T.cpp#cConsts\", 0, 1.0)", code::TABLE_PLACE, "why=Script.Table.NeedsField"),
        ("set_table(\"T.cpp#cConsts\", \"mOn\", 0, true)", code::TABLE_PLACE, "why=Script.Table.NotArray"),
        ("set_table(\"T.cpp#cSpeeds\", 0, \"x\", 1.0)", code::TABLE_PLACE, "why=Script.Table.NotRecord"),
        ("set_table(\"T.cpp#cBoxes\", 2, 1.0)", code::TABLE_PLACE, "why=Script.Table.NeedsField"),
        ("set_table(\"T.cpp#cConsts\", \"mJump\", 3, 1.0)", code::TABLE_INDEX, "count=3"),
        ("set_table(\"T.cpp#cBoxes\", -1, \"mW\", 1)", code::TABLE_INDEX, "found=-1"),
        ("set_table(\"T.cpp#cConsts\")", code::ARG_COUNT, "found=1"),
    ];
    for (call, want, arg) in cases {
        let got = said(&format!("on start {{\n    let r = {call}\n    log \"{{r}}\"\n}}\n"));
        assert!(got.iter().any(|(c, line, args)| c == want && *line == 2 && args.iter().any(|a| a == arg)), "{call}: want S{want:04} with {arg}, got {got:?}");
        assert_eq!(got.len(), 1, "{call}: one error, got {got:?}");
    }
}

/// as_class<T>(x): a view of a game object narrowed to T by the kernel's as_class (`T?`, none when it is no T or x is
/// none), x a view or an optional view of a class of T's line; the class's number is a kind 3 row.
#[test]
fn as_class_narrows_a_view_by_the_kernels_class_check() {
    let c = build(
        "on start {
    guard let a = MR::actor() else { return }
    if let k = as_class<Kuribo>(a) {
        log \"a Kuribo\"
    }
    let n = as_class<NameObj>(MR::actor())
    log \"{n == none}\"
}
",
    );
    assert_eq!(c.matches("->as_class(luma_rt_self, ").count(), 2, "{c}");
    // The narrowed view is the record's: its type is Kuribo?.
    let got = said("on start {\n    guard let a = MR::actor() else { return }\n    let k: Kuribo? = as_class<Kuribo>(a)\n    let n: LiveActor = as_class<Kuribo>(a)\n}\n");
    assert!(got.iter().any(|(c, line, _)| *c == code::MAY_BE_NONE && *line == 4), "{got:?}");
    assert_eq!(got.len(), 1, "{got:?}");
}

/// as_class refuses a class of another line (S0153), a value that is no view, and a missing type argument.
#[test]
fn as_class_refuses_another_line_of_classes() {
    let cases: &[(&str, u16)] = &[
        ("as_class<Gravity>(MR::actor())", code::AS_CLASS),
        ("as_class<Kuribo>(3)", code::AS_CLASS),
        ("as_class(MR::actor())", code::TYPE_ARG_COUNT),
    ];
    for (call, want) in cases {
        let got = said(&format!("on start {{\n    let r = {call}\n}}\n"));
        assert!(got.iter().any(|(c, line, _)| c == want && *line == 2), "{call}: want S{want:04}, got {got:?}");
        assert_eq!(got.len(), 1, "{call}: one error, got {got:?}");
    }
}

/// A host for one mod library: a core that counts every block it gives and takes back, the time service's frame
/// call, the two tables the small game has, message names that bind with no messages read yet (so `text "Key"` is a
/// row). It loads the library, fills its imports, runs begin, start, five frames, stop and finish, and prints each
/// block still held, then "held <blocks> <bytes>".
const END_HOST: &str = r#"#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <malloc.h>
#include "luma.h"
#include "luma_time.h"
#define MOST 65536
static void* blocks[MOST];
static uint64_t sizes[MOST];
static int used;
static void* alloc(const LumaMod* m, uint64_t s, uint64_t al) {
    (void)m;
    if (used == MOST) return 0;
    void* p = _aligned_malloc((size_t)s, (size_t)(al < 16 ? 16 : al));
    blocks[used] = p;
    sizes[used++] = s;
    return p;
}
static void release(const LumaMod* m, void* p) {
    (void)m;
    if (p == 0) return;
    for (int i = used - 1; i >= 0; i--) {
        if (blocks[i] == p) {
            blocks[i] = 0;
            _aligned_free(p);
            return;
        }
    }
    printf("freed a block it was not given\n");
}
static void say(const LumaMod* m, const char* t) { (void)m; printf("log %s\n", t); }
static void report(const LumaMod* m, const char* file, uint32_t line, uint32_t code, const char* const* args, uint32_t n) {
    (void)m; (void)args; (void)n; printf("report %s:%u %u\n", file, line, code);
}
static int32_t game_thread(void) { return 1; }

static void zero(void** args, void* result) { (void)args; if (result != 0) *(void**)result = 0; }
static LumaFunction any = { "any", "", zero, 0 };
static const LumaFunction* function(const char* name, const char* signature) { (void)name; (void)signature; return &any; }
static int32_t field(const char* c, const char* name, const char* type) { (void)c; (void)name; (void)type; return 0; }
static int32_t hook(const LumaMod* m, const LumaFunction* f, uint32_t k, LumaHook h, void* u) { (void)m; (void)f; (void)k; (void)h; (void)u; return 0; }
static const char* names[2];
static float speeds[3];
static int32_t table(const LumaMod* m, const char* name, LumaTable* out) {
    (void)m;
    if (strcmp(name, "T.cpp#cNames") == 0) { out->data = names; out->type = "const char *"; out->size = sizeof names; out->count = 2; return 0; }
    if (strcmp(name, "T.cpp#cSpeeds") == 0) { out->data = speeds; out->type = "float"; out->size = sizeof speeds; out->count = 3; return 0; }
    return -1;
}
static int32_t set_table(const LumaMod* m, const char* name, uint32_t offset, const void* bytes, uint32_t size) {
    LumaTable t;
    if (table(m, name, &t) != 0 || offset + size > t.size) return -1;
    memcpy((char*)t.data + offset, bytes, size);
    return 0;
}
static LumaTimeFn frame_fn;
static void* frame_user;
static LumaTimeNow clock_now;
static int32_t on_frame(const LumaMod* m, LumaTimeFn fn, void* user) { (void)m; frame_fn = fn; frame_user = user; return 0; }
static int32_t on_step(const LumaMod* m, LumaTimeFn fn, void* user) { (void)m; (void)fn; (void)user; return 0; }
static int32_t now(const LumaMod* m, LumaTimeNow* out) { (void)m; *out = clock_now; return 0; }
static LumaTimeService time_service;
static const void* service(const LumaMod* m, const char* name, uint32_t major, uint32_t minor) {
    (void)m; (void)major; (void)minor;
    return strcmp(name, LUMA_TIME_NAME) == 0 ? &time_service : 0;
}
static LumaCore core;
int main(int argc, char** argv) {
    if (argc < 2) return 2;
    core.size = sizeof core;
    core.api = LUMA_API;
    core.log = say;
    core.report = report;
    core.function = function;
    core.field = field;
    core.hook = hook;
    core.unhook = hook;
    core.service = service;
    core.game_thread = game_thread;
    core.alloc = alloc;
    core.free = release;
    core.table = table;
    core.set_table = set_table;
    time_service.on_frame = on_frame;
    time_service.on_step = on_step;
    time_service.now = now;
    clock_now.size = sizeof clock_now;
    HMODULE dll = LoadLibraryA(argv[1]);
    if (dll == 0) { printf("no library\n"); return 3; }
    const LumaModInfo* info = (const LumaModInfo*)(void*)GetProcAddress(dll, "luma_mod");
    if (info == 0) { printf("no luma_mod\n"); return 4; }
    const LumaMod* self = (const LumaMod*)(void*)&core;
    for (uint32_t i = 0; i < info->import_count; i++) {
        const LumaImport* r = &info->imports[i];
        switch (r->kind) {
        case LUMA_IMPORT_FUNCTION: *(const LumaFunction**)r->slot = &any; break;
        case LUMA_IMPORT_SERVICE: *(const void**)r->slot = service(self, r->name, 1, 0); break;
        case LUMA_IMPORT_FIELD: case LUMA_IMPORT_CLASS: *(int32_t*)r->slot = 0; break;
        case LUMA_IMPORT_SIZE: *(int32_t*)r->slot = 8; break;
        default: printf("import kind %u\n", r->kind); return 5;
        }
    }
    info->begin(&core, self);
    if (info->start(&core, self) != 0) { printf("did not start\n"); return 6; }
    for (int k = 0; k < 5; k++) {
        clock_now.frame++;
        if (frame_fn != 0) frame_fn(frame_user, &clock_now);
    }
    info->stop();
    info->finish();
    int held = 0;
    uint64_t bytes = 0;
    for (int i = 0; i < used; i++) {
        if (blocks[i] != 0) {
            printf("block of %llu bytes\n", (unsigned long long)sizes[i]);
            held++;
            bytes += sizes[i];
        }
    }
    printf("held %d %llu\n", held, (unsigned long long)bytes);
    return 0;
}
"#;

/// The library a program builds to, run in END_HOST: the host's lines.
fn run_to_the_end(mod_id: &str, path: &str, text: &str) -> Option<Vec<String>> {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("not run: LUMA_SDK is not set");
        return None;
    };
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let sources = [Source { path: path.into(), module }, lumascript::check::std_source()];
    let game = game();
    let files = lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        emit::emit_with(mod_id, "1.0.0", &sources, &[text.to_string()], &[], &checked, &game).unwrap_or_else(|d| panic!("{d:?}")).files
    })
    .unwrap();
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-end-{}-{mod_id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, t) in &files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, t).unwrap();
    }
    std::fs::create_dir_all(dir.join("source")).unwrap();
    let c_files: Vec<&String> = files.iter().map(|(rel, _)| rel).filter(|rel| rel.ends_with(".c")).collect();
    let out = std::process::Command::new(&clang)
        .current_dir(&dir)
        .arg("--target=x86_64-pc-windows-msvc")
        .args(emit::C_FLAGS)
        .arg("-I").arg(dir.join("lum")).arg("-I").arg(&sdk).arg("-I").arg(dir.join("source"))
        .args(["-shared", "-fuse-ld=lld", "-Wl,/NOENTRY", "-Wl,/NODEFAULTLIB", "-Wl,/NOIMPLIB"])
        .args(&c_files)
        .arg("-o").arg(dir.join("mod.dll"))
        .output()
        .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(out.status.success(), "the library does not link:\n{}", String::from_utf8_lossy(&out.stderr));
    std::fs::write(dir.join("host.c"), END_HOST).unwrap();
    let out = std::process::Command::new(&clang)
        .current_dir(&dir)
        .arg("--target=x86_64-pc-windows-msvc")
        .arg("-I").arg(&sdk)
        .arg("host.c").arg("-o").arg(dir.join("host.exe"))
        .output()
        .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(out.status.success(), "the host does not build:\n{}", String::from_utf8_lossy(&out.stderr));
    let run = std::process::Command::new(dir.join("host.exe")).arg(dir.join("mod.dll")).output().unwrap();
    let lines: Vec<String> = String::from_utf8_lossy(&run.stdout).lines().map(str::to_string).collect();
    assert!(run.status.success(), "exit {:?}: {lines:?}", run.status);
    let _ = std::fs::remove_dir_all(&dir);
    Some(lines)
}

/// The program both runs use: a task that waits, a list of three thousand texts (more than one pool page), a text row
/// made before the game's messages exist and read in a frame, a number written into a game table.
const ENDS: &str = "var greeting = \"\"
var lines: list<text> = []

task ticker(times: i32) {
    for i in 0..times {
        wait 1 frames
        log \"tick {i} {greeting} {lines.len()}\"
    }
}

on start {
    greeting = text \"Mod_Hi\"
    for i in 0..3000 {
        lines.push(\"line {i}\")
    }
    let b = set_table(\"T.cpp#cSpeeds\", 2, 4.5)
    log \"{b}\"
    start ticker(3)
}

on stop {
    log \"stopped\"
}
";

/// Every block the runtime takes from the core is given back by the end of the mod's finish: the pool pages, the
/// task lists, the row, the list. The same for a library mod (its finish is the same).
#[test]
fn every_block_a_mod_takes_is_given_back_by_its_finish() {
    let Some(lines) = run_to_the_end("ends", "main", ENDS) else { return };
    assert!(lines.iter().any(|l| l == "log true"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "log tick 2 Mod_Hi 3000"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "log stopped"), "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("held 0 0"), "{lines:?}");
    let library = format!("{ENDS}\npub fn count() -> i32 {{\n    return lines.len()\n}}\n");
    let Some(lines) = run_to_the_end("ends.kit", "kit", &library) else { return };
    assert!(lines.iter().any(|l| l == "log tick 2 Mod_Hi 3000"), "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("held 0 0"), "{lines:?}");
}
