//! The checker's part (c): bodies. One case per code 47 to 99 with its place, against a small game, and a sweep of the
//! language's programs against a kernel build's real rows and types (ignored: it needs the build).

use luma_validate::names::{signature_params, FunctionFacts, Names, Types};
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::{code, Arg};
use lumascript::game::Game;
use std::collections::HashMap;
use std::path::Path;

fn source(path: &str, text: &str) -> Source {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "{path} does not parse: {diags:?}");
    Source { path: path.into(), module }
}

/// A kernel build's luma folder (`LUMA_ROWS=<build>/luma cargo test -p lumascript --test body -- --ignored
/// --nocapture`): every program of the corpus checked against the real game, never a panic; what is said is printed,
/// by code, for reading (programs use LumaKit and services, which part (f) and the libraries give).
#[test]
#[ignore]
fn the_programs_check_against_a_kernel_build() {
    let rows = std::env::var("LUMA_ROWS").expect("LUMA_ROWS names a kernel build's luma folder");
    let mut names = Names::from_rows_folder(Path::new(&rows), 1).unwrap_or_else(|e| panic!("{e}"));
    if names.types.is_none() {
        let path = std::env::var("LUMA_TYPES").expect("LUMA_TYPES names its luma_types.json");
        names.types = Some(Types::from_json(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{path}: {e}")));
        // The rows' facts beside it (docs/modding/language/types.md), when the build made them.
        let facts = Path::new(&path).with_file_name(luma_validate::names::FUNCTIONS_FILE);
        if names.function_facts.is_none() && facts.is_file() {
            names.function_facts = Some(FunctionFacts::from_json(&std::fs::read(&facts).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", facts.display())));
        }
    }
    let game = Game::from_names(&names);
    eprintln!("facts: {}", game.facts.as_ref().map_or(0, |f| f.rows.len()));
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/programs");
    let modes = std::fs::read_to_string(folder.join("modes.txt")).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "lum")).collect();
    files.sort();
    let only = std::env::var("LUMA_CODE").ok().and_then(|c| c.parse::<u16>().ok());
    let mut by_code: std::collections::BTreeMap<u16, usize> = Default::default();
    for f in &files {
        let name = f.file_name().unwrap().to_str().unwrap().to_string();
        if modes.lines().any(|m| m == name) {
            continue;
        }
        let src = std::fs::read_to_string(f).unwrap().replace("\r\n", "\n");
        let s = source(&name, &src);
        let facts = ModFacts::of_game(&game);
        let checked = check::check(&[s], &[], facts);
        for (_, d) in &checked.diags {
            *by_code.entry(d.code).or_default() += 1;
            if only.is_none_or(|c| c == d.code) && d.code >= 47 {
                let args: Vec<String> = d.args.iter().map(|(n, a)| match a {
                    Arg::Text(t) => format!("{n}={t}"),
                    Arg::Phrase(p) => format!("{n}=P:{p}"),
                }).collect();
                let line = src.lines().nth(d.at.line as usize - 1).unwrap_or("").trim();
                eprintln!("{name}:{}:{} {} {} | {line}", d.at.line, d.at.col, d.code, args.join(" "));
            }
        }
    }
    eprintln!("{by_code:?}");
}

/// A small game: two classes with a base, a plain struct, an enum, functions with overloads, a global and a constant.
const GAME: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 16, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "mName", "type": "const char *", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "class", "name": "LiveActor", "size": 200, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [{"name": "mName", "from": "NameObj", "offset": 8, "type": "const char *"}], "fields": [
      {"name": "mPosition", "type": "Vec3f", "offset": 64, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mFlags.isDead", "type": "bool : 1", "offset": 80, "count": 1, "extents": null, "access": "public", "bits": {"width": 1, "offset": 640}},
      {"name": "mCount", "type": "const s32", "offset": 84, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mBox", "type": "Box", "offset": 104, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mTable", "type": "s32", "offset": 88, "count": 4, "extents": null, "access": "public", "bits": null}]},
    {"kind": "class", "name": "Kuribo", "size": 300, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "LiveActor", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "class", "name": "Thing", "size": 8, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "struct", "name": "Vec3<f32>", "size": 12, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "x", "type": "f32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "y", "type": "f32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "z", "type": "f32", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Box", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "w", "type": "s32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "h", "type": "s32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Sensor", "size": 16, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": false, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "mRadius", "type": "f32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "alias", "name": "Vec3f", "of": "Vec3<f32>", "record": "Vec3<f32>"},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "alias", "name": "u32", "of": "unsigned int", "record": null},
    {"kind": "alias", "name": "u8", "of": "unsigned char", "record": null},
    {"kind": "alias", "name": "u16", "of": "unsigned short", "record": null},
    {"kind": "alias", "name": "f32", "of": "float", "record": null},
    {"kind": "enum", "name": "Mode", "underlying": "int", "scoped": true, "values": [{"name": "Walk", "value": 0}, {"name": "Swim", "value": 1}]}
  ],
  "globals": [
    {"name": "gCount", "type": "s32", "const": false, "mangled": "?gCount", "file": "g.hpp", "line": 1, "value": null, "defined": true},
    {"name": "gFixed", "type": "const s32", "const": true, "mangled": "?gFixed", "file": "g.hpp", "line": 1, "value": null, "defined": true},
    {"name": "Limits::MAX", "type": "const s32", "const": true, "mangled": "?Limits::MAX", "file": "g.hpp", "line": 1, "value": 8, "defined": false}
  ],
  "left_out": []
}"#;

const FUNCTIONS: &[(&str, &[&str])] = &[
    ("MR::setScale", &["void(LiveActor *, f32)", "void(LiveActor *, const Vec3f &)"]),
    ("MR::isNear", &["bool(const LiveActor *, const LiveActor *, f32)"]),
    ("MR::getCount", &["s32()"]),
    ("MR::readCount", &["void(s32 &)"]),
    ("MR::frontVec", &["void(Vec3f *)"]),
    ("MR::pick", &["void(s32)", "void(u32)"]),
    ("MR::pickF", &["void(f32)", "void(s32)"]),
    ("MR::useBox", &["void(const Box &)"]),
    ("MR::touch", &["void(Box *)"]),
    ("MR::fillBox", &["void(Box &)"]),
    ("MR::sendTo", &["bool(Sensor *, Sensor *)"]),
    ("MR::sensorOf", &["Sensor *(LiveActor *)"]),
    ("MR::readBox", &["void(const Box *)"]),
    ("MR::maybe", &["LiveActor *()"]),
    ("MR::name", &["const char *()"]),
    ("MR::varargs", &["void(const char *, ...)"]),
    ("MR::amb", &["void(u8)", "void(u16)"]),
    ("MR::raw", &["void *()"]),
    ("MR::thing", &["Thing *()"]),
    ("MR::constActor", &["const LiveActor *()"]),
    ("LiveActor::appear", &["void()"]),
    ("LiveActor::isAlive", &["bool() const"]),
    ("LiveActor::LiveActor", &["(const char *)"]),
    // Part (e): a virtual and its override, a destructor, a Kuribo member, functions to hook.
    ("LiveActor::control", &["void()"]),
    // Two virtuals of one name and parameter count (S0147).
    ("LiveActor::receive", &["void(s32)", "void(f32)"]),
    ("LiveActor::~LiveActor", &["()"]),
    ("Kuribo::control", &["void()"]),
    ("Kuribo::exeWander", &["void()"]),
    ("MR::getStarPieceNum", &["s32()"]),
    ("MR::addStarPiece", &["void(s32)"]),
    ("MR::readPos", &["void(const Vec3f &)"]),
    // A class passed by const reference (MR::useBox above is the plain struct case).
    ("MR::seeActor", &["void(const LiveActor &)"]),
    // marked pointer parameters (MARKS).
    ("MR::getPlayerFrontVec", &["void(Vec3f *)"]),
    ("MR::sum", &["f32(const f32 *, s32)"]),
    ("MR::fill", &["void(f32 *, u16)"]),
    ("MR::readRow", &["void(f32 *)"]),
    ("MR::nudge", &["void(Vec3f *)"]),
    ("MR::keepPos", &["void(Vec3f *)"]),
    ("MR::outConst", &["void(const Vec3f *)"]),
    ("MR::countF", &["void(Vec3f *, f32)"]),
    ("MR::setRow", &["void(const f32 *)"]),
    ("MR::countFirst", &["void(u8, f32 *, Vec3f *)"]),
    // `nullable` where nothing else crosses: `none` only.
    ("MR::keepMtx", &["void(Vec3f *)"]),
    ("MR::calcHost", &["bool(const LiveActor *, const void *)"]),
    ("MR::rawHost", &["void(const void *)"]),
];

/// The parameter marks of the rows that have any (functions file "marks"); every other row's are all null.
const MARKS: &[(&str, &str)] = &[
    ("MR::getPlayerFrontVec", r#"{"way": "out", "count": null, "fixed": null, "nullable": false}"#),
    ("MR::sum", r#"{"way": "in", "count": 1, "fixed": null, "nullable": false}, null"#),
    ("MR::fill", r#"{"way": "out", "count": 1, "fixed": null, "nullable": false}, null"#),
    ("MR::readRow", r#"{"way": "out", "count": null, "fixed": 3, "nullable": false}"#),
    ("MR::nudge", r#"{"way": "inout", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::keepPos", r#"{"way": "kept", "count": null, "fixed": null, "nullable": false}"#),
    ("MR::keepMtx", r#"{"way": "kept", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::outConst", r#"{"way": "out", "count": null, "fixed": null, "nullable": false}"#),
    ("MR::countF", r#"{"way": "in", "count": 1, "fixed": null, "nullable": false}, null"#),
    ("MR::setRow", r#"{"way": "in", "count": null, "fixed": 3, "nullable": false}"#),
    ("MR::countFirst", r#"null, {"way": "out", "count": 0, "fixed": null, "nullable": false}, {"way": "out", "count": null, "fixed": null, "nullable": false}"#),
    ("MR::keepMtx", r#"{"way": "kept", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::calcHost", r#"null, {"way": "in", "count": null, "fixed": null, "nullable": true}"#),
];

fn small_game() -> Game {
    let mut functions: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (name, sigs) in FUNCTIONS {
        let tell = name.ends_with("::LiveActor") || name.contains("::~");
        functions.insert(name.to_string(), sigs.iter().map(|s| (s.to_string(), tell)).collect());
    }
    let mut game = Game::new(functions, Some(Types::from_json(GAME.as_bytes()).unwrap_or_else(|e| panic!("{e}"))));
    // Each row's facts (docs/modding/language/types.md): the members of LiveActor and Kuribo are methods (control virtual, Kuribo's
    // overriding LiveActor's), its constructor and destructor theirs, the rest free.
    let rows: Vec<String> = FUNCTIONS
        .iter()
        .flat_map(|(name, sigs)| sigs.iter().map(move |s| (name, s)))
        .map(|(name, sig)| {
            let (kind, owner) = match *name {
                "LiveActor::LiveActor" => ("constructor", "\"LiveActor\""),
                "LiveActor::~LiveActor" => ("destructor", "\"LiveActor\""),
                n if n.starts_with("LiveActor::") => ("method", "\"LiveActor\""),
                n if n.starts_with("Kuribo::") => ("method", "\"Kuribo\""),
                _ => ("free", "null"),
            };
            let konst = sig.ends_with(" const");
            let is_virtual = name.ends_with("::control") || name.ends_with("::receive");
            let overrides = if *name == "Kuribo::control" { r#"[{"name": "LiveActor::control", "signature": "void()", "row": true}]"# } else { "[]" };
            let marks = MARKS.iter().find(|(n, _)| n == name).map_or_else(|| vec!["null"; signature_params(sig).unwrap()].join(", "), |(_, m)| m.to_string());
            format!(r#"{{"name": "{name}", "signature": "{sig}", "kind": "{kind}", "owner": {owner}, "virtual": {is_virtual}, "pure": false, "const": {konst}, "overrides": {overrides}, "marks": [{marks}]}}"#)
        })
        .collect();
    let json = format!(r#"{{"functions": [{}]}}"#, rows.join(", "));
    game.facts = Some(FunctionFacts::from_json(json.as_bytes()).unwrap_or_else(|e| panic!("{e}")));
    game
}

/// (code, line, column) of each diagnostic of one module, in order.
fn codes(text: &str) -> Vec<(u16, u32, u32)> {
    full(text).into_iter().map(|(c, l, col, _)| (c, l, col)).collect()
}

/// Each diagnostic as (code, line, column, args as text), in order.
type Said = Vec<(u16, u32, u32, Vec<(&'static str, String)>)>;

fn full(text: &str) -> Said {
    full_in(&small_game(), text)
}

fn full_in(game: &Game, text: &str) -> Said {
    let facts = ModFacts::of_game(game);
    // As a build gives them: the standard module after the mod's own.
    let checked = check::check(&[source("main", text), check::std_source()], &[], facts);
    checked
        .diags
        .into_iter()
        .map(|(_, d)| {
            let args = d.args.iter().map(|(n, a)| (*n, match a {
                Arg::Text(t) => t.clone(),
                Arg::Phrase(p) => format!("P:{p}"),
            })).collect();
            (d.code, d.at.line, d.at.col, args)
        })
        .collect()
}

/// The codes said, without places.
fn said(text: &str) -> Vec<u16> {
    codes(text).into_iter().map(|c| c.0).collect()
}

#[test]
fn a_clean_program_says_nothing() {
    let text = r#"struct Point {
    x: f32
    y: f32 = 0.0
    fn len2(self) -> f32 { return self.x * self.x + self.y * self.y }
    fn grow(var self, by: f32) { self.x += by }
}
enum Shape { Circle(r: f32), Rect { w: f32, h: f32 }, Dot }
let names = ["a", "b"]
var total = 0
fn area(s: Shape) -> f32 {
    return match s {
        Shape.Circle(r) => 3.14 * r * r
        Shape.Rect { w, h } => w * h
        Shape.Dot => 0.0
    }
}
fn twice(f: fn(int) -> int, x: int) -> int { return f(f(x)) }
on start {
    var p = Point { x: 1.0 }
    p.grow(2.0)
    let a = area(Shape.Circle(2.0)) + area(Shape.Rect { w: 1.0, h: 2.0 }) + area(Shape.Dot)
    let step = 3
    let add = fn(i: int) -> int { return i + step }
    total += twice(add, 1)
    var xs: list<int> = []
    xs.push(4)
    let evens = xs.filter(fn(i: int) -> bool { return i % 2 == 0 })
    let m: map<text, int> = {}
    let s = "{a} {total} {evens} \u{1F600}\n"
    let big = max(1, 2.5)
    let v = vec3(1, 2, 3) * 2.0 + vec3(0, 1, 0)
    let n = low_bits<u8>(300)
    let limit = u8.max
    if let first = xs.first(), first > 2 { log "{first}" }
    for (i, x) in xs.indexed() { total += i * x }
    for i in 0..10 { total += i }
    let j = json.encode(p)
    let back = json.decode<Point>(j)
    let w: json.Value = {"a": [1, 2.5, true, none]}
    log "{s} {big} {v.x} {n} {limit} {m.len()} {back?.x ?? 0.0} {w.get("a") != none}"
}
"#;
    assert_eq!(full(text), vec![]);
}

#[test]
fn game_calls_choose_their_overload() {
    let text = r#"on start {
    if let a = MR::maybe() {
        MR::setScale(a, 2.0)
        MR::setScale(a, vec3(1, 1, 1))
        MR::setScale(a, 2)
        let near = MR::isNear(a, a, 10)
        a.appear()
        let alive = a.isAlive()
        a.mPosition.x = 3.0
        let dead = a.mFlags.isDead
        let first = a.mTable[0]
        let m = Mode::Swim
        let c = MR::getCount() + gCount + Limits::MAX
        var k = 0
        MR::readCount(var k)
        MR::pick(1)
        MR::pickF(1.5)
        log "{near} {alive} {dead} {first} {c} {k} {a.mName ?? "-"}"
    }
}
"#;
    assert_eq!(full(text), vec![]);
}

#[test]
fn names_and_members() {
    assert_eq!(codes("on start {\n    log \"{nope}\"\n}\n"), vec![(code::UNKNOWN_NAME, 2, 11)]);
    assert_eq!(said("let count = 1\non start {\n    log \"{cont}\"\n}\n"), vec![code::UNKNOWN_NAME_NEAR]);
    assert_eq!(said("struct S { a: int }\non start {\n    let s = S { a: 1 }\n    log \"{s.b}\"\n}\n"), vec![code::NO_MEMBER]);
    assert_eq!(said("on start {\n    let x = 1\n    x(2)\n}\n"), vec![code::NOT_CALLABLE]);
    assert_eq!(said("fn f() { }\non start {\n    let g = f + 1\n}\n"), vec![code::OPERATOR]);
    assert_eq!(said("enum E { A }\non start {\n    let e = E\n}\n"), vec![code::NOT_A_VALUE]);
    assert_eq!(said("on start {\n    log \"{self}\"\n}\n"), vec![code::SELF_OUTSIDE]);
}

#[test]
fn types_and_literals() {
    assert_eq!(codes("on start {\n    let x: int = \"a\"\n}\n"), vec![(code::TYPE_MISMATCH, 2, 18)]);
    assert_eq!(said("on start {\n    let x: u8 = 300\n}\n"), vec![code::LITERAL_FIT]);
    assert_eq!(said("on start {\n    let x: f32 = 16777217\n}\n"), vec![code::LITERAL_FIT]);
    assert_eq!(said("on start {\n    let x = 1 as text\n}\n"), vec![code::AS_NUMBERS]);
    assert_eq!(said("on start {\n    let x = -\"a\"\n}\n"), vec![code::OPERATOR_ONE]);
    assert_eq!(said("on start {\n    let x = [1] == [1]\n    let f = fn() { }\n    let y = f == f\n}\n"), vec![code::NOT_EQUATABLE]);
    // A struct or enum compares when each part it holds does; one that holds itself through a list still does.
    let held = "struct S {\n    n: int\n    more: list<S>\n}\nstruct F {\n    f: fn()\n}\nenum E {\n    A\n    B(g: fn())\n}\n";
    assert_eq!(said(&format!("{held}fn f(a: S, b: F, c: E) {{\n    let x = a == a\n    let y = b == b\n    let z = c != c\n    let w = [a].contains(a)\n    let v = [b].contains(b)\n}}\n")), vec![code::NOT_EQUATABLE, code::NOT_EQUATABLE, code::NOT_EQUATABLE]);
    assert_eq!(said("on start {\n    let x = true < false\n}\n"), vec![code::NOT_ORDERED]);
    assert_eq!(said("on start {\n    let x = []\n}\n"), vec![code::TYPE_UNKNOWN_HERE]);
    assert_eq!(said("on start {\n    let m = {1: 2, 3}\n}\n"), vec![code::MIXED_BRACES]);
    assert_eq!(said("on start {\n    log \"a\\q\"\n}\n"), vec![code::BAD_ESCAPE]);
    assert_eq!(said("on start {\n    log \"\\u{110000}\"\n}\n"), vec![code::BAD_ESCAPE]);
    assert_eq!(said("on start {\n    log \"\\xFF\"\n    let b = b\"\\xFF\"\n}\n"), vec![code::BAD_ESCAPE]);
    assert_eq!(said("on start {\n    let f = fn() { }\n    log \"{f}\"\n}\n"), vec![code::NOT_SHOWN]);
    assert_eq!(said("fn f() { }\non start {\n    let x = f()\n}\n"), vec![code::NO_VALUE]);
    assert_eq!(said("on start {\n    let x = if true { 1 }\n}\n"), vec![code::IF_NEEDS_ELSE]);
    assert_eq!(said("on start {\n    let x = 3!\n}\n"), vec![code::UNWRAP_NOT_OPTIONAL]);
}

#[test]
fn calls_and_arguments() {
    let f = "fn f(a: int, b: int = 2, var c: int) { c += a + b }\n";
    assert_eq!(said(&format!("{f}on start {{\n    var c = 0\n    f(1, 2, var c, 4)\n}}\n")), vec![code::ARG_COUNT]);
    assert_eq!(said(&format!("{f}on start {{\n    var c = 0\n    f(1, d: 2, c: var c)\n}}\n")), vec![code::ARG_UNKNOWN]);
    assert_eq!(said(&format!("{f}on start {{\n    var c = 0\n    f(1, a: 2, c: var c)\n}}\n")), vec![code::ARG_TWICE]);
    assert_eq!(said(&format!("{f}on start {{\n    f(1)\n}}\n")), vec![code::ARG_MISSING]);
    assert_eq!(said(&format!("{f}on start {{\n    var c = 0\n    f(1, c: c)\n}}\n")), vec![code::VAR_ARG_NEEDED]);
    assert_eq!(said(&format!("{f}on start {{\n    var c = 0\n    f(var c, c: var c)\n}}\n")), vec![code::VAR_ARG_NOT_TAKEN]);
    assert_eq!(said(&format!("{f}on start {{\n    let c = 0\n    f(1, c: var c)\n}}\n")), vec![code::NOT_A_PLACE]);
    // A result-less task called gives `pending<()>`; dropped, it loses nothing.
    assert_eq!(said("task t() { }\non start {\n    t()\n}\ntask u() {\n    wait t()\n    let p = t()\n    wait p\n}\n"), vec![]);
    assert_eq!(said("fn t() { }\non start {\n    let h = start t()\n}\n"), vec![code::START_NOT_TASK]);
    assert_eq!(said("task t() { }\ntask u() {\n    let h = start t()\n    wait h\n    wait 2 frames\n    wait 0.5 seconds\n    h.stop()\n}\n"), vec![]);
    assert_eq!(said("task u() {\n    wait \"a\"\n}\n"), vec![code::WAIT_WHAT]);
}

#[test]
fn game_call_codes() {
    let on = |body: &str| format!("on start {{\n    if let a = MR::maybe() {{\n        {body}\n    }}\n}}\n");
    assert_eq!(said(&on("MR::setScale(a, \"x\")")), vec![code::GAME_NO_SIGNATURE]);
    assert_eq!(said(&on("MR::pick(a)")), vec![code::GAME_NO_SIGNATURE]);
    assert_eq!(said(&on("MR::isNear(a, a, d: 1.0)")), vec![code::ARG_NAMED_GAME]);
    assert_eq!(said(&on("MR::varargs(\"x\")")), vec![code::GAME_NONE_CROSSING]);
    assert_eq!(said(&on("MR::touch(a)")), vec![code::TYPE_MISMATCH]);
    assert_eq!(said(&on("var k = 0\n        MR::readCount(k)")), vec![code::VAR_ARG_NEEDED]);
    assert_eq!(said(&on("MR::isNear(a, a)")), vec![code::ARG_COUNT]);
    assert_eq!(said(&on("let n = a.mName!\n        a.mCount = 2")), vec![code::NOT_A_PLACE]);
    assert_eq!(said("on start {\n    gFixed = 1\n}\n"), vec![code::NOT_A_PLACE]);
    // the script's copy goes to a `const T &` only; a pointer, const or not, and a `T &`
    // take the game's object.
    assert_eq!(
        said(&on("let b = a.mBox\n        MR::touch(b)\n        MR::useBox(b)\n        MR::useBox(a.mBox)\n        MR::fillBox(b)\n        MR::readBox(b)")),
        vec![code::COPY_NOT_VIEW, code::COPY_NOT_VIEW, code::COPY_NOT_VIEW]
    );
    assert_eq!(said(&on("var v = vec3(0, 0, 0)\n        MR::frontVec(var v)")), vec![code::GAME_PARAM_NOT_CROSSING]);
    assert_eq!(said("on start {\n    LiveActor::appear()\n}\n"), vec![code::METHOD_NEEDS_OBJECT]);
    // The other half (A42C-5): by its full name on `self`, in a hook on its record and in a class derived from it
    // (LiveActor is derivable: NameObj at its start).
    assert_eq!(said("hook before LiveActor::appear() {\n    if LiveActor::isAlive() { log \"a\" }\n}\n"), vec![]);
    assert_eq!(said("class K : LiveActor {\n    fn f() {\n        LiveActor::appear()\n    }\n}\n"), vec![]);
    assert_eq!(said(&on("if a.isAlive() { log \"alive\" }")), vec![]);
    assert_eq!(said(&on("MR::amb(1)")), vec![code::GAME_AMBIGUOUS]);
    assert_eq!(said(&on("let r = MR::raw()")), vec![code::GAME_RESULT_NOT_CROSSING]);
    assert_eq!(said("on start {
    if let t = MR::thing() {
        let r = ref(t)
    }
}
"), vec![code::REF_NOT_DESTROYED]);
    assert_eq!(said("on start {
    if let c = MR::constActor() {
        c.appear()
        let ok = c.isAlive()
    }
}
"), vec![code::CONST_VIEW_CALL]);
    assert_eq!(said("task t(later: fn() -> pending<int>) {
    later()
    let n = wait later()
}
"), vec![code::PENDING_DROPPED]);
    assert_eq!(said("on start {\n    let a = MR::maybe()\n    a.appear()\n}\n"), vec![code::MAY_BE_NONE]);
    assert_eq!(said("on start {\n    MR::setScale(MR::maybe(), 1.0)\n}\n"), vec![code::MAY_BE_NONE]);
}

#[test]
fn literals_take_their_place() {
    // A42C-1: a shift's literal left side takes the place's type, else int; never the count's.
    assert_eq!(said("on start {\n    let n: u8 = 3\n    let big: int = 1 << n\n    let m = 1 << n\n    let x: int = m\n    let w: u64 = 1 << n\n}\n"), vec![]);
    // A42C-2: an expression of literals takes its place's number type, its value checked once.
    assert_eq!(said("on start {\n    let x: u8 = 1 + 2\n    let mask: u32 = 1 << 4\n    let f: f32 = 1 + 2\n    let g: f32 = 1.5 * 2\n    let o: u16? = 2 * 3\n}\n"), vec![]);
    assert_eq!(full("on start {\n    let y: u8 = 200 + 100\n}\n"), vec![(code::LITERAL_FIT, 2, 17, vec![("found", "300".to_string()), ("expected", "u8".to_string())])]);
    assert_eq!(said("on start {\n    let z = 1 << 40\n    let k: u8 = (1 << 4) - 17\n}\n"), vec![code::LITERAL_FIT, code::LITERAL_FIT]);
    assert_eq!(said("on start {\n    let h: u8 = 300 + 1\n}\n"), vec![code::LITERAL_FIT]);
}

#[test]
fn flow_returns_and_guards() {
    // S0101: a function with a result that can reach its end, said at its closing brace.
    assert_eq!(full("fn f(x: int) -> int {\n    if x > 0 { return 1 }\n}\n"), vec![(code::MISSING_RETURN, 3, 1, vec![("expected", "i32".to_string())])]);
    assert_eq!(said("fn f(x: int) -> int {\n    if x > 0 { return 1 } else { return 2 }\n}\n"), vec![]);
    // `while true` that nothing leaves never ends; one with a `break` does.
    assert_eq!(said("fn f() -> int {\n    while true {\n    }\n}\n"), vec![]);
    assert_eq!(said("fn f() -> int {\n    while true {\n        break\n    }\n}\n"), vec![code::MISSING_RETURN]);
    // A `break` of a loop inside it does not leave the outer `while true`.
    assert_eq!(said("fn f() -> int {\n    while true {\n        for i in 0..2 { break }\n    }\n}\n"), vec![]);
    assert_eq!(said("on start {\n    let g = fn(x: int) -> int { if x > 0 { return 1 } }\n}\n"), vec![code::MISSING_RETURN]);
    assert_eq!(said("struct P {\n    x: int\n    fn get(self) -> int { log \"x\" }\n}\n"), vec![code::MISSING_RETURN]);
    // S0102: a guard's `else` must leave.
    assert_eq!(full("fn f(x: int?) {\n    guard let y = x else { log \"no\" }\n}\n"), vec![(code::GUARD_STAYS, 2, 26, vec![])]);
    assert_eq!(said("fn f(x: int?) {\n    guard let y = x else { return }\n    log y\n}\n"), vec![]);
    assert_eq!(said("fn f(xs: list<int?>) {\n    for x in xs {\n        guard let y = x else { continue }\n        log y\n    }\n}\n"), vec![]);
    assert_eq!(said("task t(x: int?) {\n    guard let y = x else { stop }\n}\n"), vec![]);
}

#[test]
fn flow_views_kept() {
    // S0103: where a view would outlive its statement: a module value, a field, a task's parameter.
    assert_eq!(full("var keep: LiveActor? = none\n"), vec![(code::VIEW_KEPT, 1, 5, vec![("found", "keep".to_string()), ("why", "P:Script.Keep.Module".to_string())])]);
    assert_eq!(full("struct S {\n    a: LiveActor\n}\n"), vec![(code::VIEW_KEPT, 2, 5, vec![("found", "a".to_string()), ("why", "P:Script.Keep.Field".to_string())])]);
    assert_eq!(said("enum E {\n    One(a: list<LiveActor>)\n}\n"), vec![code::VIEW_KEPT]);
    assert_eq!(full("task t(a: LiveActor) {\n    wait 1 frames\n}\n"), vec![(code::VIEW_KEPT, 1, 8, vec![("found", "a".to_string()), ("why", "P:Script.Keep.Task".to_string())])]);
    assert_eq!(said("class K : LiveActor {\n    var other: LiveActor? = none\n}\n"), vec![code::VIEW_KEPT]);
    // A handle is the way to keep one; a plain game struct is a copy; a local list of views lives as long as a view.
    assert_eq!(said("var keep: ref<LiveActor>? = none\nvar box = MR::maybe()?.mBox\n"), vec![]);
    assert_eq!(said("hook before LiveActor::appear() {\n    let all = [self, self]\n    for a in all { a.appear() }\n}\n"), vec![]);
    // A closure that may run later keeps what it reads; a built-in's closure argument runs at once.
    assert_eq!(said("hook before LiveActor::appear() {\n    let ok = [1, 2].any(fn(i: int) -> bool { return self.isAlive() })\n}\n"), vec![]);
    assert_eq!(full("hook before LiveActor::appear() {\n    let later = fn() -> bool { return self.isAlive() }\n}\n"), vec![(code::VIEW_KEPT, 2, 39, vec![("found", "self".to_string()), ("why", "P:Script.Keep.Closure".to_string())])]);
    assert_eq!(said("hook before LiveActor::appear() {\n    let me = self\n    let later = fn() -> bool { return me.isAlive() }\n}\n"), vec![code::VIEW_KEPT]);
    assert_eq!(said("hook before LiveActor::appear() {\n    let later = fn() { appear() }\n}\n"), vec![code::VIEW_KEPT]);
    // A state's `update` and `exit` run on later frames than its body: a view the body made is kept by the state.
    let states = |body: &str| format!("class K : LiveActor {{\n}}\nstates S for K {{\n    state A {{\n{body}    }}\n    start A\n}}\n");
    assert_eq!(said(&states("        let a = MR::maybe()\n        update { a?.appear() }\n")), vec![code::VIEW_KEPT]);
    assert_eq!(said(&states("        update {\n            let a = MR::maybe()\n            a?.appear()\n        }\n")), vec![]);
    assert_eq!(
        said(&states("        enter { wait 1 frames }\n        wait 2 frames\n        update { wait 1 frames }\n")),
        vec![code::WAIT_HERE]
    );
    // Inside a built-in's closure, a closure that may run later still keeps it.
    assert_eq!(said("hook before LiveActor::appear() {\n    let n = [1].map(fn(i: int) -> fn() { return fn() { self.appear() } })\n}\n"), vec![code::VIEW_KEPT]);
}

#[test]
fn flow_views_across_wait() {
    // S0104: a view made before a wait and used after it, naming the wait's line.
    assert_eq!(
        full("task t(h: ref<LiveActor>) {\n    guard let a = h.get() else { stop }\n    wait 1 frames\n    a.appear()\n}\n"),
        vec![(code::VIEW_ACROSS_WAIT, 4, 5, vec![("found", "a".to_string()), ("line", "3".to_string())])]
    );
    assert_eq!(said("task t(h: ref<LiveActor>) {\n    wait 1 frames\n    guard let a = h.get() else { stop }\n    a.appear()\n}\n"), vec![]);
    // Read again the next time round a loop that waits.
    assert_eq!(
        full("task t(h: ref<LiveActor>) {\n    guard let a = h.get() else { stop }\n    for i in 0..3 {\n        a.appear()\n        wait 1 frames\n    }\n}\n"),
        vec![(code::VIEW_ACROSS_WAIT, 4, 9, vec![("found", "a".to_string()), ("line", "5".to_string())])]
    );
    assert_eq!(said("task t(h: ref<LiveActor>) {\n    for i in 0..3 {\n        guard let a = h.get() else { stop }\n        a.appear()\n        wait 1 frames\n    }\n}\n"), vec![]);
    // `wait until`'s condition is read on each frame after the wait starts.
    assert_eq!(said("task t(h: ref<LiveActor>) {\n    guard let a = h.get() else { stop }\n    wait until a.isAlive()\n}\n"), vec![code::VIEW_ACROSS_WAIT]);
    // A loop going on through a local list of views.
    assert_eq!(said("on start {\n    guard let a = MR::maybe() else { return }\n    let all = [a]\n    for b in all {\n        wait 1 frames\n    }\n}\n"), vec![code::VIEW_ACROSS_WAIT]);
    // Said once for each local; a plain value across a wait is fine.
    assert_eq!(said("task t(h: ref<LiveActor>) {\n    guard let a = h.get() else { stop }\n    let n = 3\n    wait 1 frames\n    a.appear()\n    a.appear()\n    log n\n}\n"), vec![code::VIEW_ACROSS_WAIT]);
    // Paths, closures that run now, lists made in the `for` (D1 to D5 and more).
    let g = |body: &str| format!("task t(h: ref<LiveActor>, c: bool, n: int, p: pending<int?>) {{\n    guard let a = h.get() else {{ stop }}\n{body}}}\n");
    // D1: a branch's wait is not on its sibling's path. D2: it is on the path after the if.
    assert_eq!(said(&g("    if c { wait 1 frames } else { a.appear() }\n")), vec![]);
    assert_eq!(full(&g("    if c { wait 1 frames }\n    a.appear()\n")), vec![(code::VIEW_ACROSS_WAIT, 4, 5, vec![("found", "a".to_string()), ("line", "3".to_string())])]);
    assert_eq!(said(&g("    if c { wait 1 frames } else if n > 0 { a.appear() }\n")), vec![]);
    assert_eq!(said(&g("    match n {\n        1 => wait 1 frames\n        else => a.appear()\n    }\n")), vec![]);
    assert_eq!(said(&g("    match n {\n        1 => wait 1 frames\n        else => log \"x\"\n    }\n    a.appear()\n")), vec![code::VIEW_ACROSS_WAIT]);
    // A branch that leaves takes its wait with it; a `break` takes it out of the loop.
    assert_eq!(said(&g("    if c {\n        wait 1 frames\n        stop\n    }\n    a.appear()\n")), vec![]);
    assert_eq!(said(&g("    while c {\n        if n > 0 {\n            wait 1 frames\n            stop\n        }\n        a.appear()\n    }\n    a.appear()\n")), vec![]);
    assert_eq!(said(&g("    while true {\n        if c {\n            wait 1 frames\n            break\n        }\n    }\n    a.appear()\n")), vec![code::VIEW_ACROSS_WAIT]);
    assert_eq!(said(&g("    while c {\n        a.appear()\n        if n > 0 {\n            wait 1 frames\n            continue\n        }\n    }\n")), vec![code::VIEW_ACROSS_WAIT]);
    // A guard's `else` runs after its conditions, so after a `wait` that is one.
    assert_eq!(said(&g("    guard let k = wait p else {\n        a.appear()\n        stop\n    }\n")), vec![code::VIEW_ACROSS_WAIT]);
    assert_eq!(said(&g("    guard c else {\n        wait 1 frames\n        stop\n    }\n    a.appear()\n")), vec![]);
    // D3: a list made in the `for` that its body waits in.
    assert_eq!(full(&g("    for b in [a, a] {\n        b.appear()\n        wait 1 frames\n    }\n")), vec![(code::VIEW_ACROSS_WAIT, 3, 14, vec![("found", "b".to_string()), ("line", "5".to_string())])]);
    assert_eq!(said(&g("    for i in [1, 2] {\n        wait 1 frames\n    }\n")), vec![]);
    // D4, D5: a built-in's closure argument runs where it is written.
    assert_eq!(full(&g("    wait 1 frames\n    let ok = [1].any(fn(i: int) -> bool { return a.isAlive() })\n")), vec![(code::VIEW_ACROSS_WAIT, 4, 50, vec![("found", "a".to_string()), ("line", "3".to_string())])]);
    assert_eq!(said(&g("    for i in 0..3 {\n        let ok = [1].any(fn(k: int) -> bool { return a.isAlive() })\n        wait 1 frames\n    }\n")), vec![code::VIEW_ACROSS_WAIT]);
    assert_eq!(said(&g("    let ok = [1].any(fn(i: int) -> bool { return a.isAlive() })\n    wait 1 frames\n")), vec![]);
}

#[test]
fn flow_wait_and_defer() {
    // S0105: `wait` only in a task, a state's body or `enter`, a handler of start, a stage, an option or an event, a
    // game test.
    let where_ = |line: u32, col: u32| (code::WAIT_HERE, line, col, vec![("why", "P:Script.Wait.Where".to_string())]);
    assert_eq!(full("fn f() {\n    wait 1 frames\n}\n"), vec![where_(2, 5)]);
    assert_eq!(full("hook before LiveActor::appear() {\n    wait 1 frames\n}\n"), vec![where_(2, 5)]);
    assert_eq!(full("on frame {\n    wait 1 frames\n}\n"), vec![where_(2, 5)]);
    assert_eq!(full("test \"t\" {\n    wait 1 frames\n}\n"), vec![where_(2, 5)]);
    assert_eq!(full("on start {\n    let f = fn() { wait 1 frames }\n}\n"), vec![where_(2, 20)]);
    assert_eq!(said("on start {\n    wait 1 frames\n}\ntest game \"t\" {\n    wait 1 seconds\n}\ntask t() {\n    wait 2 frames\n}\n"), vec![]);
    // Only as the first thing a value reads: nothing of the statement is held across it.
    assert_eq!(said("task t(p: pending<int?>) {\n    let m = wait p ?? 0\n    let k = (wait p ?? 0) + 1\n}\n"), vec![]);
    assert_eq!(full("task t(p: pending<int?>) {\n    let n = 1 + (wait p ?? 0)\n}\n"), vec![(code::WAIT_HERE, 2, 18, vec![("why", "P:Script.Wait.Inside".to_string())])]);
    // S0106: nothing leaves or waits inside `defer`; a loop inside it may break.
    assert_eq!(full("fn f() {\n    defer return\n}\n"), vec![(code::DEFER_LEAVES, 2, 11, vec![("found", "return".to_string())])]);
    assert_eq!(said("fn f() {\n    for i in 0..3 {\n        defer break\n    }\n}\n"), vec![code::DEFER_LEAVES]);
    assert_eq!(said("fn f() {\n    defer for i in 0..3 { break }\n}\n"), vec![]);
    assert_eq!(full("task t() {\n    defer wait 1 frames\n}\n"), vec![(code::DEFER_LEAVES, 2, 11, vec![("found", "wait".to_string())])]);
    assert_eq!(said("task t() {\n    defer stop\n}\n"), vec![code::DEFER_LEAVES]);
}

/// What is said, without the notes that a fact is not described (part e's tests read the codes it gives).
fn heard(text: &str) -> Said {
    full(text).into_iter().filter(|d| d.0 != code::NOT_DESCRIBED).collect()
}

fn args(list: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    list.iter().map(|(n, v)| (*n, v.to_string())).collect()
}

/// A parameter's written type fits the game's when it names the same record: a game struct written by name for a
/// `const S &`, a class for a `const C &` (a script cannot write `const`); another record does not.
#[test]
fn written_parameter_types_fit_const_references() {
    assert_eq!(said("hook before MR::useBox(b: Box) { }\n"), vec![]);
    assert_eq!(said("hook before MR::seeActor(a: LiveActor) { }\n"), vec![]);
    assert_eq!(said("hook before MR::useBox(b: LiveActor) { }\n"), vec![code::TYPE_MISMATCH]);
}

#[test]
fn hooks_that_are_right_say_nothing() {
    let text = r#"hook before LiveActor::control() {
    if self.isAlive() { skip }
}
hook after MR::getStarPieceNum() -> n {
    result = n * 2
}
wrap MR::addStarPiece(num) {
    num *= 2
    proceed()
}
replace Kuribo::exeWander() {
    self.appear()
}
hook before `MR::isNear(const LiveActor *, const LiveActor *, f32)`(a, b, dist) -> bool {
    if dist < 0.0 { skip false }
}
hook before MR::setScale(actor, s: f32) {
    if s > 2.0 { skip }
}
wrap MR::getStarPieceNum() {
    if MR::getCount() > 3 {
        result = 1
    } else {
        proceed()
        result = result + 1
    }
}
hook after LiveActor::LiveActor(name) {
    self.appear()
}
hook before LiveActor::~LiveActor() {
    self.appear()
}
hook before every override of LiveActor::control() { }
class Ball : LiveActor {
    hook before control() { skip }
}
hook before MR::frontVec(v) {
    if let p = v { log "{p.x}" }
}
wrap MR::addStarPiece(n) {
    for i in 0..3 {
        if i == 1 {
            proceed()
            break
        }
    }
}
"#;
    assert_eq!(heard(text), vec![]);
    // The checked tree has each hook's row; `every override of` lists Kuribo's.
    let game = small_game();
    let facts = ModFacts::of_game(&game);
    let checked = check::check(&[source("main", text)], &[], facts);
    let mut rows: Vec<(String, String)> = checked.hooks.values().map(|h| (h.name.clone(), h.signature.clone())).collect();
    rows.sort();
    rows.dedup();
    assert!(rows.contains(&("MR::setScale".to_string(), "void(LiveActor *, f32)".to_string())), "{rows:?}");
    assert!(rows.contains(&("MR::isNear".to_string(), "bool(const LiveActor *, const LiveActor *, f32)".to_string())));
    let every = checked.hooks.values().find(|h| !h.overrides.is_empty()).expect("the every-override hook");
    assert_eq!(every.overrides, vec![("Kuribo::control".to_string(), "void()".to_string())]);
}

/// Language 5.5's class head and overrides: a derivable base (polymorphic, NameObj at its start),
/// `made by` and `replaces` naming game classes, an override of a virtual of the base's chain, `base.` its own.
#[test]
fn class_codes() {
    assert_eq!(said("class K : Thing {\n}\n"), vec![code::NOT_DERIVABLE]);
    // Without the placement facts the factory's rows are the kernel's to check at define_class.
    assert_eq!(said("class K : LiveActor made by Nope {\n}\n"), vec![]);
    assert_eq!(said("class Kuribo : LiveActor replaces Kuribo {\n}\n"), vec![code::REPLACES_ITSELF]);
    // With them (docs/modding/language/game-description.md, section 9): both names are rows, and the maker (`made by`, else the base's name) makes
    // exactly the base; a row whose class the facts do not say is taken.
    let mut game = small_game();
    game.factory = Some(vec![
        ("Kuribo".into(), Some("Kuribo".into())),
        ("KuriboMini".into(), Some("Kuribo".into())),
        ("Plain".into(), Some("LiveActor".into())),
        ("Unsure".into(), None),
    ]);
    let made = |text: &str| full_in(&game, text).into_iter().map(|d| d.0).collect::<Vec<_>>();
    assert_eq!(made("class K : Kuribo made by Kuribo replaces Kuribo {\n}\n"), vec![]);
    assert_eq!(made("class K : Kuribo replaces KuriboMini {\n}\n"), vec![]);
    // Case is the factory's: ignored.
    assert_eq!(made("class K : Kuribo replaces kuribomini {\n}\n"), vec![]);
    assert_eq!(made("class K : LiveActor made by Plain replaces Kuribo {\n}\n"), vec![]);
    assert_eq!(made("class K : LiveActor made by Unsure {\n}\n"), vec![]);
    assert_eq!(made("class K : LiveActor made by Nope replaces Goomba {\n}\n"), vec![code::CLASS_NAME_UNKNOWN, code::CLASS_NAME_UNKNOWN]);
    assert_eq!(
        full_in(&game, "class K : LiveActor replaces Kuribo {\n}\n"),
        vec![(code::BASE_NOT_MADE, 1, 11, args(&[("found", "K"), ("base", "LiveActor")]))]
    );
    assert_eq!(
        full_in(&game, "class K : LiveActor made by Kuribo {\n}\n"),
        vec![(code::MAKER_MAKES_OTHER, 1, 29, args(&[("found", "Kuribo"), ("made", "Kuribo"), ("base", "LiveActor")]))]
    );
    // The head in its order only (a parse error).
    let (_, parsed) = lumascript::parse_module("class K : Kuribo replaces Kuribo made by Kuribo {\n}\n");
    assert_eq!(parsed.iter().map(|d| (d.code, d.at.line, d.at.col)).collect::<Vec<_>>(), vec![(code::MADE_BY_AFTER_REPLACES, 1, 34)]);
    // Several virtuals of the name, and the parameter count does not choose.
    assert_eq!(
        heard("class K : LiveActor {\n    override fn receive(a) {\n    }\n}\n").into_iter().next(),
        Some((code::OVERRIDE_AMBIGUOUS, 2, 17, args(&[("found", "receive"), ("class", "LiveActor"), ("signatures", "void(s32); void(f32)")])))
    );
    assert_eq!(heard("class K : LiveActor {\n    override fn appear() {\n    }\n}\n"), vec![(code::OVERRIDE_NOT_VIRTUAL, 2, 17, args(&[("found", "appear"), ("class", "LiveActor")]))]);
    // A virtual of a base's base: Kuribo's chain has LiveActor::control.
    assert_eq!(said("class K : Kuribo {\n    override fn control() {\n        base.control()\n    }\n}\n"), vec![]);
    assert_eq!(heard("class K : LiveActor {\n    override fn control() {\n        base.appear()\n    }\n}\n"), vec![(code::BASE_CALL, 3, 14, args(&[("found", "appear"), ("name", "control")]))]);
    assert_eq!(said("class K : LiveActor {\n    override fn control() -> i32 {\n        return 1\n    }\n}\n"), vec![code::TYPE_MISMATCH, code::RETURN_NO_VALUE]);
    // Its own fields and the base's members, by name and on self.
    assert_eq!(said("class K : LiveActor {\n    var n = 0\n    override fn control() {\n        n += 1\n        self.n += 1\n        if isAlive() { base.control() }\n    }\n}\n"), vec![]);
    // `base` named by a local is that local.
    assert_eq!(said("class K : LiveActor {\n    override fn control() {\n        let base = 1\n        log \"{base}\"\n    }\n}\n"), vec![]);
}

#[test]
fn hook_codes() {
    // S0047, S0112 to S0116: the head.
    assert_eq!(heard("hook before MR::nothing() { }
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::UNKNOWN_NAME]);
    assert_eq!(heard("hook before MR::isNear(a, b) { }
"), vec![(code::HOOK_PARAMS, 1, 13, args(&[("name", "MR::isNear"), ("found", "2"), ("expected", "3")]))]);
    assert_eq!(heard("wrap LiveActor::LiveActor(name) {
    proceed()
}
"), vec![(code::HOOK_TELL, 1, 1, args(&[("found", "LiveActor::LiveActor")]))]);
    assert_eq!(heard("hook before every override of LiveActor::appear() { }
"), vec![(code::HOOK_NOT_VIRTUAL, 1, 31, args(&[("found", "LiveActor::appear")]))]);
    assert_eq!(heard("class Ball : LiveActor {
    hook before MR::getCount() { }
}
"), vec![(code::HOOK_NOT_OF_CLASS, 2, 17, args(&[("found", "MR::getCount"), ("class", "Ball")]))]);
    assert_eq!(heard("class Ball : LiveActor {
    hook before Kuribo::exeWander() { }
}
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::HOOK_NOT_OF_CLASS]);
    assert_eq!(heard("hook after LiveActor::appear() -> r { }
"), vec![(code::HOOK_NO_RESULT, 1, 35, args(&[("found", "LiveActor::appear")]))]);
    assert_eq!(heard("hook before MR::getCount() -> f32 {
    skip 1.0
}
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::TYPE_MISMATCH]);
    assert_eq!(heard("hook before MR::setScale(a, b) { }
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::GAME_AMBIGUOUS]);
    assert_eq!(heard("hook before MR::setScale(a, b: text) { }
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::GAME_NO_SIGNATURE]);
    assert_eq!(heard("hook before `MR::getCount(s32)` { }
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::GAME_NO_SIGNATURE]);
    // S0117 to S0119: skip.
    assert_eq!(heard("fn f() {
    skip
}
"), vec![(code::SKIP_OUTSIDE, 2, 5, vec![])]);
    assert_eq!(heard("hook after LiveActor::appear() {
    skip
}
"), vec![(code::SKIP_OUTSIDE, 2, 5, vec![])]);
    assert_eq!(heard("hook before LiveActor::appear() {
    skip 1
}
"), vec![(code::SKIP_VALUE, 2, 10, args(&[("found", "LiveActor::appear")]))]);
    assert_eq!(heard("hook before MR::getCount() {
    skip
}
"), vec![(code::SKIP_NEEDS_VALUE, 2, 5, args(&[("found", "MR::getCount"), ("expected", "i32")]))]);
    // S0120, S0124, S0123: result.
    assert_eq!(heard("hook before MR::getCount() {
    let x = result
}
"), vec![(code::RESULT_HERE, 2, 13, args(&[("why", "P:Script.Result.Before")]))]);
    assert_eq!(heard("fn f() {
    let x = result
}
"), vec![(code::RESULT_HERE, 2, 13, args(&[("why", "P:Script.Result.NotHook")]))]);
    assert_eq!(heard("wrap MR::getCount() {
    let y = result
    proceed()
}
"), vec![(code::RESULT_NOT_YET, 2, 13, vec![])]);
    assert_eq!(heard("wrap MR::getCount() {
    if MR::getCount() > 1 {
        proceed()
    }
}
"), vec![(code::RESULT_NOT_GIVEN, 5, 1, args(&[("found", "MR::getCount"), ("expected", "i32")]))]);
    assert_eq!(heard("replace MR::getCount() {
    if MR::getCount() > 1 {
        return
    }
    result = 2
}
"), vec![(code::RESULT_NOT_GIVEN, 3, 9, args(&[("found", "MR::getCount"), ("expected", "i32")]))]);
    assert_eq!(heard("hook after LiveActor::appear() {
    result = 1
}
"), vec![(code::HOOK_NO_RESULT, 2, 5, args(&[("found", "LiveActor::appear")]))]);
    // S0121, S0122: proceed.
    assert_eq!(heard("hook after MR::getCount() {
    proceed()
}
"), vec![(code::PROCEED_HERE, 2, 5, args(&[("why", "P:Script.Proceed.NotWrap")]))]);
    assert_eq!(heard("replace MR::addStarPiece(n) {
    proceed()
}
"), vec![(code::PROCEED_HERE, 2, 5, args(&[("why", "P:Script.Proceed.Replace")]))]);
    assert_eq!(heard("wrap MR::addStarPiece(n) {
    proceed()
    proceed()
}
"), vec![(code::PROCEED_TWICE, 3, 5, args(&[("line", "2")]))]);
    assert_eq!(heard("wrap MR::addStarPiece(n) {
    for i in 0..3 {
        proceed()
    }
}
"), vec![(code::PROCEED_TWICE, 3, 9, args(&[("line", "3")]))]);
    assert_eq!(heard("wrap MR::addStarPiece(n) {
    let f = fn() { proceed() }
}
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::PROCEED_HERE]);
    // S0128: self where the object is not there; S0064: a `const T &` parameter is not written; `return` has no value.
    assert_eq!(heard("hook before LiveActor::LiveActor(name) {
    self.appear()
}
"), vec![(code::SELF_NOT_BUILT, 2, 5, args(&[("why", "P:Script.Self.NotYet")]))]);
    assert_eq!(heard("hook after LiveActor::~LiveActor() {
    self.appear()
}
"), vec![(code::SELF_NOT_BUILT, 2, 5, args(&[("why", "P:Script.Self.Gone")]))]);
    assert_eq!(heard("hook before MR::readPos(p) {
    p = vec3(0, 0, 0)
}
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::NOT_A_PLACE]);
    assert_eq!(heard("hook before MR::getCount() {
    return 1
}
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::RETURN_NO_VALUE]);
    assert_eq!(heard("hook before MR::getCount() {
    self.appear()
}
").iter().map(|d| d.0).collect::<Vec<_>>(), vec![code::SELF_OUTSIDE]);
}

#[test]
fn tasks_states_and_draft_10() {
    // S0125, S0126: stop and go where they cannot be.
    assert_eq!(heard("fn f() {
    stop
}
"), vec![(code::STOP_HERE, 2, 5, vec![])]);
    assert_eq!(heard("on frame {
    stop
}
"), vec![(code::STOP_HERE, 2, 5, vec![])]);
    assert_eq!(heard("fn f() {
    go A
}
"), vec![(code::GO_HERE, 2, 5, args(&[("what", "go")]))]);
    assert_eq!(heard("task t() {
    stop
}
on start {
    stop
}
"), vec![]);
    // States: go from the machine and from the class's functions; S0127, S0129, S0130.
    let text = r#"class Orb : LiveActor {
    fn hurt() { go OrbStates.Hurt }
    fn bare() { go Hurt }
    fn wrong() { go OrbStates.Gone }
}
states OrbStates for Orb {
    state Idle {
        enter { }
        enter { }
        update { go Hurt }
    }
    state Hurt {
        wait 2 seconds
        stop
    }
    start Idle
}
"#;
    assert_eq!(heard(text), vec![
        (code::GO_NAME_MACHINE, 3, 20, args(&[("what", "go"), ("found", "Hurt"), ("machine", "OrbStates")])),
        (code::GO_UNKNOWN, 4, 31, args(&[("found", "Gone"), ("machine", "OrbStates")])),
        (code::STATE_PART_TWICE, 9, 15, args(&[("found", "enter"), ("line", "8")])),
    ]);
    // `is_state` placed and resolved as `go`; S0131 for an argument that is no state's name.
    let text = r#"class Orb : LiveActor {
    fn dizzy() -> bool { return is_state(OrbStates.Idle) }
    fn bare() -> bool { return is_state(Idle) }
    fn wrong() -> bool { return is_state(OrbStates.Gone) }
    fn value() -> bool { return is_state(1) }
    fn two() -> bool { return is_state(OrbStates.Idle, OrbStates.Idle) }
}
states OrbStates for Orb {
    state Idle {
        update { if is_state(Idle) { go Idle } }
    }
    start Idle
}
fn outside() -> bool { return is_state(OrbStates.Idle) }
"#;
    assert_eq!(heard(text), vec![
        (code::GO_NAME_MACHINE, 3, 41, args(&[("what", "is_state"), ("found", "Idle"), ("machine", "OrbStates")])),
        (code::GO_UNKNOWN, 4, 52, args(&[("found", "Gone"), ("machine", "OrbStates")])),
        (code::STATE_ARG, 5, 42, vec![]),
        (code::ARG_COUNT, 6, 31, args(&[("name", "is_state"), ("found", "2"), ("expected", "1")])),
        (code::GO_HERE, 14, 31, args(&[("what", "is_state")])),
    ]);
    // one `on` of a kind a mod (S0109), `on step` may not wait (S0105), a literal negative wait (S0110).
    assert_eq!(heard("on frame { }
on step { }
on frame { }
"), vec![(code::ON_TWICE, 3, 1, args(&[("what", "frame"), ("first", "main:1")]))]);
    assert_eq!(heard("on step {
    wait 1 frames
}
"), vec![(code::WAIT_HERE, 2, 5, args(&[("why", "P:Script.Wait.Where")]))]);
    assert_eq!(heard("task t() {
    wait -2 frames
    wait -0.5 seconds
    wait -0 frames
}
"), vec![
        (code::WAIT_NEGATIVE, 2, 10, args(&[("found", "-2")])),
        (code::WAIT_NEGATIVE, 3, 10, args(&[("found", "-0.5")])),
    ]);
    // Part (e)'s fact for 4.5: a task's local read after a `wait` it was bound before.
    let game = small_game();
    let facts = ModFacts::of_game(&game);
    let checked = check::check(&[source("main", "task t() {
    let n = 3
    let m = 4
    wait 1 frames
    log \"{n}\"
}
")], &[], facts);
    let kept: Vec<&str> = checked.across_wait.iter().map(|id| checked.locals[*id as usize].name.as_str()).collect();
    assert_eq!(kept, vec!["n"]);
}

#[test]
fn match_and_patterns() {
    let e = "enum E { A, B(x: int), C { y: int } }\n";
    assert_eq!(full(&format!("{e}fn f(e: E) {{\n    match e {{\n        E.A => log \"a\"\n    }}\n}}\n")), vec![(code::MATCH_MISSING, 3, 5, vec![("cases", "E.B, E.C".to_string())])]);
    // The game's enum: its values as a script writes them; listing each is complete.
    assert_eq!(full("fn f(m: Mode) {\n    match m {\n        Mode.Walk => log \"w\"\n    }\n}\n"), vec![(code::MATCH_MISSING, 2, 5, vec![("cases", "Mode.Swim".to_string())])]);
    assert_eq!(said("fn f(m: Mode) {\n    match m {\n        Mode.Walk => log \"w\"\n        Mode.Swim => log \"s\"\n    }\n    let n = m as int\n    let back = 1 as Mode\n}\n"), vec![]);
    assert_eq!(said("fn f(n: int) {\n    match n {\n        1 => log \"a\"\n    }\n}\n"), vec![code::MATCH_NEEDS_ELSE]);
    assert_eq!(said("fn f(n: (bool, bool)) {\n    match n {\n        (true, false) => log \"a\"\n        (false, false) => log \"b\"\n    }\n}\n").len(), 1);
    assert_eq!(said("fn f(t: (int, int)) {\n    let (1, x) = t\n}\n"), vec![code::REFUTABLE_PATTERN]);
}

#[test]
fn flow_codes() {
    assert_eq!(said("fn f() {\n    break\n}\n"), vec![code::OUTSIDE_LOOP]);
    assert_eq!(said("fn f() -> int {\n    return\n}\n"), vec![code::RETURN_NEEDS_VALUE]);
    assert_eq!(said("fn f() {\n    return 1\n}\n"), vec![code::RETURN_NO_VALUE]);
    assert_eq!(said("on start {\n    for x in 3 { }\n}\n"), vec![code::NOT_ITERABLE]);
    assert_eq!(said("on start {\n    let x = 1\n    x = 2\n}\n"), vec![code::NOT_A_PLACE]);
}

#[test]
fn values_structs_refs() {
    assert_eq!(said("struct S { a: int, b: int }\non start {\n    let s = S { a: 1 }\n}\n"), vec![code::FIELD_MISSING]);
    assert_eq!(said("enum E { A }\non start {\n    let s = E { a: 1 }\n}\n"), vec![code::NOT_A_STRUCT]);
    assert_eq!(said("on start {\n    let x = 3\n    log \"{x[0]}\"\n}\n"), vec![code::NOT_INDEXABLE]);
    assert_eq!(said("on start {\n    let r = ref(3)\n}\n"), vec![code::REF_OF]);
    assert_eq!(said("on start {\n    if let t = MR::maybe() {\n        let r = ref(t)\n    }\n}\n"), vec![]);
    // A value whose type depends on itself: said once, where the cycle closes.
    assert_eq!(codes("let a = b\nlet b = a\n"), vec![(code::TYPE_NEEDS_WRITING, 2, 9)]);
    assert_eq!(said("on start {\n    let x = json.encode(fn() { })\n}\n"), vec![code::NOT_JSON_DATA]);
}

#[test]
fn json_text_and_bytes() {
    // A json literal takes values by their kind; a value json cannot hold is said where it is.
    let ok = "struct P { x: f32 }\non start {\n    let n: u32 = 3\n    let p = P { x: 1.0 }\n    let j = json.encode({\"n\": n, \"p\": p, \"l\": [n, 2]})\n    let back = json.decode<P>(data \"p.json\")\n    let raw = \"a\".bytes()\n    let i: int? = json.decode<json.Value>(j)?.get(\"n\")?.int()\n    log \"{j} {back?.x ?? 0.0} {raw.len()} {i ?? 0} \\u{48}\"\n}\n";
    assert_eq!(full(ok), vec![]);
    assert_eq!(said("on start {\n    let f = fn() { }\n    let j = json.encode({\"f\": f})\n}\n"), vec![code::NOT_JSON_DATA]);
    assert_eq!(said("on start {\n    let b = json.decode<int>(3)\n}\n"), vec![code::TYPE_MISMATCH]);
    // A game struct by value is no json data (its fields may be pointers); a game enum is.
    let on = |body: &str| format!("on start {{\n    if let a = MR::maybe() {{\n        {body}\n    }}\n}}\n");
    assert_eq!(said(&on("let b = a.mBox\n        let j = json.encode(b)\n        let k = json.encode([Mode::Walk])")), vec![code::NOT_JSON_DATA]);
    // `\u{...}` is an escape, not an interpolation, whatever its digits.
    assert_eq!(codes("on start {\n    log \"\\u{zz}\"\n}\n"), vec![(code::BAD_ESCAPE, 2, 9)]);
    // A `var` argument by name.
    assert_eq!(said("fn f(a: int = 1, var c: int) { c += a }\non start {\n    var c = 0\n    f(c: var c)\n}\n"), vec![]);
}

#[test]
fn a_second_on_in_another_module_names_the_first_ones_file() {
    let game = small_game();
    let facts = ModFacts::of_game(&game);
    let checked = check::check(&[source("src/main.ls", "on step { }
"), source("src/more.ls", "
on step { }
")], &[], facts);
    let said: Vec<_> = checked.diags.iter().filter(|(_, d)| d.code == code::ON_TWICE).collect();
    assert_eq!(said.len(), 1);
    let (_, d) = said[0];
    assert_eq!((d.at.line, d.at.col), (2, 1));
    assert_eq!(d.args, vec![("what", Arg::Text("step".into())), ("first", Arg::Text("src/main.ls:1".into()))]);
}

#[test]
fn keep_and_kept() {
    // D11.5 (reload G-RL5): keep in `on stop`, kept<T>() in `on start` before any wait, json data both ways.
    assert_eq!(heard("struct Count { n: i32 }
on stop { let ok = keep(Count { n: 3 }) }
on start {
    let c: Count? = kept<Count>()
    wait 1 frames
}
"), vec![]);
    assert_eq!(heard("fn f() { keep(1) }
"), vec![(code::KEEP_HERE, 1, 10, args(&[("what", "keep"), ("why", "P:Script.Keep.Stop")]))]);
    assert_eq!(heard("on frame { let x = kept<i32>() }
"), vec![(code::KEEP_HERE, 1, 20, args(&[("what", "kept"), ("why", "P:Script.Kept.Start")]))]);
    assert_eq!(heard("on start {
    wait 1 frames
    let x = kept<i32>()
}
"), vec![(code::KEEP_HERE, 3, 13, args(&[("what", "kept"), ("why", "P:Script.Kept.Start")]))]);
    assert_eq!(heard("on start { let x = kept() }
"), vec![(code::TYPE_ARG_COUNT, 1, 20, args(&[("found", "kept"), ("expected", "1")]))]);
    // A game class is no json data.
    assert_eq!(heard("on start { let x = kept<LiveActor>() }
"), vec![(code::NOT_JSON_DATA, 1, 20, args(&[("found", "LiveActor")]))]);
}

#[test]
fn marked_pointer_parameters() {
    // Parameter marks (docs/modding/language/compiled-form.md, section 7): out and inout are `var x`, in a value, a count's array a list whose length the
    // compiler passes, a fixed count's a list of exactly that many, nullable also `none`.
    assert_eq!(heard("on frame {
    var front = vec3(0.0, 0.0, 0.0)
    MR::getPlayerFrontVec(var front)
    let s: f32 = MR::sum([1.0, 2.0, 3.0])
    var xs: list<f32> = [0.0, 0.0]
    MR::fill(var xs)
    var row: list<f32> = []
    MR::readRow(var row)
    MR::nudge(var front)
    MR::nudge(none)
    MR::keepMtx(none)
}
"), vec![]);
    // Kept and nullable: none alone (the game keeps the pointer; no script place lives that long).
    assert!(!heard("on frame {
    var p = vec3(0.0, 0.0, 0.0)
    MR::keepMtx(var p)
}
").is_empty());
    // out without `var`; the count is not written.
    assert_eq!(heard("on frame {
    var front = vec3(0.0, 0.0, 0.0)
    MR::getPlayerFrontVec(front)
    let s = MR::sum([1.0], 1)
}
"), vec![
        (code::VAR_ARG_NEEDED, 3, 27, args(&[("found", "MR::getPlayerFrontVec"), ("param", "1")])),
        (code::ARG_COUNT, 4, 13, args(&[("name", "MR::sum"), ("found", "2"), ("expected", "1")])),
    ]);
    // Kept, a write through a const pointer, a count that is no integer, and an unmarked pointer: refused.
    assert_eq!(heard("on frame {
    var p = vec3(0.0, 0.0, 0.0)
    MR::keepPos(var p)
    MR::outConst(var p)
    MR::countF(var p, 1.0)
    MR::frontVec(var p)
}
"), vec![
        (code::GAME_PARAM_NOT_CROSSING, 3, 5, args(&[("found", "MR::keepPos"), ("param", "1"), ("why", "P:Script.Why.Kept")])),
        (code::GAME_PARAM_NOT_CROSSING, 4, 5, args(&[("found", "MR::outConst"), ("param", "1"), ("why", "P:Script.Why.MarkNotFit")])),
        (code::GAME_PARAM_NOT_CROSSING, 5, 5, args(&[("found", "MR::countF"), ("param", "2"), ("why", "P:Script.Why.MarkNotFit")])),
        (code::GAME_PARAM_NOT_CROSSING, 6, 5, args(&[("found", "MR::frontVec"), ("param", "1"), ("why", "P:Script.Why.PointerToValue")])),
    ]);
}

/// The docs/modding/language/classes.md's make (order 8): `make(name, place, count = 1) -> list<LiveActor>`, the place the standard
/// module's MakePlace (its fields' defaults as any struct's); a literal name is a factory row or a mod class's, said
/// only with the placement facts.
#[test]
fn make_takes_a_name_a_place_and_a_count() {
    let text = "class Bob : LiveActor made by Plain {
}

on frame {
    let a: list<LiveActor> = make(\"Kuribo\", MakePlace { at: vec3(1.0, 2.0, 3.0), args: [4] })
    var p = MakePlace { at: vec3(0.0, 0.0, 0.0), dead: true }
    p.scale = vec3(2.0, 2.0, 2.0)
    let b = make(\"bob\", p, 3)
    let n = \"Kuri\"
    let c = make(\"{n}bo\", p)
    log \"{a.len()} {b.len()} {c.len()} {p.rotation.y}\"
}
";
    assert_eq!(heard(text), vec![]);
    let mut game = small_game();
    game.factory = Some(vec![("Kuribo".into(), Some("Kuribo".into())), ("Plain".into(), Some("LiveActor".into()))]);
    let made = |text: &str| full_in(&game, text).into_iter().filter(|d| d.0 != code::NOT_DESCRIBED).collect::<Vec<_>>();
    assert_eq!(made(text), vec![]);
    // An unknown literal name; a place missing `at`; a field it lacks; a count no int; a third argument too many.
    assert_eq!(made("on frame {
    make(\"Nope\", MakePlace { at: vec3(0.0, 0.0, 0.0) })
    make(\"Kuribo\", MakePlace { })
    make(\"Kuribo\", MakePlace { at: vec3(0.0, 0.0, 0.0), turn: 1.0 }, 1.5)
    make(\"Kuribo\")
}
"), vec![
        (code::CLASS_NAME_UNKNOWN, 2, 10, args(&[("found", "Nope")])),
        (code::FIELD_MISSING, 3, 20, args(&[("name", "at"), ("found", "MakePlace")])),
        (code::NO_MEMBER, 4, 57, args(&[("name", "turn"), ("found", "MakePlace")])),
        (code::TYPE_MISMATCH, 4, 70, args(&[("found", "f32"), ("expected", "i32")])),
        (code::ARG_COUNT, 5, 5, args(&[("name", "make"), ("found", "1"), ("expected", "2, 3")])),
    ]);
    // A module's own MakePlace is its own (the standard one comes after it).
    assert_eq!(heard("struct MakePlace { x: i32 }

on frame {
    let p = MakePlace { x: 1 }
    log \"{p.x}\"
}
"), vec![]);
}

#[test]
fn nullable_where_nothing_else_crosses_takes_none() {
    // A kept address, and a `void *`, marked nullable: the script leaves it out.
    assert_eq!(heard("on frame {
    MR::keepMtx(none)
    let b = MR::calcHost(none, none)
}
"), vec![]);
    // Anything but `none`; `var`; an unmarked `void *`.
    assert_eq!(heard("on frame {
    var p = vec3(0.0, 0.0, 0.0)
    MR::keepMtx(p)
    MR::keepMtx(var p)
    MR::rawHost(none)
}
"), vec![
        (code::TYPE_MISMATCH, 3, 17, args(&[("found", "vec3"), ("expected", "none")])),
        (code::VAR_ARG_NOT_TAKEN, 4, 17, args(&[("found", "MR::keepMtx"), ("param", "1")])),
        (code::TYPE_MISMATCH, 4, 21, args(&[("found", "vec3"), ("expected", "none")])),
        (code::GAME_PARAM_NOT_CROSSING, 5, 5, args(&[("found", "MR::rawHost"), ("param", "1"), ("why", "P:Script.Why.VoidPointer")])),
    ]);
}

#[test]
fn a_fixed_counts_list_literal_is_that_many() {
    assert_eq!(heard("on frame {
    MR::setRow([1.0, 2.0, 3.0])
    MR::setRow(([1.0, 2.0]))
}
"), vec![(code::FIXED_COUNT, 3, 16, args(&[("found", "MR::setRow"), ("param", "1"), ("expected", "3"), ("count", "2")]))]);
}

#[test]
fn a_hidden_count_keeps_the_parameters_numbers() {
    // `void(u8 n, f32 *xs, Vec3f *out)`, xs counted by n: the script writes (var xs, var out); out is parameter 3.
    assert_eq!(heard("on frame {
    var xs: list<f32> = [0.0]
    var v = vec3(0.0, 0.0, 0.0)
    MR::countFirst(var xs, var v)
    MR::countFirst(var xs, v)
}
"), vec![(code::VAR_ARG_NEEDED, 5, 28, args(&[("found", "MR::countFirst"), ("param", "3")]))]);
}

/// `on frame`, `on start`, `on stop` and kit rule 4's `on mods.stopped` are one a module
/// (a kit module ticks itself), so two modules may each have them; a second in one module is S0109.
#[test]
fn frame_start_stop_and_mods_stopped_are_one_a_module() {
    let game = small_game();
    let each = "on frame { }\non start { }\non stop { }\non mods.stopped(gone) { }\n";
    let checked = check::check(&[source("src/main.ls", each), source("src/more.ls", each)], &[], ModFacts::of_game(&game));
    let said: Vec<_> = checked.diags.iter().filter(|(_, d)| d.code == code::ON_TWICE).collect();
    assert!(said.is_empty(), "{said:?}");
    let twice = "on frame { }\non mods.stopped(gone) { }\non frame { }\non mods.stopped(g: u64) { }\n";
    let checked = check::check(&[source("src/main.ls", twice)], &[], ModFacts::of_game(&game));
    let said: Vec<_> = checked.diags.iter().filter(|(_, d)| d.code == code::ON_TWICE).map(|(_, d)| d.at.line).collect();
    assert_eq!(said, vec![3, 4]);
}

/// Director 2026-11-12, decision (a): a record that cannot be the script's copy (`Sensor` copies itself, as the
/// game's HitSensor) written in a type is its view: a parameter, a function type's parameter, a game function's
/// pointer argument, its fields read. As every view it is not kept (language rule 3): a struct's field of it is S0103.
#[test]
fn a_record_that_cannot_be_copied_is_its_view() {
    let none: Vec<u16> = Vec::new();
    let text = "fn send(mine: Sensor, other: Sensor) -> bool {\n    return MR::sendTo(mine, other)\n}\n\nfn hit(a: LiveActor, then: fn(Sensor, Sensor) -> bool) -> bool {\n    guard let s = MR::sensorOf(a) else { return false }\n    return then(s, s) and s.mRadius > 0.0\n}\n\nfn fire(a: LiveActor) -> bool {\n    return hit(a, send)\n}\n";
    assert_eq!(said(text), none);
    assert_eq!(said("struct Hit {\n    sensor: Sensor\n    at: vec3\n}\n"), vec![code::VIEW_KEPT]);
}

/// `x as? T`: a view, or an optional one, of the same line of classes gives `T?`; a struct, a
/// number or a class of another line is S0069. `fail <text>` leaves the function as a script error does (no return
/// is missing after it) and takes text only.
#[test]
fn narrowing_and_fail() {
    let none: Vec<u16> = Vec::new();
    assert_eq!(said("fn take(k: Kuribo) -> int {\n    return 1\n}\n\nfn f(a: LiveActor?) -> int {\n    if let k = a as? Kuribo {\n        return take(k)\n    }\n    return 0\n}\n"), none);
    assert_eq!(said("fn f(k: Kuribo) -> bool {\n    let a = k as? LiveActor\n    return a != none\n}\n"), none);
    assert_eq!(said("fn f(a: LiveActor) -> bool {\n    let t = a as? Thing\n    return t != none\n}\n"), vec![code::AS_NUMBERS]);
    assert_eq!(said("fn f(n: int) -> bool {\n    let t = n as? Kuribo\n    return t != none\n}\n"), vec![code::AS_NUMBERS]);
    assert_eq!(said("fn f(n: int) -> int {\n    if n < 0 {\n        fail \"below zero: {n}\"\n    }\n    return n\n}\n"), none);
    assert_eq!(said("fn f() -> int {\n    fail \"never\"\n}\n"), none);
    // `fail` is a statement only before a text; what follows is one expression, which must be text.
    assert_eq!(said("fn f() {\n    fail \"a\" == \"b\"\n}\n"), vec![code::TYPE_MISMATCH]);
}
