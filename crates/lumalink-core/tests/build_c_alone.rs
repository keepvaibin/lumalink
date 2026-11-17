//! A mod of C alone (no LumaScript; its C declares its entry with LUMA_MOD) is built by `lumalink build` as it is: a
//! library for each platform asked, recorded in luma.json, with nothing generated.
//!
//! Needs LLVM 22.1.6 (LUMALINK_CLANG, else `build::default_clang`) and the SDK: LUMA_SDK, else the Nebula tree beside
//! this workspace (`nebula/native/include`). Without either the test says which is missing and checks nothing.

use lumalink_core::build;
use lumascript::game::Game;
use std::path::PathBuf;

const SOURCE: &str = r#"#include <luma.h>

static const LumaFunction* left;

static int32_t start(const LumaCore* core, const LumaMod* mod) {
    core->log(mod, "c alone: started");
    return 0;
}

static const LumaImport imports[] = {
    LUMA_FUNCTION(left, "MR::getPlayerLeft", "s32()"),
};

LUMA_MOD(imports, start, 0);
"#;

fn sdk() -> Option<PathBuf> {
    std::env::var_os("LUMA_SDK").map(PathBuf::from).or_else(|| {
        let beside = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../nebula/native/include");
        beside.join("luma.h").is_file().then_some(beside)
    })
}

#[test]
fn a_mod_of_c_alone_is_built_with_its_own_entry() {
    let Some(sdk) = sdk() else {
        eprintln!("skipped: no SDK (LUMA_SDK, or nebula/native/include beside this workspace)");
        return;
    };
    let Some(host) = build::HOST else {
        eprintln!("skipped: this system is none of the nine platforms");
        return;
    };
    let clang = build::default_clang();
    if build::clang_version(&clang).ok().as_deref() != Some(build::LLVM) {
        eprintln!("skipped: no LLVM {} at {}", build::LLVM, clang.display());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("alone");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    let (major, minor) = luma_validate::FORMAT;
    std::fs::write(
        folder.join("luma.json"),
        format!("{{\n  \"format\": \"{major}.{minor}\",\n  \"id\": \"test.alone\",\n  \"version\": \"1.0.0\",\n  \"name\": \"Alone\",\n  \"summary\": \"A mod of C alone.\",\n  \"authors\": [\"keepvaibin\"],\n  \"game\": {{ \"id\": \"RMG\" }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(folder.join("source/alone.c"), SOURCE).unwrap();

    let built = build::build(&folder, &Game::default(), &sdk, &clang, &dir.path().join("work"), &[host.to_string()]).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(built.libraries.iter().map(|(p, _, _)| *p).collect::<Vec<_>>(), [host]);
    assert_eq!(built.recorded, [host], "the platform recorded in luma.json");
    assert!(built.imports.is_empty(), "nothing generated, so no import of the generated code's");
    let library = std::fs::read(&built.libraries[0].1).unwrap();
    assert!(library.windows(b"luma_mod".len()).any(|w| w == b"luma_mod"), "the library exports luma_mod");
    // Nothing generated: the work folder holds no C of the compiler's.
    let generated = walk(&dir.path().join("work")).into_iter().filter(|p| p.extension().is_some_and(|e| e == "c")).count();
    assert_eq!(generated, 0);
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// An author's own C may have a frame of any size (luma.h gives the stack probe); the 4 KiB limit checks only the C the
/// compiler generates.
#[test]
fn an_authors_c_may_have_a_frame_over_4_kib() {
    let Some(sdk) = sdk() else {
        eprintln!("skipped: no SDK (LUMA_SDK, or nebula/native/include beside this workspace)");
        return;
    };
    let Some(host) = build::HOST else {
        eprintln!("skipped: this system is none of the nine platforms");
        return;
    };
    let clang = build::default_clang();
    if build::clang_version(&clang).ok().as_deref() != Some(build::LLVM) {
        eprintln!("skipped: no LLVM {} at {}", build::LLVM, clang.display());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("big");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    let (major, minor) = luma_validate::FORMAT;
    std::fs::write(
        folder.join("luma.json"),
        format!("{{\n  \"format\": \"{major}.{minor}\",\n  \"id\": \"test.big\",\n  \"version\": \"1.0.0\",\n  \"name\": \"Big\",\n  \"summary\": \"A frame of 64 KiB.\",\n  \"authors\": [\"keepvaibin\"],\n  \"game\": {{ \"id\": \"RMG\" }}\n}}\n"),
    )
    .unwrap();
    let source = "#include <luma.h>\n\nstatic int32_t start(const LumaCore* core, const LumaMod* mod) {\n    volatile unsigned char big[65536];\n    for (unsigned i = 0; i < sizeof(big); i++) big[i] = (unsigned char)i;\n    core->log(mod, big[1000] ? \"big\" : \"zero\");\n    return 0;\n}\n\nLUMA_MOD_NO_IMPORTS(start, 0);\n";
    std::fs::write(folder.join("source/big.c"), source).unwrap();
    let built = build::build(&folder, &Game::default(), &sdk, &clang, &dir.path().join("work"), &[host.to_string()]).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(built.recorded, [host]);
}

/// A mod with neither LumaScript nor C is refused, by name, before any compile.
#[test]
fn a_mod_with_no_code_is_refused() {
    let Some(sdk) = sdk() else {
        eprintln!("skipped: no SDK (LUMA_SDK, or nebula/native/include beside this workspace)");
        return;
    };
    let Some(host) = build::HOST else {
        eprintln!("skipped: this system is none of the nine platforms");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("empty");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    std::fs::write(folder.join("source/notes.h"), "/* a header alone is no code */\n").unwrap();
    let (major, minor) = luma_validate::FORMAT;
    std::fs::write(
        folder.join("luma.json"),
        format!("{{\n  \"format\": \"{major}.{minor}\",\n  \"id\": \"test.empty\",\n  \"version\": \"1.0.0\",\n  \"name\": \"Empty\",\n  \"summary\": \"No code.\",\n  \"authors\": [\"keepvaibin\"],\n  \"game\": {{ \"id\": \"RMG\" }}\n}}\n"),
    )
    .unwrap();
    let result = build::build(&folder, &Game::default(), &sdk, &build::default_clang(), &dir.path().join("work"), &[host.to_string()]);
    assert!(matches!(result, Err(build::BuildError::NoScripts(_))), "{result:?}");
}
