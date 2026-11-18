//! `lumalink test` on a mod with `test` blocks and no test folder (docs/modding/reference/lumascript/tests.md): the
//! mod is built as a test build in a copy under the work folder, and the plain tests run in a process of their own
//! with no game: each test said, each failed `expect` with its sides, a test that reaches the game named with what it
//! reached, the count; the exit code 0 only when every test passed. The mod's own folder is left as it was.
//!
//! Needs LLVM 22.1.6 (LUMA_CLANG, else the default), the SDK (LUMA_SDK) and the game's names: LUMA_ROWS (a folder
//! with luma_types.json, else LUMA_TYPES's folder). Without them the test says which is missing and checks nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest(id: &str) -> String {
    let (major, minor) = luma_validate::FORMAT;
    format!(
        "{{\n  \"format\": \"{major}.{minor}\",\n  \"id\": \"{id}\",\n  \"version\": \"1.0.0\",\n  \"name\": \"Blocks\",\n  \"summary\": \"Tests in a mod's code.\",\n  \"authors\": [\"keepvaibin\"],\n  \"game\": {{ \"id\": \"RMG\" }}\n}}\n"
    )
}

/// The game's names (a folder with luma_types.json) and the SDK, from the gate's variables.
fn inputs() -> Option<(PathBuf, PathBuf)> {
    let Some(sdk) = std::env::var_os("LUMA_SDK").map(PathBuf::from) else {
        eprintln!("skipped: LUMA_SDK is not set (the folder with luma.h)");
        return None;
    };
    let rows = std::env::var_os("LUMA_ROWS").map(PathBuf::from).filter(|r| r.join("luma_types.json").is_file());
    let types = std::env::var_os("LUMA_TYPES").map(PathBuf::from).and_then(|t| t.parent().map(Path::to_path_buf));
    let Some(names) = rows.or(types).filter(|n| n.join("luma_types.json").is_file()) else {
        eprintln!("skipped: neither LUMA_ROWS nor LUMA_TYPES names a folder with luma_types.json");
        return None;
    };
    Some((names, sdk))
}

/// `lumalink test` on a mod of one script; (exit code, output, error output).
fn test(dir: &Path, id: &str, script: &str, names: &Path, sdk: &Path) -> (i32, String, String) {
    let folder = dir.join(id);
    std::fs::create_dir_all(folder.join("source")).unwrap();
    std::fs::write(folder.join("luma.json"), manifest(id)).unwrap();
    std::fs::write(folder.join("source/main.lum"), script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lumalink"))
        .args(["--language", "UsEnglish", "test"])
        .arg(&folder)
        .arg("--bin")
        .arg(dir.join("no-game"))
        .arg("--work")
        .arg(dir.join(format!("{id}-work")))
        .env("LUMA_NAMES", names)
        .env("LUMA_INCLUDE", sdk)
        .output()
        .unwrap();
    // The mod's own folder: its luma.json and its source, no code built into it.
    assert_eq!(std::fs::read_to_string(folder.join("luma.json")).unwrap(), manifest(id));
    assert!(!folder.join("code").exists(), "the test build was written into the mod's own folder");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

const SCRIPT: &str = "let base = 40

fn count_after_throws(throws: int) -> int {
    if throws > 2 {
        return 2
    }
    return throws
}

test \"two balls at most\" {
    expect count_after_throws(5) == 2
}

test \"sides shown\" {
    expect count_after_throws(1) == 2
    expect base + 2 == 42
}

test \"after a failure\" {
    expect base == 40
}
";

#[test]
#[ignore = "needs LLVM 22.1.6, the SDK (LUMA_SDK) and the game's names (LUMA_ROWS or LUMA_TYPES)"]
fn plain_tests_run_without_the_game_and_each_is_said() {
    let Some((names, sdk)) = inputs() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = test(dir.path(), "test.blocks", SCRIPT, &names, &sdk);
    assert_eq!(code, 1, "{out}\n{err}");
    for line in [
        "Test \"two balls at most\": passed",
        "Test \"sides shown\": failed",
        "main.lum:15: expect count_after_throws(1) == 2 does not hold: 1 == 2",
        "Test \"after a failure\": passed",
        "Tests in the mod's code: 2 of 3 passed",
    ] {
        assert!(out.contains(line), "no {line:?} in:\n{out}\n{err}");
    }
    // The second expect of the failed test held: only the one that did not is said.
    assert!(!out.contains("base + 2 == 42 does not hold"), "{out}");

    // Every test passing: exit 0.
    let passing = "test \"holds\" {\n    expect 1 + 1 == 2\n}\n";
    let (code, out, err) = test(dir.path(), "test.passing", passing, &names, &sdk);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("Tests in the mod's code: 1 of 1 passed"), "{out}\n{err}");
}

#[test]
#[ignore = "needs LLVM 22.1.6, the SDK (LUMA_SDK) and the game's names (LUMA_ROWS or LUMA_TYPES)"]
fn a_plain_test_that_reaches_a_service_is_named_and_the_rest_do_not_run() {
    let Some((names, sdk)) = inputs() else { return };
    let dir = tempfile::tempdir().unwrap();
    let script = "use svc.time\n\ntest \"first\" {\n    expect true\n}\n\ntest \"clock\" {\n    expect time.monotonic_ns() > 0\n}\n\ntest \"after\" {\n    expect true\n}\n";
    let (code, out, err) = test(dir.path(), "test.touched", script, &names, &sdk);
    assert_eq!(code, 1, "{out}\n{err}");
    for line in [
        "Test \"first\": passed",
        "Test \"clock\": it reached time, which needs the game; write it as a test game block",
        "Test \"after\": not run",
        "Tests in the mod's code: 1 of 3 passed",
    ] {
        assert!(out.contains(line), "no {line:?} in:\n{out}\n{err}");
    }
}

const GAME_SCRIPT: &str = "use svc.time

let base = 40
var started: i64 = 0

on start {
    started = time.monotonic_ns()
}

fn double(x: int) -> int {
    return x * 2
}

test \"doubles\" {
    expect double(base) == 80
}

test game \"start ran first\" {
    expect started > 0
}

test game \"waits two frames\" {
    let before = time.monotonic_ns()
    wait 2 frames
    expect time.monotonic_ns() > before
}

test game \"says a failure\" {
    expect double(base) == 81
}
";

/// The game tests in the game: a game build (LUMA_TEST_BIN, with its pak.path and test extension) and Nebula's runner
/// (LUMA_TEST_RUNNER). They run after `on start`, one after another, the one that waits ending frames later; each is
/// said from the run's facts, the failed one with its sides, and the harness test fails with the count.
#[test]
#[ignore = "needs LLVM 22.1.6, the SDK (LUMA_SDK), a game build (LUMA_TEST_BIN) and its runner (LUMA_TEST_RUNNER)"]
fn game_tests_run_in_the_game_one_after_another() {
    let Some(sdk) = std::env::var_os("LUMA_SDK").map(PathBuf::from) else {
        eprintln!("skipped: LUMA_SDK is not set (the folder with luma.h)");
        return;
    };
    let (Some(bin), Some(runner)) = (std::env::var_os("LUMA_TEST_BIN").map(PathBuf::from), std::env::var_os("LUMA_TEST_RUNNER").map(PathBuf::from)) else {
        eprintln!("skipped: LUMA_TEST_BIN and LUMA_TEST_RUNNER name no game build and runner");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("test.game");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    std::fs::write(folder.join("luma.json"), manifest("test.game")).unwrap();
    std::fs::write(folder.join("source/main.lum"), GAME_SCRIPT).unwrap();
    let renderer = if cfg!(windows) { "d3d12" } else { "vulkan" };
    let out = Command::new(env!("CARGO_BIN_EXE_lumalink"))
        .args(["--language", "UsEnglish", "test"])
        .arg(&folder)
        .arg("--bin")
        .arg(&bin)
        .arg("--runner")
        .arg(&runner)
        .arg("--work")
        .arg(dir.path().join("work"))
        .args(["--renderer", renderer])
        .env("LUMA_INCLUDE", &sdk)
        .env_remove("LUMA_NAMES")
        .output()
        .unwrap();
    let (code, out, err) = (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(code, 1, "{out}\n{err}");
    for line in [
        "Test \"doubles\": passed",
        "Tests in the mod's code: 1 of 1 passed",
        &format!("lumascript.game-tests ({renderer}): fail"),
        "pass  the mod test.game loaded: found true",
        "Test \"start ran first\": passed",
        "Test \"waits two frames\": passed",
        "Test \"says a failure\": failed",
        "main.lum:29: expect double(base) == 81 does not hold: 80 == 81",
        "Game tests in the mod's code: 2 of 3 passed",
        "0 of 1 tests passed",
    ] {
        assert!(out.contains(line), "no {line:?} in:\n{out}\n{err}");
    }
}

/// A mod with neither a test folder nor a test block is refused by name, naming both.
#[test]
fn a_mod_without_tests_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("none");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    std::fs::write(folder.join("luma.json"), manifest("test.none")).unwrap();
    std::fs::write(folder.join("source/main.lum"), "on start {\n    log \"hi\"\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lumalink")).args(["--language", "UsEnglish", "test"]).arg(&folder).arg("--bin").arg(dir.path()).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("has no tests") && err.contains("a test block in the mod's LumaScript"), "{err}");
}
