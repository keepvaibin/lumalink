//! A mod's floating point gives the same bits on every platform, as the game's does (Nebula native/CMakeLists.txt,
//! -ffp-contract=off, A-10): every multiply and add rounds once each, on every CPU. clang's default for C fuses a
//! multiply and an add of one expression into one instruction where the CPU has one (every arm64), so a mod moving
//! the player computed other bits on Android, Apple and Windows on Arm than on an x86-64 PC, and a replay or a step
//! hash taken with mods on parted between them.
//!
//! Each of the nine targets compiles, to assembly, a probe of the forms clang fuses (C as the back end writes it and
//! as an author writes it, C++ as an author writes it) and lumart.c (sin, cos, atan2, sqrt): no fused multiply-add
//! instruction. The same probe with -ffp-contract=fast shows the check sees one where the CPU has it.
//!
//! Needs clang (LUMA_CLANG, else `build::default_clang`) and the SDK (LUMA_SDK: luma.h); without LUMA_SDK it says so
//! and checks nothing.

use lumalink_core::build;
use std::path::{Path, PathBuf};

const PROBE: &str = r#"
float f_madd(float a, float b, float c) { return a * b + c; }
float f_msub(float a, float b, float c) { return a * b - c; }
float f_nmadd(float a, float b, float c) { return -(a * b) - c; }
double d_madd(double a, double b, double c) { return a * b + c; }
float f_dot(const float *a, const float *b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
void f_axpy(float *y, const float *x, float k, int n) {
    for (int i = 0; i < n; i++) y[i] = k * x[i] + y[i];
}
"#;

/// The first word of each instruction line that is a fused multiply-add: arm64 (scalar and vector) and x86 (FMA3, FMA4).
fn fused(asm: &str) -> Vec<String> {
    asm.lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_ascii_lowercase)
        .filter(|w| {
            matches!(w.as_str(), "fmadd" | "fmsub" | "fnmadd" | "fnmsub" | "fmla" | "fmls")
                || w.starts_with("vfmadd")
                || w.starts_with("vfmsub")
                || w.starts_with("vfnmadd")
                || w.starts_with("vfnmsub")
        })
        .collect()
}

fn assembly(clang: &Path, target: &build::Target, flags: &[&str], extra: &[&str], include: &[&Path], source: &Path) -> String {
    let mut c = std::process::Command::new(clang);
    c.arg(format!("--target={}", target.triple)).args(flags).args(target.code).args(extra);
    for i in include {
        c.arg("-I").arg(i);
    }
    let out = c.arg("-S").arg(source).arg("-o").arg("-").output().unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(
        out.status.success(),
        "{} {}: {}",
        target.platform,
        source.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn every_multiply_and_add_rounds_once_each_on_every_platform() {
    let Some(sdk) = std::env::var_os("LUMA_SDK").map(PathBuf::from) else {
        eprintln!("skipped: LUMA_SDK is not set (the folder with luma.h)");
        return;
    };
    let clang = std::env::var_os("LUMA_CLANG").map(PathBuf::from).unwrap_or_else(build::default_clang);
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lumascript/rt");
    let dir = tempfile::tempdir().unwrap();
    let probe_c = dir.path().join("probe.c");
    let probe_cc = dir.path().join("probe.cpp");
    std::fs::write(&probe_c, PROBE).unwrap();
    std::fs::write(&probe_cc, format!("extern \"C\" {{\n{PROBE}\n}}\n")).unwrap();
    let own: Vec<&str> = build::own_flags().collect();
    let own_cxx: Vec<&str> = own.iter().copied().filter(|f| !f.starts_with("-std=") && *f != "-Werror=implicit-function-declaration").chain(build::CXX_FLAGS.iter().copied()).collect();

    let lumart = rt.join("lumart.c");
    let runtime_include: [&Path; 2] = [rt.as_path(), sdk.as_path()];
    let mut wrong = Vec::new();
    let mut seen_fused = 0;
    for target in build::TARGETS.iter() {
        let cases: [(&str, &[&str], &Path, &[&Path]); 4] = [
            ("generated C", build::COMMON_FLAGS, probe_c.as_path(), &[]),
            ("an author's C", own.as_slice(), probe_c.as_path(), &[]),
            ("an author's C++", own_cxx.as_slice(), probe_cc.as_path(), &[]),
            ("lumart.c", build::COMMON_FLAGS, lumart.as_path(), &runtime_include),
        ];
        for (what, flags, source, include) in cases {
            let found = fused(&assembly(&clang, target, flags, &[], include, source));
            if !found.is_empty() {
                wrong.push(format!("{} {what}: {}", target.platform, found.join(", ")));
            }
        }
        // The check sees a fused instruction where the CPU has one (every arm64; x86-64 has none at its base level).
        let control = fused(&assembly(&clang, target, build::COMMON_FLAGS, &["-ffp-contract=fast"], &[], &probe_c));
        if target.triple.starts_with("aarch64") || target.triple.starts_with("arm64") {
            assert!(!control.is_empty(), "{}: -ffp-contract=fast gave no fused instruction to see", target.platform);
            seen_fused += 1;
        }
    }
    assert_eq!(seen_fused, 5, "the five arm64 targets each showed the check a fused instruction");
    assert!(wrong.is_empty(), "fused multiply-adds:\n{}", wrong.join("\n"));
}
