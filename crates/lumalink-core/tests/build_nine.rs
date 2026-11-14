//! One mod built by `lumalink build` for all nine platforms of luma-format 6, packed, and each library in
//! the .luma read as its platform's loader will read it (check_luma_platforms.py: form,
//! luma_mod exported, no import from the system, no executable stack, constructors out of .init_array, 16 KiB pages on
//! Android, the Mach-O platform and its signature). Built twice: the same bytes.
//!
//! Needs LLVM 22.1.6 (LUMA_CLANG, else `build::default_clang`), the SDK (LUMA_SDK: luma.h and luma_ctors.ld) and
//! Python; the checker is LUMA_PLATFORM_CHECK (the path of check_luma_platforms.py). Without LUMA_SDK or
//! LUMA_PLATFORM_CHECK the test says which is missing and checks nothing.

use lumalink_core::{build, luma_file};
use lumascript::game::Game;
use std::path::{Path, PathBuf};

const SCRIPT: &str = "var seen = 0\n\non start {\n    var total = 0\n    var i = 0\n    while i < 100 {\n        total = total + i\n        i = i + 1\n    }\n    seen = total\n    log \"nine platforms: {seen}\"\n}\n";

fn manifest() -> String {
    let (major, minor) = luma_validate::FORMAT;
    format!(
        "{{\n  \"format\": \"{major}.{minor}\",\n  \"id\": \"test.nine\",\n  \"version\": \"1.0.0\",\n  \"name\": \"Nine\",\n  \"summary\": \"One mod for every platform.\",\n  \"authors\": [\"keepvaibin\"],\n  \"game\": {{ \"id\": \"RMG\" }}\n}}\n"
    )
}

fn libraries(folder: &Path) -> Vec<(String, Vec<u8>)> {
    build::TARGETS
        .iter()
        .map(|t| {
            let path = folder.join("code").join(t.platform).join(t.library);
            (t.platform.to_string(), std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        })
        .collect()
}

#[test]
#[ignore = "needs LLVM 22.1.6, the SDK (LUMA_SDK) and Python"]
fn one_mod_builds_for_all_nine_and_each_library_reads_as_its_loader_will() {
    let Some(sdk) = std::env::var_os("LUMA_SDK").map(PathBuf::from) else {
        eprintln!("skipped: LUMA_SDK is not set (the folder with luma.h and luma_ctors.ld)");
        return;
    };
    let Some(check) = std::env::var_os("LUMA_PLATFORM_CHECK").map(PathBuf::from) else {
        eprintln!("skipped: LUMA_PLATFORM_CHECK is not set (the path of check_luma_platforms.py)");
        return;
    };
    let clang = std::env::var_os("LUMA_CLANG").map(PathBuf::from).unwrap_or_else(build::default_clang);
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("nine");
    std::fs::create_dir_all(folder.join("source")).unwrap();
    std::fs::write(folder.join("luma.json"), manifest()).unwrap();
    std::fs::write(folder.join("source/main.lum"), SCRIPT).unwrap();
    let game = Game::default();

    let built = build::build(&folder, &game, &sdk, &clang, &dir.path().join("work1"), &[]).unwrap_or_else(|e| panic!("{e:?}"));
    let platforms: Vec<&str> = built.libraries.iter().map(|(p, _, _)| *p).collect();
    assert_eq!(platforms, luma_validate::PLATFORMS);
    assert_eq!(built.recorded, luma_validate::PLATFORMS, "each platform recorded in luma.json once");
    let first = libraries(&folder);
    // The same sources, another work folder: the same bytes (docs/modding/language/compiled-form.md, section 1.2), and nothing more to record.
    let again = build::build(&folder, &game, &sdk, &clang, &dir.path().join("work2"), &[]).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(again.recorded.is_empty());
    for ((platform, a), (_, b)) in first.iter().zip(libraries(&folder)) {
        assert!(*a == b, "{platform}: a second build gives other bytes");
    }

    let luma = dir.path().join("nine.luma");
    luma_file::write(&luma_file::folder_files(&folder).unwrap(), &luma).unwrap();
    let out = std::process::Command::new("python")
        .args(["-X", "utf8", "-I"])
        .arg(&check)
        .arg(&luma)
        .output()
        .unwrap_or_else(|e| panic!("python {}: {e}", check.display()));
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}{}", String::from_utf8_lossy(&out.stderr));
    // One line a platform (`<platform> OK ...`), each of luma-format's nine, then the checker's count.
    let ok: Vec<&str> = said.lines().filter_map(|l| l.split_whitespace().collect::<Vec<_>>().get(..2).filter(|w| w[1] == "OK").map(|w| w[0])).collect();
    let mut nine = luma_validate::PLATFORMS.to_vec();
    nine.sort_unstable();
    assert_eq!(ok, nine, "{said}");
    assert!(said.contains("9 of 9 needed platforms OK"), "{said}");

    // One platform asked for alone: only it is built, and an unknown one is refused by name before anything is.
    let one = build::build(&folder, &game, &sdk, &clang, &dir.path().join("work3"), &["ios-arm64".to_string()]).unwrap();
    assert_eq!(one.libraries.len(), 1);
    assert!(matches!(build::build(&folder, &game, &sdk, &clang, &dir.path().join("work4"), &["ps2".to_string()]), Err(build::BuildError::UnknownPlatform(p)) if p == "ps2"));
}
