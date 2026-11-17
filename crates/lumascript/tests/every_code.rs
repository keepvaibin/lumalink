//! Every code the compiler can say (diag::code::ALL) has a test that makes it: one of the workspace's tests names
//! it (`code::NAME`), or it is listed below with the reason no program can.

use std::path::Path;

/// Codes no program meets, and why.
const NO_PROGRAM: &[(&str, &str)] = &[
    ("NO_COMPILER_THREAD", "the compiler's own thread failing to start"),
    ("TASK_CALL", "retired: a task called gives its pending; nothing says it"),
];

/// The Rust files under `dir`: whole (`unit_only` false), or from each file's `#[cfg(test)]` on (its unit tests).
fn sources(dir: &Path, unit_only: bool, out: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, unit_only, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            match (unit_only, text.find("#[cfg(test)]")) {
                (false, _) => out.push_str(&text),
                (true, Some(at)) => out.push_str(&text[at..]),
                (true, None) => {}
            }
        }
    }
}

#[test]
fn every_code_has_a_test_that_makes_it() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let diag = std::fs::read_to_string(crates.join("lumascript/src/diag.rs")).unwrap();
    // The names in ALL, as diag.rs lists them.
    let all = &diag[diag.find("pub const ALL").expect("diag.rs has ALL")..];
    let all = &all[all.find("= &[").unwrap() + 4..all.find("];").unwrap()];
    let names: Vec<&str> = all.split(',').map(|n| n.trim()).filter(|n| !n.is_empty()).collect();
    assert_eq!(names.len(), lumascript::diag::code::ALL.len(), "ALL read whole");

    // The tests: each crate's tests/ folder, and the unit tests inside the sources (a `#[cfg(test)]` module).
    let mut tests = String::new();
    for krate in std::fs::read_dir(&crates).unwrap().flatten() {
        sources(&krate.path().join("tests"), false, &mut tests);
        sources(&krate.path().join("src"), true, &mut tests);
    }
    // This file names no code as `code::` (it would count itself).
    let mut missing = Vec::new();
    for name in &names {
        let excused = NO_PROGRAM.iter().any(|(n, _)| n == name);
        let named = tests.contains(&format!("code::{name}"));
        assert!(!(excused && named), "{name} is listed as made by no program, but a test makes it: take it off the list");
        if !excused && !named {
            missing.push(*name);
        }
    }
    assert!(missing.is_empty(), "codes with no test that makes them: {missing:?}");
}
