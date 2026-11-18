//! A mod's `test` blocks (docs/modding/reference/lumascript/tests.md): left out of the mod's own build; in a test build
//! the plain tests run from `luma_tests` with no game (a host as LumaLink's runner is), and the game tests one after
//! another from start, each saying its end as a fact. Built and run with clang against the SDK (`LUMA_SDK`).

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::emit;
use lumascript::game::Game;
use std::collections::HashMap;

const GAME: &str = r#"{"types": [{"kind": "alias", "name": "s32", "of": "int", "record": null}], "globals": [], "left_out": []}"#;

/// A game with one function and, when LUMA_SDK names the SDK, the services its headers declare.
fn game() -> Game {
    let mut functions: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    functions.entry("MR::isPlayerDead".to_string()).or_default().push(("bool()".to_string(), false));
    let mut game = Game::new(functions, Some(Types::from_json(GAME.as_bytes()).unwrap()));
    if let Some(sdk) = std::env::var_os("LUMA_SDK") {
        let mut headers = std::collections::BTreeMap::new();
        for e in std::fs::read_dir(&sdk).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("luma") && name.ends_with(".h") {
                headers.insert(name, std::fs::read_to_string(e.path()).unwrap());
            }
        }
        let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
        assert!(errors.is_empty(), "{errors:?}");
        game.services = services;
    }
    game
}

/// The C of a program, as a test build (`tests`) or the mod's own.
fn files(text: &str, tests: bool) -> Vec<(String, String)> {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let sources = [Source { path: "main".into(), module }, check::std_source()];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let emitted = if tests {
            emit::emit_tests("tested", "1.0.0", &sources, &[text.to_string()], &[], &checked, &game)
        } else {
            emit::emit_with("tested", "1.0.0", &sources, &[text.to_string()], &[], &checked, &game)
        };
        emitted.unwrap_or_else(|d| panic!("{d:?}")).files
    })
    .unwrap()
}

/// The program's test build linked as a library and run by `host` (C): its exit code and its lines. None without the SDK.
fn run(name: &str, text: &str, host: &str) -> Option<(i32, Vec<String>)> {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("not run: LUMA_SDK is not set");
        return None;
    };
    let files = files(text, true);
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-tests-{}-{name}", std::process::id()));
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
    std::fs::write(dir.join("host.c"), host).unwrap();
    let out = std::process::Command::new(&clang)
        .current_dir(&dir)
        .arg("--target=x86_64-pc-windows-msvc")
        .arg("-I").arg(&sdk).arg("-I").arg(dir.join("lum"))
        .arg("host.c").arg("-o").arg(dir.join("host.exe"))
        .output()
        .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(out.status.success(), "the host does not build:\n{}", String::from_utf8_lossy(&out.stderr));
    let ran = std::process::Command::new(dir.join("host.exe")).arg(dir.join("mod.dll")).output().unwrap();
    let lines: Vec<String> = String::from_utf8_lossy(&ran.stdout).lines().map(str::to_string).collect();
    let _ = std::fs::remove_dir_all(&dir);
    Some((ran.status.code().unwrap_or(-1), lines))
}

/// LumaLink's runner as C: luma_tests with a host whose lines it prints; a name of the game reached prints
/// "touched NAME" and ends the process with 3; the option `speed` is "2.5" by default.
const PLAIN_HOST: &str = r#"#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <malloc.h>
#include "lumart.h"
static void say(const char* line) { printf("%s\n", line); }
static void touched(const char* what) { printf("touched %s\n", what); fflush(stdout); ExitProcess(3); }
static void* take(uint64_t size, uint64_t align) { return _aligned_malloc((size_t)size, (size_t)(align < 16 ? 16 : align)); }
static void give(void* p) { _aligned_free(p); }
static int64_t option(const char* key, char* buf, uint64_t size) {
    if (strcmp(key, "speed") != 0) return -1;
    if (buf != 0 && size >= 4) memcpy(buf, "2.5", 4);
    return 3;
}
int main(int argc, char** argv) {
    if (argc < 2) return 2;
    HMODULE dll = LoadLibraryA(argv[1]);
    if (dll == 0) { printf("no library\n"); return 4; }
    typedef int32_t (*Tests)(const LumartTestHost*);
    Tests tests = (Tests)(void*)GetProcAddress(dll, "luma_tests");
    if (tests == 0) { printf("no luma_tests\n"); return 5; }
    LumartTestHost host = { sizeof host, say, touched, take, give, option };
    printf("ran %d\n", (int)tests(&host));
    return 0;
}
"#;

const PROGRAM: &str = "let base = 40

fn count_after_throws(throws: int) -> int {
    if throws > 2 {
        return 2
    }
    return throws
}

struct Point {
    x: int
    y: int
}

test \"two balls at most\" {
    expect count_after_throws(5) == 2
}

test \"sides shown\" {
    expect count_after_throws(1) == 2
    expect base + 2 == 42
    let a = Point { x: 1, y: 2 }
    let b = Point { x: 1, y: 3 }
    expect a == b
    expect \"a{base}\" != \"a41\"
}

test \"a script error ends it\" {
    let l = [1, 2]
    expect l[5] == 1
    expect false
}

test game \"waits a frame\" {
    wait 1 frames
    expect base == 40
}

test game \"at once\" {
    expect base == 41
}
";

#[test]
fn the_mods_own_build_leaves_tests_out() {
    // The runtime's header declares the test part for every build; only a test build's lumart.c defines it.
    let rt = |tests: bool| files(PROGRAM, tests).into_iter().find(|(p, _)| p == "lum/lumart.c").unwrap().1;
    assert!(!rt(false).starts_with("#define LUMART_TESTS") && rt(true).starts_with("#define LUMART_TESTS 1\n"));
    let generated = |tests: bool| -> String { files(PROGRAM, tests).iter().filter(|(p, _)| !p.starts_with("lum/lumart.")).map(|(_, t)| t.as_str()).collect() };
    let c = generated(false);
    assert!(!c.contains("luma_tests") && !c.contains("lm_test_") && !c.contains("lumart_test"), "{c}");
    let c = generated(true);
    assert!(c.contains("LUMA_EXPORT int32_t luma_tests(") && c.contains("lumart_tests_game(lm_game_tests, 2);"));
}

/// Each plain test in written order, its failed expects with both sides, its end, then the count; a script error ends
/// its test (reported, the test an error) and the next runs.
#[test]
fn plain_tests_run_in_order_and_say_each_failure() {
    let Some((code, lines)) = run("plain", PROGRAM, PLAIN_HOST) else { return };
    assert_eq!(code, 0, "{lines:?}");
    let expected = [
        "fact test {\"name\": \"two balls at most\", \"result\": \"pass\", \"failed\": 0}",
        "fact expect {\"test\": \"sides shown\", \"file\": \"main.lum\", \"line\": 20, \"text\": \"count_after_throws(1) == 2\", \"op\": \"==\", \"left\": 1, \"right\": 2}",
        "fact expect {\"test\": \"sides shown\", \"file\": \"main.lum\", \"line\": 24, \"text\": \"a == b\", \"op\": \"==\", \"left\": {\"x\": 1, \"y\": 2}, \"right\": {\"x\": 1, \"y\": 3}}",
        "fact test {\"name\": \"sides shown\", \"result\": \"fail\", \"failed\": 2}",
    ];
    for (i, e) in expected.iter().enumerate() {
        assert_eq!(lines.get(i).map(String::as_str), Some(*e), "{lines:#?}");
    }
    let error = lines.iter().position(|l| l.starts_with("fact error {\"test\": \"a script error ends it\", \"file\": \"main.lum\", \"line\": 30")).unwrap_or_else(|| panic!("{lines:#?}"));
    assert_eq!(lines[error + 1], "fact test {\"name\": \"a script error ends it\", \"result\": \"error\", \"failed\": 0}");
    assert_eq!(lines[error + 2], "fact tests {\"kind\": \"plain\", \"count\": 3, \"passed\": 1}");
    assert_eq!(lines[error + 3], "ran 0");
    assert!(!lines.iter().any(|l| l.contains("waits a frame") || l.contains("at once")), "game tests never run here: {lines:#?}");
}

/// A plain test that reaches the game stops there, the name said; so does one that reaches a service. An option is
/// its default as the runner gives it.
#[test]
fn a_plain_test_that_reaches_the_game_names_what_it_reached() {
    let text = "test \"dead\" {\n    expect not MR::isPlayerDead()\n}\n";
    let Some((code, lines)) = run("touched", text, PLAIN_HOST) else { return };
    assert_eq!(code, 3, "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("touched MR::isPlayerDead"), "{lines:?}");
    let text = "use svc.time\n\ntest \"time\" {\n    let n = time.monotonic_ns()\n}\n";
    let Some((code, lines)) = run("touched-service", text, PLAIN_HOST) else { return };
    assert_eq!(code, 3, "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("touched time"), "{lines:?}");
}

/// The game as a host is: start (after every `on start`) runs the game tests one after another, the one that waits
/// ending a frame later and starting the next; the count once all have ended.
const GAME_HOST: &str = r#"#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <malloc.h>
#include "luma.h"
#include "luma_time.h"
static void* alloc(const LumaMod* m, uint64_t s, uint64_t al) { (void)m; return _aligned_malloc((size_t)s, (size_t)(al < 16 ? 16 : al)); }
static void release(const LumaMod* m, void* p) { (void)m; if (p != 0) _aligned_free(p); }
static void say(const LumaMod* m, const char* t) { (void)m; printf("%s\n", t); }
static void report(const LumaMod* m, const char* file, uint32_t line, uint32_t code, const char* const* args, uint32_t n) {
    (void)m; (void)args; (void)n; printf("report %s:%u %u\n", file, line, code);
}
static int32_t game_thread(void) { return 1; }
static void zero(void** args, void* result) { (void)args; if (result != 0) *(unsigned char*)result = 0; }
static LumaFunction any = { "any", "", zero, 0 };
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
    core.service = service;
    core.game_thread = game_thread;
    core.alloc = alloc;
    core.free = release;
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
        if (r->kind == LUMA_IMPORT_FUNCTION) *(const LumaFunction**)r->slot = &any;
        if (r->kind == LUMA_IMPORT_SERVICE) *(const void**)r->slot = service(self, r->name, 1, 0);
    }
    info->begin(&core, self);
    if (info->start(&core, self) != 0) { printf("did not start\n"); return 6; }
    printf("started\n");
    for (int k = 0; k < 3; k++) {
        clock_now.frame++;
        printf("frame %d\n", k);
        if (frame_fn != 0) frame_fn(frame_user, &clock_now);
    }
    info->stop();
    info->finish();
    return 0;
}
"#;

#[test]
fn game_tests_run_one_after_another_from_start() {
    let text = format!("on start {{\n    log \"on start\"\n}}\n\n{PROGRAM}");
    let Some((code, lines)) = run("game", &text, GAME_HOST) else { return };
    assert_eq!(code, 0, "{lines:?}");
    let want = [
        "on start",
        "started",
        "frame 0",
        "fact test {\"name\": \"waits a frame\", \"result\": \"pass\", \"failed\": 0}",
        "fact expect {\"test\": \"at once\", \"file\": \"main.lum\", \"line\": 44, \"text\": \"base == 41\", \"op\": \"==\", \"left\": 40, \"right\": 41}",
        "fact test {\"name\": \"at once\", \"result\": \"fail\", \"failed\": 1}",
        "fact tests {\"kind\": \"game\", \"count\": 2, \"passed\": 1}",
        "frame 1",
        "frame 2",
    ];
    assert_eq!(lines, want, "{lines:#?}");
}

/// A game test still running when the mod stops ends as stopped; the next never starts and the count is never said.
#[test]
fn a_game_test_the_mod_stops_is_said_stopped() {
    let text = "test game \"long\" {\n    wait 100 frames\n}\n\ntest game \"never\" {\n    expect true\n}\n";
    let Some((code, lines)) = run("stopped", text, GAME_HOST) else { return };
    assert_eq!(code, 0, "{lines:?}");
    assert!(lines.contains(&"fact test {\"name\": \"long\", \"result\": \"stopped\", \"failed\": 0}".to_string()), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("never") || l.starts_with("fact tests ")), "{lines:#?}");
}

/// A failed expect shows its value as written: the spaces of a line kept, a string whole with what it holds, a call's
/// arguments, a line break one space, the comment after it left out.
#[test]
fn an_expect_keeps_its_text_as_written() {
    let text = "test \"t\" {\n    expect  pick( 1,2 ) == \"x{base}\"   // why\n    expect [1,\n        2] != []\n}\n";
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "{diags:?}");
    let lumascript::ast::ItemKind::Test { body, .. } = &module.items[0].kind else { panic!("{:?}", module.items) };
    let written: Vec<&str> = body
        .stmts
        .iter()
        .filter_map(|s| match &s.kind {
            lumascript::ast::StmtKind::Expect { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(written, ["pick( 1,2 ) == \"x{base}\"", "[1, 2] != []"]);
}
