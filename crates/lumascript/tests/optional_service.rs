//! `use optional svc.<name>` (language 12; consent 1.4): the service's kind 5 row carries LUMA_OPTIONAL, so the mod
//! loads when the service is not given (the player declined its permission) and the slot stays null. The alias is
//! `none` then: `x == none` and `x != none` test the slot, and a call needs such a test before it (`if x != none`, a
//! `guard`, or an `if x == none` whose block leaves), else it is S0067 as for any value that may be none. One plain
//! `use` of the same service in any module makes it needed again.

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::emit;
use lumascript::game::Game;
use std::collections::HashMap;

const GAME: &str = r#"{"types": [{"kind": "alias", "name": "s32", "of": "int", "record": null}], "globals": [], "left_out": []}"#;

const CALL: &str = "if let s = ui.open(\"Hud\", ui.Layer.Pause) {\n        ui.close(s)\n    }";

/// The checker's error codes and lines, and, with none, the mod's imports and main C, for modules `texts` (the first
/// `main`, the others `m1`, `m2`...).
fn build(texts: &[String]) -> (Vec<(u16, u32)>, String, String) {
    let dir = format!("{}/tests/data/svc", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_ui.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let sources: Vec<Source> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let (module, d) = lumascript::parse_module(t);
            assert!(d.is_empty(), "{d:?}");
            Source { path: if i == 0 { "main".into() } else { format!("m{i}") }, module }
        })
        .collect();
    let mut game = Game::new(HashMap::new(), Some(Types::from_json(GAME.as_bytes()).unwrap()));
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<(u16, u32)> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).map(|(_, d)| (d.code, d.at.line)).collect();
        if !errors.is_empty() {
            return (errors, String::new(), String::new());
        }
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let file = |name: &str| out.files.iter().find(|(p, _)| p == name).map(|(_, c)| c.clone()).unwrap();
        (errors, file("lum/imports.c"), file("lum/main.c"))
    })
    .unwrap()
}

#[test]
fn an_optional_service_use_imports_the_service_optional() {
    let (e, only, _) = build(&[format!("use optional svc.ui\n\non start {{\n    if ui != none {{\n        {CALL}\n    }}\n}}\n")]);
    assert!(e.is_empty(), "{e:?}");
    assert!(only.contains("{LUMA_IMPORT_SERVICE, LUMA_OPTIONAL, \"ui\", \"1.0\", &lm_svc_0, 0, 0, 0}"), "{only}");
    assert!(!only.contains("LUMA_SERVICE("), "{only}");
    let (e, plain, _) = build(&[format!("use svc.ui\n\non start {{\n    {CALL}\n}}\n")]);
    assert!(e.is_empty(), "{e:?}");
    assert!(plain.contains("LUMA_SERVICE(lm_svc_0, \"ui\", \"1.0\")") && !plain.contains("LUMA_OPTIONAL"), "{plain}");
    // Optional in one module, needed in another: needed.
    let (e, both, _) = build(&[
        format!("use optional svc.ui\n\non start {{\n    guard ui != none else {{ return }}\n    {CALL}\n}}\n"),
        "use svc.ui\n\npub fn shown() -> bool {\n    return ui.Layer.Pause == ui.Layer.Pause\n}\n".to_string(),
    ]);
    assert!(e.is_empty(), "{e:?}");
    assert!(both.contains("LUMA_SERVICE(lm_svc_0, \"ui\", \"1.0\")") && !both.contains("LUMA_OPTIONAL"), "{both}");
}

/// The three tests that show the service given, each written as the slot's null test, and calls after them.
#[test]
fn a_test_against_none_is_the_slots_null_test_and_lets_the_calls_after_it() {
    let text = format!(
        "use optional svc.ui\n\nvar seen = false\n\non start {{\n    seen = ui == none\n    if ui != none {{\n        {CALL}\n    }}\n}}\n\n\
         fn early() {{\n    if ui == none {{\n        return\n    }}\n    {CALL}\n}}\n\n\
         fn guarded() {{\n    guard ui != none, seen else {{ return }}\n    {CALL}\n}}\n\n\
         fn both() {{\n    if not (ui == none) and seen {{\n        {CALL}\n    }}\n}}\n\n\
         fn otherwise() {{\n    if ui == none {{\n        log \"none\"\n    }} else {{\n        {CALL}\n    }}\n}}\n\n\
         fn later_arm() {{\n    if ui == none {{\n        log \"none\"\n    }} else if seen {{\n        {CALL}\n    }}\n}}\n"
    );
    let (e, _, main) = build(&[text]);
    assert!(e.is_empty(), "{e:?}");
    assert!(main.contains("((uint8_t)(lm_svc_0 == 0))") && main.contains("((uint8_t)(lm_svc_0 != 0))"), "{main}");
}

/// A call no test covers: S0067 at the alias. Outside the `if` that tested it, in its `else`, after an `if x == none`
/// that does not leave, and in a condition before the test.
#[test]
fn a_call_on_an_optional_service_without_a_test_before_it_is_may_be_none() {
    let cases = [
        (format!("use optional svc.ui\n\non start {{\n    {CALL}\n}}\n"), 4),
        (format!("use optional svc.ui\n\non start {{\n    if ui != none {{\n        log \"a\"\n    }}\n    {CALL}\n}}\n"), 7),
        (format!("use optional svc.ui\n\non start {{\n    if ui != none {{\n        log \"a\"\n    }} else {{\n        {CALL}\n    }}\n}}\n"), 7),
        (format!("use optional svc.ui\n\non start {{\n    if ui == none {{\n        log \"a\"\n    }}\n    {CALL}\n}}\n"), 7),
        ("use optional svc.ui\n\non start {\n    if ui.open(\"Hud\", ui.Layer.Pause) != none and ui != none {\n        log \"a\"\n    }\n}\n".to_string(), 4),
    ];
    for (text, line) in cases {
        let (e, _, _) = build(&[text.clone()]);
        assert_eq!(e, vec![(67, line)], "{text}");
    }
}
