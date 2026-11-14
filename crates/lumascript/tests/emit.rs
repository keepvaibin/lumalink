//! The back end on the programs of the corpus, against a kernel build's description:
//! `LUMA_ROWS=<build>/luma LUMA_TYPES=<build>/luma_types/luma_types.json cargo test -p lumascript --test emit -- --ignored
//! --nocapture`. Each program the checker passes is written as C; what the back end does not write yet (S0800) is
//! counted by construct, for reading. Never a panic.

use luma_validate::names::{FunctionFacts, Names, Types};
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::code;
use lumascript::game::Game;
use lumascript::{emit, Arg};
use std::collections::BTreeMap;
use std::path::Path;

fn game() -> Game {
    let rows = std::env::var("LUMA_ROWS").expect("LUMA_ROWS names a kernel build's luma folder");
    let mut names = Names::from_rows_folder(Path::new(&rows), 1).unwrap_or_else(|e| panic!("{e}"));
    if names.types.is_none() {
        let path = std::env::var("LUMA_TYPES").expect("LUMA_TYPES names its luma_types.json");
        names.types = Some(Types::from_json(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{path}: {e}")));
        let facts = Path::new(&path).with_file_name(luma_validate::names::FUNCTIONS_FILE);
        if names.function_facts.is_none() && facts.is_file() {
            names.function_facts = Some(FunctionFacts::from_json(&std::fs::read(&facts).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", facts.display())));
        }
    }
    Game::from_names(&names)
}

#[test]
#[ignore]
fn the_programs_emit_against_a_kernel_build() {
    let game = game();
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/programs");
    let modes = std::fs::read_to_string(folder.join("modes.txt")).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "lum")).collect();
    files.sort();
    let (mut written, mut not_checked) = (0, 0);
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        let name = f.file_name().unwrap().to_str().unwrap().to_string();
        if modes.lines().any(|m| m == name) {
            continue;
        }
        let src = std::fs::read_to_string(f).unwrap().replace("\r\n", "\n");
        let module = name.trim_end_matches(".lum").replace(['.', '-'], "_");
        let (parsed, _) = lumascript::parse_module(&src);
        let sources = [Source { path: module, module: parsed }];
        let out = lumascript::on_compiler_stack(|| {
            let checked = check::check(&sources, &[], ModFacts::of_game(&game));
            if checked.diags.iter().any(|(_, d)| !lumascript::diag::is_note(d.code)) {
                return None;
            }
            Some(emit::emit("corpus", &sources, &checked, &game))
        })
        .unwrap();
        match out {
            None => not_checked += 1,
            Some(Ok(_)) => {
                written += 1;
                eprintln!("{name}: written");
            }
            Some(Err(diags)) => {
                for (_, d) in &diags {
                    assert_eq!(d.code, code::BACKEND_NOT_YET);
                    let what = d.args.iter().map(|(_, a)| match a {
                        Arg::Text(t) => t.clone(),
                        Arg::Phrase(p) => p.to_string(),
                    });
                    *missing.entry(what.collect::<Vec<_>>().join(" ")).or_default() += 1;
                }
                eprintln!("{name}: {} not written", diags.len());
            }
        }
    }
    eprintln!("written {written}, checker errors {not_checked}");
    let mut by: Vec<_> = missing.into_iter().collect();
    by.sort_by(|a, b| b.1.cmp(&a.1));
    for (what, n) in by {
        eprintln!("{n:5} {what}");
    }
}
