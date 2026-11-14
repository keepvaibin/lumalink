//! The parser against the language's own programs (the worked programs, with ls_errata.json applied) and
//! against the cases it must refuse, each by its code and line (the parser's negative cases and probes).

use lumascript::diag::code;
use lumascript::{parse_module, parse_statements, Diagnostic};
use std::path::Path;

fn shown(name: &str, d: &[Diagnostic]) -> String {
    d.iter().map(|d| format!("{name}:{}:{} {} {:?}\n", d.at.line, d.at.col, d.row(), d.args)).collect()
}

#[test]
fn every_program_of_the_language_parses() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/programs");
    let modes = std::fs::read_to_string(folder.join("modes.txt")).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "lum"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 46, "the corpus has 46 programs");
    let mut bad = String::new();
    for f in &files {
        let name = f.file_name().unwrap().to_str().unwrap();
        let src = std::fs::read_to_string(f).unwrap();
        let statements = modes.lines().any(|m| m == name);
        // As written on Windows too: a line may end in CR LF.
        for src in [src.replace("\r\n", "\n"), src.replace("\r\n", "\n").replace('\n', "\r\n")] {
            let diags = if statements { parse_statements(&src).1 } else { parse_module(&src).1 };
            bad.push_str(&shown(name, &diags));
        }
    }
    assert!(bad.is_empty(), "{bad}");
}

/// (name, source, code, line); code 0: no error.
const CASES: &[(&str, &str, u16, u32)] = &[
    ("andand", "on start {\n    if x && y { log \"a\" }\n}\n", code::DOUBLED_OPERATOR, 2),
    ("bang", "on start {\n    if !x { log \"a\" }\n}\n", code::PREFIX_BANG, 2),
    ("bare", "draw.set_main_region(1)\n", code::UNEXPECTED, 1),
    ("chain", "on start {\n    let a = 1 == 2 == 3\n}\n", code::CHAINED_COMPARISON, 2),
    ("const", "const k = 1\n", code::UNEXPECTED, 1),
    ("wrapconv", "on start {\n    let s = wrap(n as u8)\n}\n", code::UNEXPECTED, 2),
    ("structcond", "on start {\n    if Point { x: 1 } == p { log \"a\" }\n}\n", code::STRUCT_IN_CONDITION, 2),
    ("structcond-module", "on start {\n    while geo.Point { x: 1 } != p { log \"a\" }\n}\n", code::STRUCT_IN_CONDITION, 2),
    ("structcond-paren", "on start {\n    if (Point { x: 1 }) == p { log \"a\" }\n}\n", 0, 0),
    ("block-after-capital", "on start {\n    if ready == Phase.Run { go Run }\n}\n", 0, 0),
    ("underscore-arm", "on start {\n    match n {\n        _ => log \"a\"\n    }\n}\n", code::UNDERSCORE_PATTERN, 3),
    ("bare-arm", "on start {\n    match n {\n        other => log \"a\"\n    }\n}\n", code::BARE_ARM_NAME, 3),
    (
        "bare-in-tuple-arm",
        "on start {\n    match (a, b) {\n        (0, rest) => log \"a\"\n        else => log \"b\"\n    }\n}\n",
        code::BARE_ARM_NAME,
        3,
    ),
    ("underscore-let", "on start {\n    let (_, b) = pair\n}\n", code::UNDERSCORE_NAME, 2),
    ("underscore-param", "fn f(_: int) { }\n", code::UNDERSCORE_NAME, 1),
    (
        "case-arms",
        "on start {\n    match s {\n        Shape.Ball(r) => log \"{r}\"\n        Shape.Box { w, h } => log \"{w}\"\n        (Shape.Dot, 0) => log \"d\"\n        else => log \"x\"\n    }\n}\n",
        0,
        0,
    ),
    ("if-let-case", "on start {\n    if let Shape.Ball(r) = s { log \"{r}\" }\n    let (a, b) = pair\n}\n", 0, 0),
    (
        "good-mix",
        "let k = low_bits<u16>(70000)\non start {\n    draw.tint(a, multiply: #FFFFFF, add: c)\n    if let p = MarioAccess::getPlayerActor() {\n        p.mVelocity = vec3(0.0, 1.0, 0.0)\n    }\n    let v = x!.get() ?? none\n    match n {\n        0 => log \"zero\"\n        -1 => log \"neg\"\n        else => log \"other\"\n    }\n}\nhook after MR::isInWater(at: vec3) -> inside {\n    result = inside\n}\n",
        0,
        0,
    ),
    // The lexer's own.
    ("no-closing-quote", "on start {\n    log \"a\n}\n", code::NO_CLOSING_QUOTE, 2),
    ("no-interpolation-end", "on start {\n    log \"a {b\"\n}\n", code::NO_INTERPOLATION_END, 2),
    ("no-comment-end", "on start { }\n/* open\n", code::NO_COMMENT_END, 2),
    ("not-a-token", "on start {\n    let a = 1 $ 2\n}\n", code::NOT_A_TOKEN, 2),
    ("no-closing-backquote", "let `a = 1\n", code::NO_CLOSING_BACKQUOTE, 1),
    // The parser's other rules.
    ("content-interpolates", "on start {\n    let m = model \"a/{n}\"\n}\n", code::CONTENT_INTERPOLATES, 2),
    ("item-in-block", "on start {\n    hook before X::y() { }\n}\n", code::ITEM_IN_BLOCK, 2),
    ("positional-after-named", "on start {\n    f(a: 1, 2)\n}\n", code::POSITIONAL_AFTER_NAMED, 2),
    ("self-not-first", "struct P {\n    fn f(a: int, self) { }\n}\n", code::SELF_NOT_FIRST, 2),
    ("no-start-state", "states S for C {\n    state A { }\n}\n", code::ONE_START_STATE, 3),
    ("two-start-states", "states S for C {\n    state A { }\n    start A\n    start A\n}\n", code::ONE_START_STATE, 4),
    ("type-args-before-equals", "let a: list<int>= []\nlet b: map<text, list<int>>= {}\n", 0, 0),
    ("enum-field-sep", "enum S {\n    Ball(r: f32 s: f32)\n}\n", code::UNEXPECTED, 2),
    ("use-forms", "use ./ui/parts.{Bar, Gauge}\nuse lumakit.player.{form} as pf\nuse optional net.socket as s\nuse c \"x.h\"\n", 0, 0),
    ("nested-types", "let m: map<text, list<int>> = {}\nlet d = json.decode<map<text, list<int>>>(s)\n", 0, 0),
    ("less-than-is-a-comparison", "on start {\n    let a = b.c < d\n    let e = f.g < h > (i)\n}\n", 0, 0),
    // only a block's `}` lets the next statement follow on its line.
    ("block-then-statement", "on start {\n    if a { log \"a\" } log \"b\"\n}\n", 0, 0),
    ("literal-is-no-block", "on start {\n    let p = P { x: 1 } let q = 2\n}\n", code::UNEXPECTED, 2),
    ("map-is-no-block", "on start {\n    let m = {1: 2} log \"a\"\n}\n", code::UNEXPECTED, 2),
    // G.3, G.4: a comma list has no comma before its closer; G.2: a hook's list is names and types only.
    ("trailing-comma-args", "on start {\n    f(1, 2,)\n}\n", code::UNEXPECTED, 2),
    ("trailing-comma-list", "let a = [1, 2,]\n", code::UNEXPECTED, 1),
    ("trailing-comma-params", "fn f(a: int,) { }\n", code::UNEXPECTED, 1),
    ("trailing-comma-fn-type", "let f: fn(int,) = g\n", code::UNEXPECTED, 1),
    ("hook-param-default", "hook before MR::f(a = 1) { }\n", code::UNEXPECTED, 1),
    ("hook-param-var", "wrap MR::f(var a) { }\n", code::UNEXPECTED, 1),
    ("sep-list-trailing", "struct P {\n    x: int,\n    y: int,\n}\nlet m = {1: 2,}\n", 0, 0),
    ("pub-c-struct", "pub c struct P {\n    x: u32\n}\n", 0, 0),
    ("module-case-fields", "on start {\n    match s {\n        geo.Shape.Box { w } => log \"{w}\"\n        else => log \"x\"\n    }\n}\n", 0, 0),
    ("on-module-named-stage", "on stage.entered(name) { }\non stage \"EggStarGalaxy\" { }\n", 0, 0),
    ("log-fact-forms", "on start {\n    log fact \"lib_ready\" 1\n    log fact\n    log fact + 1\n    log fact.n\n}\n", 0, 0),
    ("test-in-block", "on start {\n    test \"a\" { }\n}\n", code::ITEM_IN_BLOCK, 2),
    ("state-block-then-wait", "states S for C {\n    state A {\n        enter { } wait 180 frames\n    }\n    start A\n}\n", 0, 0),
    // a type ending in `>` or `>>` ends its line.
    ("generic-field-line-end", "struct S {\n    items: list<int>\n    count: int\n}\n", 0, 0),
    ("ref-field-line-end", "struct S {\n    target: ref<Actor>\n    n: int\n}\n", 0, 0),
    ("nested-generic-line-end", "on start {\n    var m: map<text, list<int>>\n    log \"a\"\n}\n", 0, 0),
    ("var-generic-no-value", "on start {\n    var xs: list<int>\n    log \"a\"\n}\n", 0, 0),
    ("as-generic-line-end", "on start {\n    let v = y as list<int>\n    log \"a\"\n}\n", 0, 0),
    ("generic-param-lines", "fn f(\n    a: list<int>,\n    b: int\n) -> list<int> {\n    return a\n}\n", 0, 0),
    ("gt-shift-line-end", "on start {\n    let ok = a >\n        b\n    let s = a >>\n        b\n}\n", 0, 0),
    // A42R-1: a call's type arguments end a line and its `(` starts the next.
    ("type-args-line-end", "on start {\n    let d = json.decode<list<int>>\n        (data)\n}\n", code::TYPE_ARGS_LINE_END, 3),
    ("type-args-line-end-fn", "on start {\n    let d = make<int>\n        (1)\n}\n", code::TYPE_ARGS_LINE_END, 3),
    ("compare-shift-line-end", "on start {\n    let ok = a < b >>\n        (c)\n}\n", 0, 0),
    // an interpolation, and a string inside one, end with their line.
    ("interp-across-lines", "on start {\n    log \"a {x\n    + y}\"\n}\n", code::NO_INTERPOLATION_END, 2),
    ("interp-inner-string-across-lines", "on start {\n    log \"a {f(\"b)}\n    log 1\n}\n", code::NO_INTERPOLATION_END, 2),
    ("interp-unclosed-then-lines", "on start {\n    log \"count {n\"\n    let t = \"x\"\n}\n", code::NO_INTERPOLATION_END, 2),
    // a leading byte-order mark is skipped; a lone CR ends a line.
    ("bom", "\u{feff}on start {\n    log \"a\"\n}\n", 0, 0),
    ("lone-cr", "on start {\r    log \"a\"\r    let b = @\r}\r", code::NOT_A_TOKEN, 3),
    // names are ASCII, said once for the whole word.
    ("non-ascii-name", "on start {\n    let caf\u{e9} = 1\n}\n", code::NON_ASCII_NAME, 2),
    ("tuple-field-range", "on start {\n    let a = p.0.1\n    let r = 1..5\n}\n", 0, 0),
    ("empty-file", "", 0, 0),
];

/// The probes that give more than a code and a line, in full.
#[test]
fn f_probes_say_what_they_should() {
    let said = |src: &str| -> Vec<(u16, u32, u32)> { parse_module(src).1.iter().map(|d| (d.code, d.at.line, d.at.col)).collect() };
    assert_eq!(said("on start {\n    let caf\u{e9} = 1\n}\n"), [(code::NON_ASCII_NAME, 2, 9)]);
    // A tab is one column.
    assert_eq!(said("on start {\n\tlet x = @\n}\n")[0], (code::NOT_A_TOKEN, 2, 10));
    let (_, d) = parse_statements("let a = 1\nlet b = a +\n");
    assert_eq!(d.iter().map(|d| (d.code, d.at.line)).collect::<Vec<_>>(), [(code::UNEXPECTED, 3)]);
    // An interpolation broken over two lines: said at its brace, and never two errors at one place.
    let d = said("on start {\n    log \"a {x\n    + y}\"\n}\n");
    assert_eq!(d[0], (code::NO_INTERPOLATION_END, 2, 12));
    let mut places: Vec<_> = d.iter().map(|&(_, l, c)| (l, c)).collect();
    places.dedup();
    assert_eq!(places.len(), d.len(), "{d:?}");
}

/// `>` and `>>` at a line end: the next line goes on with them as operators.
#[test]
fn a_line_end_after_greater_than_goes_on() {
    use lumascript::ast::{BinOp, ExprKind, ItemKind, StmtKind};
    let (m, d) = parse_module("on start {\n    let ok = a >\n        b\n    let s = a >>\n        b\n}\n");
    assert!(d.is_empty(), "{}", shown("gt", &d));
    let ItemKind::On(on) = &m.items[0].kind else { panic!("an on item") };
    let ops: Vec<_> = on
        .body
        .stmts
        .iter()
        .map(|s| match &s.kind {
            StmtKind::Binding(b) => match b.value.as_ref().map(|v| &v.kind) {
                Some(ExprKind::Binary { op, .. }) => Some(*op),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(ops, [Some(BinOp::Gt), Some(BinOp::Shr)]);
}

#[test]
fn each_refusal_is_said_with_its_code_and_line() {
    let mut bad = String::new();
    for &(name, src, want, line) in CASES {
        let (_, diags) = parse_module(src);
        for d in &diags {
            let names: Vec<_> = d.args.iter().map(|(n, _)| *n).collect();
            if names != lumascript::diag::args_of(d.code) {
                bad.push_str(&format!("{name}: S{:04} gives {names:?}, its row fills {:?}\n", d.code, lumascript::diag::args_of(d.code)));
            }
            for (_, a) in &d.args {
                if let lumascript::Arg::Phrase(p) = a {
                    if !lumascript::diag::PHRASES.contains(p) {
                        bad.push_str(&format!("{name}: {p} is not in PHRASES\n"));
                    }
                }
            }
        }
        let first = diags.first().map(|d| (d.code, d.at.line));
        let expected = (want != 0).then_some((want, line));
        if first != expected {
            bad.push_str(&format!("{name}: expected {expected:?}, got\n{}", shown(name, &diags)));
        }
    }
    assert!(bad.is_empty(), "{bad}");
}

/// A42R-1: type arguments at a line end with the call's `(` on the next line are said once, as that, and read as the
/// call they mean, so nothing after it is said for them.
#[test]
fn type_arguments_at_a_line_end_are_said_as_such() {
    use lumascript::ast::{ExprKind, ItemKind, StmtKind};
    let (m, d) = parse_module("on start {\n    let d = json.decode<list<int>>\n        (data)\n    log \"after\"\n}\n");
    let got: Vec<_> = d.iter().map(|d| (d.code, d.at.line, d.at.col)).collect();
    assert_eq!(got, [(code::TYPE_ARGS_LINE_END, 3, 9)], "{}", shown("targs", &d));
    let ItemKind::On(on) = &m.items[0].kind else { panic!("an on item") };
    assert_eq!(on.body.stmts.len(), 2);
    let StmtKind::Binding(b) = &on.body.stmts[0].kind else { panic!("a binding") };
    let Some(ExprKind::Call { callee, args }) = b.value.as_ref().map(|v| &v.kind) else { panic!("a call") };
    assert!(matches!(&callee.kind, ExprKind::Member { targs, .. } if targs.len() == 1));
    assert_eq!(args.len(), 1);
}

/// One mistake is said once, and the parser goes on to say the next one in the file.
#[test]
fn errors_after_the_first_are_still_found() {
    let src = "on start {\n    let a = 1 == 2 == 3\n    let b = (\n    log \"fine\"\n}\nfn f(_: int) { }\nlet c = 2\n";
    let (module, diags) = parse_module(src);
    let got: Vec<_> = diags.iter().map(|d| (d.code, d.at.line)).collect();
    assert_eq!(got.first(), Some(&(code::CHAINED_COMPARISON, 2)), "{}", shown("recover", &diags));
    assert!(got.contains(&(code::UNDERSCORE_NAME, 6)), "{}", shown("recover", &diags));
    assert!(got.iter().filter(|(_, l)| *l == 2).count() == 1, "{}", shown("recover", &diags));
    assert!(module.items.len() >= 2, "the items after the errors are kept");
}

#[test]
fn every_code_is_a_known_row() {
    for c in code::ALL {
        assert!(*c >= 1 && *c < 900, "S{c:04} is a build-time code");
    }
    let mut sorted = code::ALL.to_vec();
    sorted.dedup();
    assert_eq!(sorted.len(), code::ALL.len());
}

/// What the parser builds, where the grammar has two readings.
#[test]
fn the_tree_takes_the_grammars_reading() {
    use lumascript::ast::{HookRet, ItemKind, OnEvent, TypeKind};
    let (m, d) = parse_module("hook after MR::getStarPieceNum() -> n {\n    result = n * 2\n}\nhook before MR::isNear(a, b, c) -> bool {\n    skip false\n}\nhook after MR::f() -> list<int> { }\n");
    assert!(d.is_empty(), "{}", shown("hooks", &d));
    let rets: Vec<_> = m
        .items
        .iter()
        .map(|i| match &i.kind {
            ItemKind::Hook(h) => match &h.ret {
                Some(HookRet::Name(n)) => format!("name {}", n.name),
                Some(HookRet::Type(t)) => match &t.kind {
                    TypeKind::Named { parts, args, .. } => format!("type {} {}", parts[0].name, args.len()),
                    _ => "type other".into(),
                },
                None => "none".into(),
            },
            _ => "not a hook".into(),
        })
        .collect();
    assert_eq!(rets, ["name n", "type bool 0", "type list 1"]);

    let (m, d) = parse_module("use draw as d\non draw d.Point.Hud { }\non draw.resized { }\n");
    assert!(d.is_empty(), "{}", shown("draw", &d));
    match (&m.items[1].kind, &m.items[2].kind) {
        (ItemKind::On(a), ItemKind::On(b)) => {
            assert!(matches!(&a.event, OnEvent::Draw(p) if p.len() == 3 && p[0].name == "d"));
            assert!(matches!(&b.event, OnEvent::Event { path, names: None, .. } if path.len() == 2));
        }
        _ => panic!("two on items"),
    }

    // `(T)` is T grouped, not a tuple of one: `(fn(A) -> bool)?` is an optional function (FB, FC 2026-11-12).
    let (m, d) = parse_module("fn f(g: (fn(i32) -> bool)?, h: (i32), t: (i32, f32)) {\n}\n");
    assert!(d.is_empty(), "{}", shown("grouped", &d));
    let ItemKind::Fn(f) = &m.items[0].kind else { panic!("a fn") };
    let ty = |i: usize| f.params[i].ty.as_ref().expect("a type");
    assert!(matches!(&ty(0).kind, TypeKind::Fn { params, ret: Some(_) } if params.len() == 1) && ty(0).optional == 1);
    assert!(matches!(&ty(1).kind, TypeKind::Named { parts, .. } if parts[0].name == "i32") && ty(1).optional == 0);
    assert!(matches!(&ty(2).kind, TypeKind::Tuple(items) if items.len() == 2));
}

/// An error inside an interpolation is said at its place in the file.
#[test]
fn an_interpolation_is_checked_where_it_stands() {
    let (_, d) = parse_module("on start {\n    log \"a {1 == 2 == 3} b\"\n}\n");
    let got: Vec<_> = d.iter().map(|d| (d.code, d.at.line, d.at.col)).collect();
    assert_eq!(got.len(), 1, "{}", shown("interp", &d));
    assert_eq!((got[0].0, got[0].1), (code::CHAINED_COMPARISON, 2));
    assert!(got[0].2 > 10, "the column is inside the string");
}
