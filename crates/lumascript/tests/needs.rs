//! Part (g): what a game call needs, proved where it is (S0154, S0155; Nebula's docs/plan/M2-NEEDS-PROOF.md). Each
//! proof form proves, each place that proves nothing is said with the needs and what would prove them, and the
//! emitted C runs each new handler when its set holds.

use luma_validate::names::{FunctionFacts, Types};
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::{code, Arg};
use lumascript::game::Game;
use std::collections::HashMap;

const TYPES: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 16, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "LiveActor", "size": 200, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "alias", "name": "f32", "of": "float", "record": null}
  ],
  "globals": [],
  "left_out": []
}"#;

/// Rows: name, signature, kind, needs, null_when.
const ROWS: &[(&str, &str, &str, &str, &str)] = &[
    ("MR::free", "s32()", "free", "", ""),
    ("MR::getFile", "s32()", "free", "UserFile", ""),
    ("MR::getStageThing", "s32()", "free", "StageSwitchContainer scene", ""),
    ("MR::getPlayerNum", "s32()", "free", "MarioActor scene", ""),
    ("MR::getOdd", "s32()", "free", "GalaxyMapController", ""),
    ("MR::findActor", "LiveActor *()", "free", "", "NamePosHolder"),
    ("MR::useNamePos", "void()", "free", "NamePosHolder", ""),
    ("MR::useScene", "void()", "free", "scene", ""),
    ("MR::twin", "void(s32)", "free", "scene", ""),
    ("MR::twin", "void(f32)", "free", "MarioActor scene", ""),
    ("LiveActor::control", "void()", "method", "scene", ""),
    ("MR::makeOdd", "void()", "free", "", ""),
];

/// Rows that make something: name, what.
const MAKES: &[(&str, &str)] = &[("MR::makeOdd", "GalaxyMapController")];

const SETS: &str = r#"[
  {"name": "system", "holds": "the game system's files are read", "needs": {"UserFile": "w"}},
  {"name": "stage", "holds": "a game scene is made", "needs": {"UserFile": "w", "scene": "w", "StageSwitchContainer": "w"}},
  {"name": "player", "holds": "the player is there", "needs": {"UserFile": "w", "scene": "w", "StageSwitchContainer": "w", "MarioActor": "w"}}
]"#;

fn list(words: &str) -> String {
    let w: Vec<String> = words.split_whitespace().map(|n| format!("\"{n}\"")).collect();
    format!("[{}]", w.join(", "))
}

fn game() -> Game {
    let mut functions: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (name, sig, ..) in ROWS {
        functions.entry(name.to_string()).or_default().push((sig.to_string(), false));
    }
    let mut game = Game::new(functions, Some(Types::from_json(TYPES.as_bytes()).unwrap_or_else(|e| panic!("{e}"))));
    let rows: Vec<String> = ROWS
        .iter()
        .map(|(name, sig, kind, needs, null_when)| {
            let owner = if *kind == "method" { "\"LiveActor\"" } else { "null" };
            let is_virtual = *kind == "method";
            let marks = if sig.contains("()") { "" } else { "null" };
            let makes = MAKES.iter().find(|(n, _)| n == name).map_or("", |(_, m)| m);
            format!(
                r#"{{"name": "{name}", "signature": "{sig}", "kind": "{kind}", "owner": {owner}, "virtual": {is_virtual}, "pure": false, "const": false, "overrides": [], "marks": [{marks}], "needs": {}, "null_when": {}, "makes": {}}}"#,
                list(needs),
                list(null_when),
                list(makes)
            )
        })
        .collect();
    let json = format!(r#"{{"functions": [{}], "guarantees": {SETS}}}"#, rows.join(", "));
    game.facts = Some(FunctionFacts::from_json(json.as_bytes()).unwrap_or_else(|e| panic!("{e}")));
    game
}

/// Each S0154/S0155 said: (code, line, found, needs, how).
fn needs_said(text: &str) -> Vec<(u16, u32, String, String, String)> {
    let g = game();
    let facts = ModFacts::of_game(&g);
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let checked = check::check(&[Source { path: "main".into(), module }, check::std_source()], &[], facts);
    let arg = |d: &lumascript::diag::Diagnostic, n: &str| {
        d.args.iter().find(|(k, _)| *k == n).map_or(String::new(), |(_, a)| match a {
            Arg::Text(t) => t.clone(),
            Arg::Phrase(p) => (*p).to_string(),
            Arg::Phrased(p, v) => format!("{p} {v:?}"),
        })
    };
    let other: Vec<_> = checked.diags.iter().filter(|(_, d)| ![code::NEEDS_NOT_PROVED, code::CAN_WHAT, code::HOLDS_WHAT].contains(&d.code)).map(|(_, d)| (d.code, d.at.line)).collect();
    assert!(other.is_empty(), "other codes said: {other:?}\n{text}");
    checked
        .diags
        .iter()
        .map(|(_, d)| (d.code, d.at.line, arg(d, "found"), arg(d, "needs"), arg(d, "how")))
        .collect()
}

fn refused(line: u32, found: &str, needs: &str, how: &str) -> (u16, u32, String, String, String) {
    (code::NEEDS_NOT_PROVED, line, found.into(), needs.into(), format!("Script.Needs.{how}"))
}

#[test]
fn a_call_with_no_needs_is_never_said() {
    assert_eq!(needs_said("on frame {\n    log MR::free()\n}\n"), vec![]);
}

#[test]
fn on_frame_and_on_start_prove_nothing() {
    assert_eq!(needs_said("on frame {\n    log MR::getPlayerNum()\n}\n"), vec![refused(2, "MR::getPlayerNum", "MarioActor, scene", "Player")]);
    assert_eq!(needs_said("on start {\n    log MR::getStageThing()\n}\n"), vec![refused(2, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]);
    assert_eq!(needs_said("on start {\n    log MR::getOdd()\n}\n"), vec![refused(2, "MR::getOdd", "GalaxyMapController", "Can")]);
    // A module value is made at load.
    assert_eq!(needs_said("let n = MR::getFile()\n"), vec![refused(1, "MR::getFile", "UserFile", "Stage")]);
}

#[test]
fn each_handler_proves_its_set() {
    assert_eq!(needs_said("on play {\n    log MR::getPlayerNum()\n    log MR::getStageThing()\n    log MR::getFile()\n}\n"), vec![]);
    assert_eq!(needs_said("on player ready {\n    log MR::getPlayerNum()\n}\n"), vec![]);
    assert_eq!(needs_said("on stage ready {\n    log MR::getStageThing()\n    log MR::getPlayerNum()\n}\n"), vec![refused(3, "MR::getPlayerNum", "MarioActor", "Player")]);
    assert_eq!(needs_said("on stage \"RedBlueExGalaxy\" {\n    log MR::getStageThing()\n}\n"), vec![]);
    assert_eq!(needs_said("on stage ended {\n    log MR::getFile()\n    log MR::getStageThing()\n}\n"), vec![refused(3, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]);
}

#[test]
fn a_wait_ends_the_proof() {
    assert_eq!(needs_said("on stage ready {\n    log MR::getStageThing()\n    wait 1 frames\n    log MR::getStageThing()\n}\n"), vec![refused(4, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]);
    // A wait in one arm reaches the join.
    assert_eq!(
        needs_said("on stage ready {\n    if MR::free() > 0 {\n        wait 1 frames\n    }\n    log MR::getStageThing()\n}\n"),
        vec![refused(5, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]
    );
    // The next time round a loop that waits, what was proved before it does not stand; what is proved in it does.
    assert_eq!(
        needs_said("on stage ready {\n    while true {\n        log MR::getStageThing()\n        wait 1 frames\n    }\n}\n"),
        vec![refused(3, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]
    );
    assert_eq!(needs_said("on stage ready {\n    while true {\n        if can(MR::getStageThing) {\n            log MR::getStageThing()\n        }\n        wait 1 frames\n    }\n}\n"), vec![]);
    // A loop that does not wait keeps it.
    assert_eq!(needs_said("on stage ready {\n    for i in 0..3 {\n        log MR::getStageThing()\n    }\n}\n"), vec![]);
}

#[test]
fn can_proves_its_function() {
    assert_eq!(needs_said("on frame {\n    if can(MR::getOdd) {\n        log MR::getOdd()\n    }\n    log MR::getOdd()\n}\n"), vec![refused(5, "MR::getOdd", "GalaxyMapController", "Can")]);
    assert_eq!(needs_said("on frame {\n    if not can(MR::getOdd) {\n        return\n    }\n    log MR::getOdd()\n}\n"), vec![]);
    assert_eq!(needs_said("on frame {\n    guard can(MR::getOdd) else { return }\n    log MR::getOdd()\n}\n"), vec![]);
    assert_eq!(needs_said("on frame {\n    if can(MR::getOdd) and can(MR::getFile) {\n        log MR::getOdd() + MR::getFile()\n    }\n}\n"), vec![]);
    assert_eq!(needs_said("on frame {\n    if not can(MR::getOdd) {\n        log 0\n    } else {\n        log MR::getOdd()\n    }\n}\n"), vec![]);
    // `or` proves neither.
    assert_eq!(needs_said("on frame {\n    if can(MR::getOdd) or can(MR::getFile) {\n        log MR::getFile()\n    }\n}\n"), vec![refused(3, "MR::getFile", "UserFile", "Stage")]);
    // Every row of the name, or the one written.
    assert_eq!(needs_said("on frame {\n    if can(MR::twin) {\n        MR::twin(1)\n        MR::twin(1.5)\n    }\n}\n"), vec![]);
    assert_eq!(needs_said("on frame {\n    if can(`MR::twin(s32)`) {\n        MR::twin(1)\n        MR::twin(1.5)\n    }\n}\n"), vec![refused(4, "MR::twin", "MarioActor", "Player")]);
}

#[test]
fn can_takes_a_game_function_name() {
    let said: Vec<u16> = needs_said("on frame {\n    log can(1)\n    log can(MR::free, MR::free)\n    log can(`MR::twin(bool)`)\n}\n").into_iter().map(|d| d.0).collect();
    assert_eq!(said, vec![code::CAN_WHAT, code::CAN_WHAT, code::CAN_WHAT]);
}

#[test]
fn holds_proves_its_set() {
    assert_eq!(needs_said("on frame {\n    if holds(\"player\") {\n        log MR::getPlayerNum() + MR::getStageThing()\n    }\n    log MR::getStageThing()\n}\n"), vec![refused(5, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]);
    assert_eq!(needs_said("on frame {\n    guard holds(\"stage\") else { return }\n    log MR::getStageThing()\n    log MR::getPlayerNum()\n}\n"), vec![refused(4, "MR::getPlayerNum", "MarioActor", "Player")]);
    // A set's needs only: an object no set names is not among them.
    assert_eq!(needs_said("on frame {\n    if holds(\"player\") {\n        log MR::getOdd()\n    }\n}\n"), vec![refused(3, "MR::getOdd", "GalaxyMapController", "Can")]);
    // A wait ends it, as any proof.
    assert_eq!(needs_said("task t() {\n    if holds(\"stage\") {\n        wait 1 frames\n        log MR::getStageThing()\n    }\n}\n"), vec![refused(4, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]);
}

#[test]
fn the_left_of_and_or_proves_in_its_right() {
    assert_eq!(needs_said("on frame {
    log holds(\"player\") and MR::getPlayerNum() > 0
    log not holds(\"player\") or MR::getPlayerNum() > 0
}
"), vec![]);
    assert_eq!(needs_said("on frame {
    log holds(\"player\") or MR::getPlayerNum() > 0
}
"), vec![refused(2, "MR::getPlayerNum", "MarioActor, scene", "Player")]);
    // Not past the expression.
    assert_eq!(needs_said("on frame {
    let ok = holds(\"player\") and MR::getPlayerNum() > 0
    log MR::getPlayerNum()
}
"), vec![refused(3, "MR::getPlayerNum", "MarioActor, scene", "Player")]);
}

#[test]
fn holds_takes_a_set_s_name() {
    let said: Vec<u16> = needs_said("on frame {\n    log holds(\"nothing\")\n    log holds(1)\n    log holds(\"stage\", \"player\")\n    let s = \"stage\"\n    log s\n    log holds(s)\n    log holds(\"{s}\")\n}\n").into_iter().map(|d| d.0).collect();
    assert_eq!(said, vec![code::HOLDS_WHAT; 5]);
}

#[test]
fn a_call_proves_what_it_makes() {
    assert_eq!(needs_said("on frame {\n    MR::makeOdd()\n    log MR::getOdd()\n}\n"), vec![]);
    assert_eq!(needs_said("on frame {\n    log MR::getOdd()\n    MR::makeOdd()\n}\n"), vec![refused(2, "MR::getOdd", "GalaxyMapController", "Can")]);
    // In its own branch only, and until a wait.
    assert_eq!(
        needs_said("on frame {\n    if MR::free() > 0 {\n        MR::makeOdd()\n    }\n    log MR::getOdd()\n}\n"),
        vec![refused(5, "MR::getOdd", "GalaxyMapController", "Can")]
    );
    assert_eq!(needs_said("task t() {\n    MR::makeOdd()\n    wait 1 frames\n    log MR::getOdd()\n}\n"), vec![refused(4, "MR::getOdd", "GalaxyMapController", "Can")]);
}

#[test]
fn if_let_proves_what_the_result_is_null_without() {
    assert_eq!(needs_said("on frame {\n    if let a = MR::findActor() {\n        MR::useNamePos()\n    }\n    MR::useNamePos()\n}\n"), vec![refused(5, "MR::useNamePos", "NamePosHolder", "Can")]);
}

#[test]
fn a_hook_and_an_override_prove_what_their_function_needs() {
    assert_eq!(needs_said("hook before LiveActor::control() {\n    MR::useScene()\n    log MR::getStageThing()\n}\n"), vec![refused(3, "MR::getStageThing", "StageSwitchContainer", "Stage")]);
    assert_eq!(needs_said("class Mine : LiveActor {\n    override fn control() {\n        MR::useScene()\n    }\n}\n"), vec![]);
    // An object of a mod's class lives only while the set `stage`'s objects do; not the player.
    assert_eq!(
        needs_said("class Mine : LiveActor {\n    override fn control() {\n        log MR::getStageThing()\n        log MR::getPlayerNum()\n    }\n}\n"),
        vec![refused(4, "MR::getPlayerNum", "MarioActor", "Player")]
    );
}

#[test]
fn a_class_s_machine_proves_what_its_control_and_the_stage_need_through_its_waits() {
    // Its states run inside the object's control, each part and each resumption after a wait: the set `stage` and
    // the control row's needs stand all through, waits too; not the player's.
    let machine = "class Mine : LiveActor { }\n\nstates Phases for Mine {\n    state Fly {\n        enter { MR::useScene(); log MR::getStageThing() }\n        update { log MR::getStageThing() }\n    }\n    state Burst {\n        enter { log MR::getStageThing() }\n        wait 30 frames\n        log MR::getStageThing()\n        log MR::getPlayerNum()\n    }\n    start Fly\n}\n";
    assert_eq!(needs_said(machine), vec![refused(12, "MR::getPlayerNum", "MarioActor", "Player")]);
}

#[test]
fn a_mod_function_s_needs_are_its_callers() {
    let helper = "fn helper() -> s32 {\n    return MR::getStageThing()\n}\n";
    assert_eq!(needs_said(&format!("{helper}on frame {{\n    log helper()\n}}\n")), vec![refused(5, "helper", "StageSwitchContainer, scene", "Stage")]);
    assert_eq!(needs_said(&format!("{helper}on stage ready {{\n    log helper()\n}}\n")), vec![]);
    // Through another, and what one proves itself is not its callers'.
    let two = "fn a() -> s32 {\n    return b() + c()\n}\nfn b() -> s32 {\n    return MR::getStageThing()\n}\nfn c() -> s32 {\n    if can(MR::getOdd) {\n        return MR::getOdd()\n    }\n    return 0\n}\n";
    assert_eq!(needs_said(&format!("{two}on frame {{\n    log a()\n}}\n")), vec![refused(14, "a", "StageSwitchContainer, scene", "Stage")]);
    // Calling itself.
    assert_eq!(needs_said("fn r(n: s32) -> s32 {\n    if n == 0 { return MR::getFile() }\n    return r(n - 1)\n}\non frame {\n    log r(3)\n}\n"), vec![refused(6, "r", "UserFile", "Stage")]);
    // A struct's own function.
    let s = "struct S {\n    n: s32 = 0\n    fn get(self) -> s32 { return MR::getStageThing() }\n}\n";
    assert_eq!(needs_said(&format!("{s}on frame {{\n    log S().get()\n}}\n")), vec![refused(6, "S.get", "StageSwitchContainer, scene", "Stage")]);
    assert_eq!(needs_said(&format!("{s}on play {{\n    log S().get()\n}}\n")), vec![]);
}

#[test]
fn what_is_given_away_proves_its_own() {
    // A function named as a value, a closure that runs later, a task, an export.
    assert_eq!(
        needs_said("fn cb() -> s32 {\n    return MR::getStageThing()\n}\non stage ready {\n    let f = cb\n    log cb()\n}\n"),
        vec![refused(2, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]
    );
    assert_eq!(
        needs_said("on stage ready {\n    let f = fn() -> s32 { return MR::getStageThing() }\n}\n"),
        vec![refused(2, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]
    );
    assert_eq!(needs_said("task t() {\n    log MR::getStageThing()\n}\n"), vec![refused(2, "MR::getStageThing", "StageSwitchContainer, scene", "Stage")]);
    // A closure a built-in runs at once keeps what is proved around it.
    assert_eq!(needs_said("on stage ready {\n    let ok = [1, 2].any(fn(i: int) -> bool { return MR::getStageThing() > i })\n}\n"), vec![]);
}

#[test]
fn a_game_without_needs_says_nothing() {
    let mut g = game();
    g.facts = None;
    let facts = ModFacts::of_game(&g);
    let (module, _) = lumascript::parse_module("on frame {\n    log MR::getPlayerNum()\n}\n");
    let checked = check::check(&[Source { path: "main".into(), module }, check::std_source()], &[], facts);
    assert!(checked.diags.iter().all(|(_, d)| d.code != code::NEEDS_NOT_PROVED), "{:?}", checked.diags);
}
