//! A mod of C alone (no LumaScript; its C declares its entry with LUMA_MOD) is built by `lumalink build` as it is: a
//! library for each platform asked, recorded in luma.json, with nothing generated.
//!
//! Needs LLVM 22.1.6 (LUMALINK_CLANG, else `build::default_clang`) and the SDK: LUMA_SDK, else the Nebula tree beside
//! this workspace (`nebula/native/include`). Without either the test says which is missing and checks nothing; with
//! LUMA_SDK given (the gate gives it), a missing LLVM fails it: a run asked to build never passes by building nothing
//! (N-4062).

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

/// LLVM 22.1.6's clang (`build::default_clang`). Without it: None, said, when no LUMA_SDK is given; a failure when
/// one is (the gate's run, which must build).
fn clang() -> Option<PathBuf> {
    let clang = build::default_clang();
    let found = build::clang_version(&clang).ok();
    if found.as_deref() == Some(build::LLVM) {
        return Some(clang);
    }
    let why = format!("no LLVM {} at {} (found {:?})", build::LLVM, clang.display(), found);
    assert!(std::env::var_os("LUMA_SDK").is_none(), "LUMA_SDK is given, so this test must build: {why}");
    eprintln!("skipped: {why}");
    None
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
    let Some(clang) = clang() else { return };
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
    let Some(clang) = clang() else { return };
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

/// An SDK whose folder has a comma in its path links every ELF platform (the linker script's path goes to the linker
/// whole, M-149), and the arm64 ones carry their atomics inline, importing no `__aarch64_` helper (M-142).
#[test]
fn an_sdk_path_with_a_comma_links_and_arm64_atomics_are_inline() {
    let Some(sdk) = sdk() else {
        eprintln!("skipped: no SDK (LUMA_SDK, or nebula/native/include beside this workspace)");
        return;
    };
    let Some(clang) = clang() else { return };
    let dir = tempfile::tempdir().unwrap();
    let copied = dir.path().join("sd,k");
    copy_folder(&sdk, &copied);
    let folder = dir.path().join("atomic");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    let (major, minor) = luma_validate::FORMAT;
    std::fs::write(
        folder.join("luma.json"),
        format!("{{
  \"format\": \"{major}.{minor}\",
  \"id\": \"test.atomic\",
  \"version\": \"1.0.0\",
  \"name\": \"Atomic\",
  \"summary\": \"Atomics.\",
  \"authors\": [\"keepvaibin\"],
  \"game\": {{ \"id\": \"RMG\" }}
}}
"),
    )
    .unwrap();
    let source = "#include <luma.h>

static int counter;

static int32_t start(const LumaCore* core, const LumaMod* mod) {
    int expected = 0;
    __atomic_compare_exchange_n(&counter, &expected, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
    core->log(mod, __atomic_fetch_add(&counter, 1, __ATOMIC_SEQ_CST) ? \"one\" : \"zero\");
    return 0;
}

LUMA_MOD_NO_IMPORTS(start, 0);
";
    std::fs::write(folder.join("source/atomic.c"), source).unwrap();
    let elf: Vec<String> = ["linux-x64", "linux-arm64", "android-arm64", "android-x64"].map(String::from).to_vec();
    let built = build::build(&folder, &Game::default(), &copied, &clang, &dir.path().join("work"), &elf).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(built.libraries.iter().map(|(p, _, _)| *p).collect::<Vec<_>>(), ["linux-x64", "linux-arm64", "android-arm64", "android-x64"]);
    for (platform, library, _) in &built.libraries {
        let bytes = std::fs::read(library).unwrap();
        assert!(!bytes.windows(b"__aarch64_".len()).any(|w| w == b"__aarch64_"), "{platform} imports an outline atomic helper");
    }
}

fn copy_folder(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(e.file_name());
        if e.path().is_dir() {
            copy_folder(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
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
