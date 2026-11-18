//! Nesting: every form of nesting is taken up to the limit on the compiler's own stack in this build
//! (a debug build under `cargo test`), refused one past it with S0022, and refused, never a crash, at 100 000; called
//! from a thread of 1 MB, as a Windows main thread has.

use lumascript::diag::code;
use lumascript::{parse_module, Diagnostic, MAX_DEPTH};

/// Each form, and the deepest of it the parser takes (MAX_DEPTH less the levels around it: the `on` block, the `let`;
/// a block in a block is two levels, its `if` and its braces).
const FORMS: &[(&str, usize)] = &[
    ("paren", 510),
    ("neg", 510),
    ("not", 510),
    ("list", 511),
    ("block", 255),
    ("type", 510),
    ("sum", 510),
    ("coalesce", 510),
    ("call", 510),
    ("member", 510),
    ("pattern", 510),
    ("defer", 510),
    ("as", 509),
    ("interp", 510),
];

fn source(form: &str, n: usize) -> String {
    let body = match form {
        "paren" => format!("let x = {}1{}", "(".repeat(n), ")".repeat(n)),
        "neg" => format!("let x = {}1", "- ".repeat(n)),
        "not" => format!("let x = {}y", "not ".repeat(n)),
        "list" => format!("let x = {}{}", "[".repeat(n), "]".repeat(n)),
        "block" => format!("{}log 1\n{}", "if x {\n".repeat(n), "}\n".repeat(n)),
        "type" => format!("var x: {}int{}", "list<".repeat(n), ">".repeat(n)),
        "sum" => format!("let x = {}1", "1 + ".repeat(n)),
        "coalesce" => format!("let x = {}b", "a ?? ".repeat(n)),
        "call" => format!("let x = f{}", "()".repeat(n)),
        "member" => format!("let x = a{}", ".b".repeat(n)),
        "pattern" => format!("let {}a{} = x", "(".repeat(n), ")".repeat(n)),
        "defer" => format!("{}log 1", "defer ".repeat(n)),
        "as" => format!("let x = y{}", " as int".repeat(n)),
        "interp" => {
            let mut s = String::from("x");
            for _ in 0..n {
                s = format!("\"{{{s}}}\"");
            }
            format!("log {s}")
        }
        _ => unreachable!(),
    };
    format!("on start {{\n{body}\n}}\nlet after = 1\n")
}

/// Parses on a thread of 1 MB (the compiler takes its own stack) and gives the diagnostics.
fn parse_on_small_stack(src: String) -> Vec<Diagnostic> {
    std::thread::Builder::new().stack_size(1 << 20).spawn(move || parse_module(&src).1).unwrap().join().unwrap()
}

#[test]
fn each_form_is_taken_to_the_limit_and_refused_past_it() {
    assert_eq!(MAX_DEPTH, 512, "FORMS is worked out for 512");
    let mut bad = String::new();
    for &(form, deepest) in FORMS {
        for (n, refusals) in [(deepest, 0), (deepest + 1, 1), (100_000, 1)] {
            let d = parse_on_small_stack(source(form, n));
            let too_deep: Vec<_> = d.iter().filter(|d| d.code == code::TOO_DEEP).collect();
            // Only the one refusal, and the parse goes on to what follows (no error at `let after`).
            if too_deep.len() != refusals || d.len() != refusals {
                bad.push_str(&format!("{form} {n}: {d:?}\n"));
            }
            for t in too_deep {
                if t.args != [("limit", lumascript::Arg::Text(MAX_DEPTH.to_string()))] {
                    bad.push_str(&format!("{form} {n}: {:?}\n", t.args));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{bad}");
}

/// Brackets that never close and closers with no opener: followed in linear time by the lexer and by the recovery (a
/// quadratic one would take minutes here), each mistake said.
#[test]
fn unmatched_brackets_stay_linear() {
    for src in [
        format!("on start {{\n    let x = {}{}\n}}\n", "[".repeat(100_000), ")".repeat(100_000)),
        format!("on start {{\n    let x = 1{}\n    let y = {}\n}}\n", ")".repeat(100_000), "(".repeat(100_000)),
        format!("on start {{\n    let x = {}1\n}}\n", "{(".repeat(50_000)),
    ] {
        let d = parse_on_small_stack(src);
        assert!(!d.is_empty() && d.len() < 10, "{d:?}");
    }
}

/// A long run of line ends (blank lines, before an `else` and before anything else) is read in linear time: the lexer
/// looked past every line end for an `else` from each one, so 200,000 blank lines took minutes.
#[test]
fn blank_lines_stay_linear() {
    let blank = "\n".repeat(200_000);
    let src = format!("on start {{\n    if a {{\n        log \"a\"\n    }}{blank}    else {{\n        log \"b\"\n    }}{blank}    log \"c\"\n}}{blank}");
    let start = std::time::Instant::now();
    let (module, d) = parse_module(&src);
    let took = start.elapsed();
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(module.items.len(), 1);
    // Linear, this is a few milliseconds in a debug build; the quadratic look took minutes.
    assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
}

/// Many unknown names in a module of many items: each is said with its nearest known names, found in time that does
/// not grow with the product of the two (each one listed, sorted and compared with every known name took minutes).
#[test]
fn unknown_names_stay_linear() {
    use lumascript::check::{check, ModFacts, Source};
    const N: usize = 20_000;
    let mut src = String::new();
    for i in 0..N {
        src.push_str(&format!("fn known_{i}() {{ }}\n"));
    }
    src.push_str("on start {\n");
    for i in 0..N {
        src.push_str(&format!("    unknown_{i}()\n"));
    }
    src.push_str("}\n");
    let (module, parsed) = parse_module(&src);
    assert!(parsed.is_empty(), "{parsed:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let game = lumascript::game::Game::new(Default::default(), None);
    let start = std::time::Instant::now();
    let said = lumascript::on_compiler_stack(|| check(&sources, &[], ModFacts::of_game(&game)).diags.len()).unwrap();
    let took = start.elapsed();
    assert_eq!(said, N);
    assert!(took < std::time::Duration::from_secs(10), "took {took:?}");
}

/// A long chain of structs, each also holding the first by value: every cycle is said, each with a chain of bounded
/// length, in linear time (each cycle kept whole to be said once, and said with every name, took memory and time with
/// the square of the chain: N-4029).
#[test]
fn value_cycles_stay_linear() {
    use lumascript::check::{check, ModFacts, Source};
    const N: usize = 10_000;
    let mut src = String::new();
    for i in 0..N - 1 {
        src.push_str(&format!("struct S{i} {{\n    n: S{}\n    z: S0\n}}\n", i + 1));
    }
    src.push_str(&format!("struct S{} {{\n    z: S0\n}}\n", N - 1));
    let (module, parsed) = parse_module(&src);
    assert!(parsed.is_empty(), "{parsed:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let game = lumascript::game::Game::new(Default::default(), None);
    let start = std::time::Instant::now();
    let diags = lumascript::on_compiler_stack(|| check(&sources, &[], ModFacts::of_game(&game)).diags).unwrap();
    let took = start.elapsed();
    let cycles: Vec<&Diagnostic> = diags.iter().map(|(_, d)| d).filter(|d| d.code == code::VALUE_CYCLE).collect();
    // S0's own `z` closes S0 -> S0; each S{i}.z closes S0 -> ... -> S{i} -> S0.
    assert_eq!(cycles.len(), N);
    let longest = cycles.iter().flat_map(|d| d.args.iter()).map(|(_, a)| format!("{a:?}").len()).max().unwrap_or(0);
    assert!(longest < 400, "a chain of {longest} characters");
    let last = cycles.iter().find(|d| format!("{:?}", d.args).contains(&format!("S{}", N - 1))).expect("the longest cycle");
    assert_eq!(last.args, [("chain", lumascript::Arg::Text(format!("S0 -> S1 -> S2 -> S3 -> S4 -> S5 -> S6 -> S7 -> ... -> S{} -> S{} -> S{} -> S{} -> S{} -> S{} -> S{} -> S0", N - 7, N - 6, N - 5, N - 4, N - 3, N - 2, N - 1)))]);
    assert!(took < std::time::Duration::from_secs(10), "took {took:?}");
}

/// A large module with no mistake: `n` structs and functions that call each other, with locals, loops, branches and
/// struct literals.
fn large_module(n: usize) -> String {
    let mut src = String::from("fn f0(x: i32, s: S0) -> i32 {\n    return x + s.a\n}\nstruct S0 {\n    a: i32\n    b: vec3\n}\n");
    for i in 1..n {
        let p = i - 1;
        src.push_str(&format!(
            "struct S{i} {{\n    a: i32\n    b: vec3\n}}\nfn f{i}(x: i32, s: S{i}) -> i32 {{\n    var t = x + s.a\n    for k in 0..3 {{\n        t += k\n    }}\n    if t > 3 {{\n        return t\n    }} else {{\n        return f{p}(t, S{p} {{ a: 1, b: vec3(0.0, 0.0, 0.0) }})\n    }}\n}}\n"
        ));
    }
    src.push_str(&format!("on start {{\n    log f{}(1, S{} {{ a: 2, b: vec3(1.0, 2.0, 3.0) }})\n}}\n", n - 1, n - 1));
    src
}

/// A large module is checked and built in time that grows with it, not with its square (M13: N-4026 and N-4027 were
/// two such places; this holds the rest).
#[test]
fn a_large_module_checks_and_builds_in_linear_time() {
    use lumascript::check::{check, ModFacts, Source};
    let game = lumascript::game::Game::new(Default::default(), None);
    let mut took = Vec::new();
    for n in [2_000usize, 8_000] {
        let (module, parsed) = parse_module(&large_module(n));
        assert!(parsed.is_empty(), "{parsed:?}");
        let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
        let start = std::time::Instant::now();
        let built = lumascript::on_compiler_stack(|| {
            let checked = check(&sources, &[], ModFacts::of_game(&game));
            assert!(checked.diags.iter().all(|(_, d)| lumascript::diag::is_note(d.code)), "{:?}", checked.diags);
            lumascript::emit::emit("large", &sources, &checked, &game).is_ok()
        })
        .unwrap();
        took.push(start.elapsed());
        assert!(built);
    }
    eprintln!("2000: {:?}, 8000: {:?}", took[0], took[1]);
    // Four times the module: linear is about four times the time; a square, sixteen.
    assert!(took[1] < took[0] * 8, "{took:?}");
}

/// The tree given back is held to the limit too, so dropping it (or walking it) on a small stack is safe.
#[test]
fn the_tree_given_back_is_no_deeper_than_the_limit() {
    for &(form, deepest) in FORMS {
        let src = source(form, deepest);
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let (module, d) = parse_module(&src);
                assert!(d.is_empty(), "{form}: {d:?}");
                drop(module);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
