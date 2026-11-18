//! `optional <game call>` (docs/modding/reference/lumascript/the-game-by-name.md): the function's kind 0 row carries
//! LUMA_OPTIONAL when only `optional` calls use it, so the mod loads on a game without the function and the slot stays
//! null. The call then runs nothing, its arguments included, and gives none (false for a function with no result);
//! with the slot filled it is the call, its result an option. One plain call of the function makes its row required.
//! `optional` before anything but a game call is S0162.

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::code;
use lumascript::emit;
use lumascript::game::Game;
use std::collections::HashMap;

const GAME: &str = r#"{"types": [{"kind": "alias", "name": "s32", "of": "int", "record": null}], "globals": [], "left_out": []}"#;

fn game() -> Game {
    let mut functions: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (name, sig) in [("MR::isPlayerDead", "bool()"), ("MR::stopBlur", "void()"), ("MR::setFlag", "void(bool)"), ("MR::getCount", "s32()")] {
        functions.entry(name.to_string()).or_default().push((sig.to_string(), false));
    }
    Game::new(functions, Some(Types::from_json(GAME.as_bytes()).unwrap()))
}

/// The checker's error codes and lines, and, with none, the mod's files.
fn build(text: &str) -> (Vec<(u16, u32)>, Vec<(String, String)>) {
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }, check::std_source()];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<(u16, u32)> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).map(|(_, d)| (d.code, d.at.line)).collect();
        if !errors.is_empty() {
            return (errors, Vec::new());
        }
        let out = emit::emit_with("optional", "1.0.0", &sources, &[text.to_string()], &[], &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        (errors, out.files)
    })
    .unwrap()
}

fn file<'f>(files: &'f [(String, String)], name: &str) -> &'f str {
    files.iter().find(|(p, _)| p == name).map(|(_, c)| c.as_str()).unwrap_or_else(|| panic!("no {name}"))
}

const PROGRAM: &str = "var counted = 0

fn arg() -> bool {
    counted += 1
    return true
}

on start {
    let dead = optional MR::isPlayerDead()
    log \"dead {dead ?? false} {dead == none}\"
    let ran = optional MR::stopBlur()
    log \"ran {ran}\"
    let set = optional MR::setFlag(arg())
    log \"set {set} counted {counted}\"
    let n = optional MR::getCount()
    log \"count {n ?? -1}\"
    log \"required {MR::getCount()}\"
}
";

#[test]
fn a_function_only_optional_calls_use_is_imported_optional() {
    let (e, files) = build(PROGRAM);
    assert!(e.is_empty(), "{e:?}");
    let imports = file(&files, "lum/imports.c");
    for name in ["MR::isPlayerDead", "MR::stopBlur", "MR::setFlag"] {
        assert!(imports.contains(&format!("{{LUMA_IMPORT_FUNCTION, LUMA_OPTIONAL, \"{name}\"")), "{name}: {imports}");
    }
    // A plain call of MR::getCount besides the optional one: required.
    assert!(imports.contains("LUMA_FUNCTION(") && imports.contains("\"MR::getCount\", \"s32()\")"), "{imports}");
    assert!(!imports.contains("LUMA_OPTIONAL, \"MR::getCount\""), "{imports}");
}

#[test]
fn an_optional_call_in_an_optional_calls_arguments_leaves_both_optional() {
    let (e, files) = build("on start {\n    let set = optional MR::setFlag(optional MR::isPlayerDead() ?? false)\n    log \"{set}\"\n}\n");
    assert!(e.is_empty(), "{e:?}");
    let imports = file(&files, "lum/imports.c");
    for name in ["MR::isPlayerDead", "MR::setFlag"] {
        assert!(imports.contains(&format!("{{LUMA_IMPORT_FUNCTION, LUMA_OPTIONAL, \"{name}\"")), "{name}: {imports}");
    }
}

#[test]
fn optional_goes_before_a_game_call_only() {
    for (text, line) in [
        ("on start {\n    let x = optional 3\n}\n", 2),
        ("fn f() -> int {\n    return 1\n}\n\non start {\n    let x = optional f()\n}\n", 6),
        ("on start {\n    let n = 2\n    let x = optional n\n}\n", 3),
    ] {
        let (e, _) = build(text);
        assert_eq!(e, vec![(code::OPTIONAL_NOT_GAME_CALL, line)], "{text}");
    }
    // A name `optional` is still a name.
    let (e, _) = build("on start {\n    let optional = 2\n    log \"{optional + 1}\"\n}\n");
    assert!(e.is_empty(), "{e:?}");
}

/// A host that fills each required row's slot and, with `full`, each optional one's; leaves an optional slot empty
/// otherwise, as the loader does on a game without the function.
const HOST: &str = r#"#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <malloc.h>
#include "luma.h"
static void* alloc(const LumaMod* m, uint64_t s, uint64_t al) { (void)m; return _aligned_malloc((size_t)s, (size_t)(al < 16 ? 16 : al)); }
static void release(const LumaMod* m, void* p) { (void)m; if (p != 0) _aligned_free(p); }
static void say(const LumaMod* m, const char* t) { (void)m; printf("%s\n", t); }
static void report(const LumaMod* m, const char* file, uint32_t line, uint32_t code, const char* const* args, uint32_t n) {
    (void)m; (void)args; (void)n; printf("report %s:%u %u\n", file, line, code);
}
static int32_t game_thread(void) { return 1; }
static void dead(void** args, void* result) { (void)args; *(unsigned char*)result = 1; }
static void stop_blur(void** args, void* result) { (void)args; (void)result; printf("stopBlur called\n"); }
static void set_flag(void** args, void* result) { (void)result; printf("setFlag called %d\n", (int)**(unsigned char**)&args[1]); }
static void count(void** args, void* result) { (void)args; *(int32_t*)result = 7; }
static LumaFunction fns[] = {
    {"MR::isPlayerDead", "bool()", dead, 0},
    {"MR::stopBlur", "void()", stop_blur, 0},
    {"MR::setFlag", "void(bool)", set_flag, 0},
    {"MR::getCount", "s32()", count, 0},
};
static LumaCore core;
int main(int argc, char** argv) {
    if (argc < 3) return 2;
    int full = strcmp(argv[2], "full") == 0;
    core.size = sizeof core;
    core.api = LUMA_API;
    core.log = say;
    core.report = report;
    core.game_thread = game_thread;
    core.alloc = alloc;
    core.free = release;
    HMODULE dll = LoadLibraryA(argv[1]);
    if (dll == 0) { printf("no library\n"); return 3; }
    const LumaModInfo* info = (const LumaModInfo*)(void*)GetProcAddress(dll, "luma_mod");
    if (info == 0) { printf("no luma_mod\n"); return 4; }
    const LumaMod* self = (const LumaMod*)(void*)&core;
    for (uint32_t i = 0; i < info->import_count; i++) {
        const LumaImport* r = &info->imports[i];
        if (r->kind != LUMA_IMPORT_FUNCTION) continue;
        if ((r->flags & LUMA_OPTIONAL) != 0 && !full) { printf("left empty %s\n", r->name); continue; }
        for (unsigned k = 0; k < sizeof fns / sizeof fns[0]; k++) {
            if (strcmp(fns[k].name, r->name) == 0) *(const LumaFunction**)r->slot = &fns[k];
        }
    }
    info->begin(&core, self);
    if (info->start(&core, self) != 0) { printf("did not start\n"); return 6; }
    info->stop();
    info->finish();
    return 0;
}
"#;

/// The program's own build linked as a library and run by HOST with `mode`: its lines. None without the SDK.
fn run(mode: &str) -> Option<Vec<String>> {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("not run: LUMA_SDK is not set");
        return None;
    };
    let (e, files) = build(PROGRAM);
    assert!(e.is_empty(), "{e:?}");
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-optional-{}-{mode}", std::process::id()));
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
    std::fs::write(dir.join("host.c"), HOST).unwrap();
    let out = std::process::Command::new(&clang)
        .current_dir(&dir)
        .arg("--target=x86_64-pc-windows-msvc")
        .arg("-I").arg(&sdk)
        .arg("host.c").arg("-o").arg(dir.join("host.exe"))
        .output()
        .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(out.status.success(), "the host does not build:\n{}", String::from_utf8_lossy(&out.stderr));
    let ran = std::process::Command::new(dir.join("host.exe")).arg(dir.join("mod.dll")).arg(mode).output().unwrap();
    let lines: Vec<String> = String::from_utf8_lossy(&ran.stdout).lines().map(str::to_string).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ran.status.code(), Some(0), "{lines:?}");
    Some(lines)
}

#[test]
fn an_empty_slot_runs_nothing_and_gives_none() {
    let Some(lines) = run("empty") else { return };
    assert_eq!(
        lines,
        [
            "left empty MR::isPlayerDead",
            "left empty MR::setFlag",
            "left empty MR::stopBlur",
            "dead false true",
            "ran false",
            "set false counted 0",
            "count 7",
            "required 7",
        ],
        "{lines:#?}"
    );
}

#[test]
fn a_filled_slot_is_the_call() {
    let Some(lines) = run("full") else { return };
    assert_eq!(
        lines,
        ["dead true false", "stopBlur called", "ran true", "setFlag called 1", "set true counted 1", "count 7", "required 7"],
        "{lines:#?}"
    );
}
