//! The checker's part (b): items, uses and declarations. One case per code, with its place, in the style of
//! tests/parse.rs.

use luma_validate::names::Types;
use lumascript::check::{self, Library, ModFacts, OptionKind, OptionSpec, Source};
use lumascript::diag::{code, Arg};
use lumascript::game::Game;
use std::collections::HashMap;

const GAME: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 8, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "LiveActor", "size": 200, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "struct", "name": "Mario::MovementStates", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "enum", "name": "MarioStatus", "underlying": "int", "scoped": false, "values": [{"name": "MarioStatus_Swim", "value": 1}]}
  ],
  "globals": [],
  "left_out": [{"kind": "record", "name": "Gone", "file": "a.hpp", "line": 1, "reason": "no size (incomplete or dependent)"}]
}"#;

fn game() -> Game {
    Game::new(HashMap::new(), Some(Types::from_json(GAME.as_bytes()).unwrap()))
}

fn source(path: &str, text: &str) -> Source {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "{path} does not parse: {diags:?}");
    Source { path: path.into(), module }
}

/// Each diagnostic as (module, code, line, column, args as text), in order.
type Said = Vec<(String, u16, u32, u32, Vec<(&'static str, String)>)>;

fn said(modules: &[Source], libraries: &[Library], needs: &[&str], options: &[OptionSpec], c_headers: &[&str]) -> Said {
    let game = game();
    let needs: Vec<String> = needs.iter().map(|s| s.to_string()).collect();
    let c_headers: Vec<String> = c_headers.iter().map(|s| s.to_string()).collect();
    let facts = ModFacts { needs: &needs, options, c_headers: &c_headers, game: &game, content: None };
    let checked = check::check(modules, libraries, facts);
    checked
        .diags
        .into_iter()
        .map(|(m, d)| {
            let args = d.args.iter().map(|(n, a)| (*n, match a {
                Arg::Text(t) => t.clone(),
                Arg::Phrase(p) => format!("P:{p}"),
            })).collect();
            (m, d.code, d.at.line, d.at.col, args)
        })
        .collect()
}

fn one(text: &str) -> Said {
    said(&[source("main", text)], &[], &[], &[], &[])
}

fn a(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(n, v)| (*n, v.to_string())).collect()
}

#[test]
fn a_name_declared_twice_is_said_at_the_second() {
    assert_eq!(one("fn a() { }\nlet a = 1\n"), vec![("main".into(), code::NAME_TWICE, 2, 5, a(&[("name", "a"), ("line", "1")]))]);
    // Fields, cases, parameters and states each in their own scope.
    let text = "struct S {\n    x: int\n    x: f32\n}\nenum E { A, B, A }\nfn f(p: int, p: int) { }\n";
    let got: Vec<(u16, u32, u32)> = one(text).iter().map(|d| (d.1, d.2, d.3)).collect();
    assert_eq!(got, vec![(code::NAME_TWICE, 3, 5), (code::NAME_TWICE, 5, 16), (code::NAME_TWICE, 6, 14)]);
    // A struct's function may not reuse a field's name.
    assert_eq!(one("struct S {\n    x: int\n    fn x(self) { }\n}\n")[0].1, code::NAME_TWICE);
}

#[test]
fn uses_name_modules_that_exist_and_are_needed() {
    let main = source("main", "use ./ball\nuse ./gone\nuse kit.hud\nuse svc.net\nuse c \"helpers.h\"\nuse c \"other.h\"\n");
    let ball = source("ball", "pub fn roll() { }\n");
    let got = said(&[main, ball], &[], &[], &[], &["helpers.h"]);
    assert_eq!(
        got,
        vec![
            ("main".into(), code::NO_MODULE, 2, 7, a(&[("found", "./gone")])),
            ("main".into(), code::NOT_NEEDED, 3, 5, a(&[("found", "kit.hud"), ("need", "lumakit")])),
            ("main".into(), code::NOT_DESCRIBED, 4, 5, a(&[("found", "svc.net"), ("what", "P:Script.What.ServiceFaces")])),
            ("main".into(), code::NO_C_HEADER, 6, 7, a(&[("found", "\"other.h\"")])),
        ]
    );
    assert!(lumascript::diag::is_note(code::NOT_DESCRIBED) && !lumascript::diag::is_note(code::NO_MODULE));
}

#[test]
fn names_taken_from_a_library_are_its_pub_ones() {
    let kit = Library { id: "lumakit".into(), modules: vec![source("hud", "pub struct Meter { max: int }\nstruct Inner { }\n")] };
    let main = source("main", "use kit.hud.{Meter, Inner, Nothing}\nuse lumakit.gone\nlet m: Meter? = none\nlet h: hud.Meter? = none\n");
    let got = said(&[main], &[kit], &["lumakit"], &[], &[]);
    assert_eq!(
        got,
        vec![
            ("main".into(), code::NAME_NOT_PUB, 1, 21, a(&[("name", "Inner"), ("module", "kit.hud")])),
            ("main".into(), code::NO_NAME_IN_MODULE, 1, 28, a(&[("name", "Nothing"), ("module", "kit.hud")])),
            ("main".into(), code::NO_MODULE, 2, 5, a(&[("found", "lumakit.gone")])),
            // `hud` is not bound: names were taken from it, without `as`.
            ("main".into(), code::UNKNOWN_TYPE, 4, 8, a(&[("found", "hud.Meter")])),
        ]
    );
}

#[test]
fn a_cycle_of_modules_is_said_once_with_its_chain() {
    let modules = [source("main", "use ./a\n"), source("a", "use ./b\n"), source("b", "use ./main\n")];
    let got = said(&modules, &[], &[], &[], &[]);
    assert_eq!(got, vec![("b".into(), code::MODULE_CYCLE, 1, 1, a(&[("chain", "main -> a -> b -> main")]))]);
    // C5: a library (a mod with a `pub` item) may: its modules' functions are declared before any body.
    let modules = [source("main", "use ./a\n"), source("a", "use ./b\n\npub fn f() {\n}\n"), source("b", "use ./main\n")];
    assert_eq!(said(&modules, &[], &[], &[], &[]), vec![]);
}

#[test]
fn written_types_resolve_or_are_said() {
    let text = "struct P { x: int }\nenum K { A, B }\nenum D { A(x: int) }\n\
                fn f(a: vec3, b: list<int>, c: map<K, P>, d: P?, e: set<D>, g: vec, h: list, i: int<f32>, j: json.Value) { }\n";
    let got = one(text);
    assert_eq!(
        got,
        vec![
            ("main".into(), code::MAP_KEY, 4, 57, a(&[("found", "D")])),
            ("main".into(), code::UNKNOWN_TYPE_NEAR, 4, 64, a(&[("found", "vec"), ("near", "`vec2`, `vec3`, `vec4`")])),
            ("main".into(), code::TYPE_ARG_COUNT, 4, 72, a(&[("found", "list"), ("expected", "1")])),
            ("main".into(), code::TYPE_ARG_COUNT, 4, 81, a(&[("found", "int"), ("expected", "0")])),
        ]
    );
}

#[test]
fn game_types_by_name_follow_the_self_description() {
    let text = "fn f(a: LiveActor, b: Mario::MovementStates, c: ref<LiveActor>, d: MarioStatus, e: s32, g: ref<int>, h: `Gone`, i: Nope) { }\n";
    let got = one(text);
    assert_eq!(
        got,
        vec![
            ("main".into(), code::REF_OF, 1, 96, a(&[("found", "int")])),
            ("main".into(), code::GAME_TYPE_NOT_CROSSING, 1, 105, a(&[("found", "`Gone`"), ("why", "P:Script.Why.LeftOut")])),
            ("main".into(), code::UNKNOWN_TYPE, 1, 116, a(&[("found", "Nope")])),
        ]
    );
}

#[test]
fn a_class_derives_from_a_game_class() {
    let text = "class Ball : LiveActor {\n    var age: int = 0\n    override fn control() { }\n    override fn init(iter) { }\n}\n\
                class Bad : Mario::MovementStates { }\n";
    let got = one(text);
    assert_eq!(
        got,
        vec![
            // This game has no function facts: one note per class for its overrides; an override's parameter types
            // may be left out.
            ("main".into(), code::NOT_DESCRIBED, 3, 17, a(&[("found", "control"), ("what", "P:Script.What.Virtuals")])),
            ("main".into(), code::BASE_NOT_GAME_CLASS, 6, 13, a(&[("found", "Mario::MovementStates")])),
        ]
    );
}

#[test]
fn options_match_luma_json() {
    let options = [
        OptionSpec { key: "lights".into(), kind: OptionKind::Toggle },
        OptionSpec { key: "count".into(), kind: OptionKind::Number { whole: true } },
        OptionSpec { key: "speed".into(), kind: OptionKind::Number { whole: false } },
    ];
    let main = source("main", "option lights: bool\noption count: f32\noption speed: f32\noption gone: int\n");
    let got = said(&[main], &[], &[], &options, &[]);
    assert_eq!(
        got,
        vec![
            ("main".into(), code::OPTION_TYPE, 2, 15, a(&[("found", "count"), ("expected", "int")])),
            ("main".into(), code::OPTION_NOT_IN_LUMA_JSON, 4, 8, a(&[("found", "gone")])),
        ]
    );
}

#[test]
fn states_run_for_a_class_of_the_mod_and_start_at_one_of_theirs() {
    let text = "class Ball : LiveActor { }\nstates Fine for Ball {\n    state A { }\n    start A\n}\n\
                states Odd for Nope {\n    state A { }\n    state A { }\n    start B\n}\n";
    let got: Vec<(u16, u32, u32)> = one(text).iter().map(|d| (d.1, d.2, d.3)).filter(|d| d.0 != code::NOT_DESCRIBED).collect();
    assert_eq!(got, vec![(code::STATES_FOR, 6, 16), (code::NAME_TWICE, 8, 11), (code::START_NOT_STATE, 9, 11)]);
}

#[test]
fn a_bare_svc_is_no_module_and_hooks_keep_their_own_types() {
    assert_eq!(one("use svc\n"), vec![("main".into(), code::NO_MODULE, 1, 5, a(&[("found", "svc")]))]);
    // A class's hooks: each written type resolved (the result type too), each hook's types under its own key.
    let text = "class Ball : LiveActor {\n    hook before MR::f(a: int) -> Nope { }\n    hook before MR::g(a: f32) { }\n}\n";
    let main = source("main", text);
    let game = game();
    let facts = ModFacts::of_game(&game);
    let checked = check::check(&[main], &[], facts);
    let codes: Vec<u16> = checked.diags.iter().map(|(_, d)| d.code).filter(|c| *c != code::NOT_DESCRIBED).collect();
    // This game has neither function: part (e) says each hook's name is unknown (S0047), around the type's S0034.
    assert_eq!(codes, vec![code::UNKNOWN_NAME, code::UNKNOWN_TYPE, code::UNKNOWN_NAME]);
    let mut keys: Vec<&str> = checked.types.keys().map(|(_, k)| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec![".hook0(a)", ".hook0->", ".hook1(a)"]);
}

#[test]
fn a_value_that_holds_itself_is_said_once_with_its_chain() {
    // Through a field, an optional, a tuple and an enum case; a list or a ref breaks it.
    let text = "struct Node {\n    next: Node?\n}\nstruct A {\n    b: (int, B)\n}\nenum B {\n    Leaf,\n    Branch(a: A)\n}\n\
                struct Fine {\n    kids: list<Fine>\n}\n";
    assert_eq!(
        one(text),
        vec![
            ("main".into(), code::VALUE_CYCLE, 2, 11, a(&[("chain", "Node -> Node")])),
            ("main".into(), code::VALUE_CYCLE, 9, 15, a(&[("chain", "A -> B -> A")])),
        ]
    );
}

#[test]
fn a_name_that_hides_a_game_type_gets_a_note_once() {
    let text = "struct LiveActor { x: int }\nfn f(a: LiveActor, b: LiveActor, c: `LiveActor`, d: s32) { }\n";
    assert_eq!(one(text), vec![("main".into(), code::NAME_HIDES_GAME, 2, 9, a(&[("found", "LiveActor")]))]);
    assert!(lumascript::diag::is_note(code::NAME_HIDES_GAME));
}

#[test]
fn a_key_option_is_input_action_by_its_type() {
    let options = [OptionSpec { key: "jump".into(), kind: OptionKind::Key }, OptionSpec { key: "dash".into(), kind: OptionKind::Key }];
    let main = source("main", "use svc.input\noption jump: input.Action\noption dash: int\n");
    let got: Vec<(u16, u32, u32)> = said(&[main], &[], &[], &options, &[]).iter().map(|d| (d.1, d.2, d.3)).collect();
    assert_eq!(got, vec![(code::NOT_DESCRIBED, 1, 5), (code::OPTION_TYPE, 3, 14)]);
    // Without its `use`, `input` names nothing.
    let main = source("main", "option jump: input.Action\n");
    let got: Vec<u16> = said(&[main], &[], &[], &options[..1], &[]).iter().map(|d| d.1).collect();
    assert_eq!(got, vec![code::UNKNOWN_TYPE]);
}

#[test]
fn every_code_gives_the_values_its_row_fills() {
    // The args each test above saw are the ones args_of names (the text table's check reads args_of).
    let all = [
        one("fn a() { }\nlet a = 1\n"),
        one("fn f(a: vec, b: list, c: map<vec3, int>, d: ref<int>, e: `Gone`, g: Nope) { }\nclass C : Mario::MovementStates { }\n"),
        one("struct LiveActor { n: LiveActor? }\nfn f(a: LiveActor) { }\n"),
    ];
    for d in all.iter().flatten() {
        let names: Vec<&str> = d.4.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, lumascript::diag::args_of(d.1), "S{:04}", d.1);
    }
}

#[test]
fn a_used_modules_enum_case_is_made_and_matched_by_its_module() {
    let shapes = source(
        "shapes",
        "pub enum Shape { Circle(r: f32), Rect { w: f32, h: f32 }, Dot }\nenum Hidden { A(x: int) }\npub fn area(s: Shape) -> f32 {\n    return 0.0\n}\n",
    );
    let main = source(
        "main",
        "use ./shapes\nfn f() {\n    let a = shapes.area(shapes.Shape.Circle(1.0))\n    let b = shapes.area(shapes.Shape.Rect { w: 2.0, h: 3.5 })\n    let c = shapes.area(shapes.Shape.Dot)\n    let d = shapes.Hidden.A(1)\n}\n",
    );
    let got = said(&[main, shapes], &[], &[], &[], &[]);
    let codes: Vec<(u16, u32, u32)> = got.iter().map(|d| (d.1, d.2, d.3)).collect();
    // Only the case of a type the module keeps to itself is refused, once, at its name.
    assert_eq!(codes, vec![(code::NAME_NOT_PUB, 6, 20)], "{got:?}");
}
