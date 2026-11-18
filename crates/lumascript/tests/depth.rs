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

/// Checks a body and gives the time and the mistakes (notes left out).
fn check_timed(src: &str) -> (std::time::Duration, Vec<Diagnostic>) {
    use lumascript::check::{check, ModFacts, Source};
    let (module, parsed) = parse_module(src);
    assert!(parsed.is_empty(), "{parsed:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let game = lumascript::game::Game::new(Default::default(), None);
    let start = std::time::Instant::now();
    let diags = lumascript::on_compiler_stack(|| check(&sources, &[], ModFacts::of_game(&game)).diags).unwrap();
    let took = start.elapsed();
    (took, diags.into_iter().map(|(_, d)| d).filter(|d| !lumascript::diag::is_note(d.code)).collect())
}

/// A body of many branches that wait, and of many locals each read: checked in time that grows with it (the waits of
/// two branches were joined by comparing each with each, and each read looked through every local in scope for the
/// body that holds it: N-4030).
#[test]
fn bodies_stay_linear() {
    let mut took = Vec::new();
    for n in [1_000usize, 4_000] {
        let mut src = String::from("var c = true\n\ntask waits() {\n");
        for _ in 0..n {
            src.push_str("    if c {\n        wait 1 frames\n    } else {\n        wait 2 frames\n    }\n");
        }
        src.push_str("}\n\ntask reads() {\n");
        for i in 0..n {
            src.push_str(&format!("    let x{i} = {i}\n"));
        }
        for i in 0..n {
            src.push_str(&format!("    log x{i}\n"));
        }
        src.push_str("}\n\non start {\n    waits()\n    reads()\n}\n");
        let (t, said) = check_timed(&src);
        assert!(said.is_empty(), "{said:?}");
        took.push(t);
    }
    eprintln!("1000: {:?}, 4000: {:?}", took[0], took[1]);
    // Four times the body: linear is about four times the time; a square, sixteen; a cube, sixty-four.
    assert!(took[1] < took[0] * 8, "{took:?}");
}

/// A body of 200,000 waits: its list of waits is let go of one by one, not by a recursion as deep as the list.
#[test]
fn a_long_list_of_waits_is_let_go_of() {
    let src = format!("task waits() {{\n{}}}\n\non start {{\n    waits()\n}}\n", "    wait 1 frames\n".repeat(200_000));
    let (_, said) = check_timed(&src);
    assert!(said.is_empty(), "{said:?}");
}

/// A long chain of module values, each's type found from the next's: checked in linear time, not a crash of the
/// compiler's stack (N-4032: each was found inside the walk of the one before).
#[test]
fn a_long_chain_of_module_values_is_checked() {
    let mut took = Vec::new();
    for (n, wrap) in [(25_000usize, 0usize), (100_000, 0), (2_000, 500)] {
        let mut src = String::new();
        for i in 0..n {
            src.push_str(&format!("let a{i} = {}a{}{}
", "(".repeat(wrap), i + 1, ")".repeat(wrap)));
        }
        src.push_str(&format!("let a{n} = 1
on start {{
    log a0
}}
"));
        let (t, said) = check_timed(&src);
        assert!(said.is_empty(), "{n}: {:?}", &said[..said.len().min(3)]);
        took.push(t);
    }
    eprintln!("25000: {:?}, 100000: {:?}", took[0], took[1]);
    assert!(took[1] < took[0] * 8, "{took:?}");
    // A cycle met on the work list (b and c, found while a is) is said once, at the read that closes it, as the
    // recursion said it.
    let (_, said) = check_timed("let a = b\nlet b = c\nlet c = b\non start {\n    log a\n}\n");
    assert_eq!(said.iter().map(|d| (d.code, d.at.line, d.args.clone())).collect::<Vec<_>>(), [(code::TYPE_NEEDS_WRITING, 3, vec![("found", lumascript::Arg::Text("b".into()))])]);
}

/// A match on a case of `n` bools with an arm for each row of `rows` (a field's value, or None for a name that takes
/// any), and `else` when asked.
fn bool_match(n: usize, rows: &[Vec<Option<bool>>], with_else: bool) -> String {
    let fields: Vec<String> = (0..n).map(|i| format!("f{i}: bool")).collect();
    let mut src = format!("enum W {{ C({}) }}\n\nfn f(w: W) -> int {{\n    match w {{\n", fields.join(", "));
    for (i, row) in rows.iter().enumerate() {
        let cells: Vec<String> = row.iter().enumerate().map(|(k, v)| v.map_or(format!("x{k}"), |b| b.to_string())).collect();
        src.push_str(&format!("        W.C({}) => return {i}\n", cells.join(", ")));
    }
    if with_else {
        src.push_str("        else => return -1\n");
    }
    src.push_str("    }\n}\n\non start {\n    log f(W.C(");
    src.push_str(&vec!["true"; n].join(", "));
    src.push_str("))\n}\n");
    src
}

/// An arm for each value of each field (every value covered): the coverage walk took time doubling with each field
/// (N-4034: 11 fields 42 ms, 22 fields 41 s).
fn wide_rows(n: usize) -> Vec<Vec<Option<bool>>> {
    let mut rows = Vec::new();
    for i in 0..n {
        for v in [true, false] {
            rows.push((0..n).map(|k| (k == i).then_some(v)).collect());
        }
    }
    rows
}

/// Whether arms cover every value is as hard as whether a formula always holds: the pigeonhole rows (`holes` + 1
/// pigeons, a field for each pigeon and hole; an arm for a pigeon in no hole, and one for each two pigeons in one
/// hole) cover every value, and any walk that tries fields one at a time takes time growing faster than any power.
fn pigeon_rows(holes: usize) -> (usize, Vec<Vec<Option<bool>>>) {
    let pigeons = holes + 1;
    let n = pigeons * holes;
    let at = |p: usize, h: usize| p * holes + h;
    let mut rows = Vec::new();
    for p in 0..pigeons {
        let mut row = vec![None; n];
        for h in 0..holes {
            row[at(p, h)] = Some(false);
        }
        rows.push(row);
    }
    for h in 0..holes {
        for p in 0..pigeons {
            for q in p + 1..pigeons {
                let mut row = vec![None; n];
                row[at(p, h)] = Some(true);
                row[at(q, h)] = Some(true);
                rows.push(row);
            }
        }
    }
    (n, rows)
}

#[test]
fn a_wide_match_is_checked_in_bounded_time() {
    let mut took = Vec::new();
    for n in [11usize, 22, 200] {
        let (t, said) = check_timed(&bool_match(n, &wide_rows(n), false));
        assert!(said.is_empty(), "{n}: {said:?}");
        took.push(t);
    }
    eprintln!("11, 22, 200 fields: {took:?}");
    assert!(took[1] < std::time::Duration::from_secs(1), "{took:?}");
    // One value left out is still found: only the arms for a field true leave out every field false.
    let rows: Vec<_> = wide_rows(22).into_iter().filter(|r| r.contains(&Some(true))).collect();
    let (_, said) = check_timed(&bool_match(22, &rows, false));
    assert_eq!(said.iter().map(|d| d.code).collect::<Vec<_>>(), [code::MATCH_MISSING], "{said:?}");
    // A match too hard to check in its budget asks for `else`, in bounded time; with `else` nothing is said.
    let (n, rows) = pigeon_rows(6);
    let (t, said) = check_timed(&bool_match(n, &rows, false));
    eprintln!("pigeons: {t:?}");
    assert_eq!(said.iter().map(|d| (d.code, d.at.line)).collect::<Vec<_>>(), [(code::MATCH_TOO_LARGE, 4)], "{said:?}");
    assert!(t < std::time::Duration::from_secs(10), "{t:?}");
    let (_, said) = check_timed(&bool_match(n, &rows, true));
    assert!(said.is_empty(), "{said:?}");
}

/// Wide items, each member used: a struct of `n` fields and `n` functions, an enum of `n` cases and a case of `n`
/// fields, a function of `n` parameters called by their names, a match of an arm for each case.
fn wide_items(n: usize) -> String {
    let list = |f: &dyn Fn(usize) -> String, sep: &str| (0..n).map(f).collect::<Vec<_>>().join(sep);
    let mut src = String::new();
    src.push_str(&format!("struct S {{\n{}\n", list(&|i| format!("    f{i}: int"), "\n")));
    src.push_str(&format!("{}\n}}\n\n", list(&|i| format!("    fn m{i}(self) -> int {{ return self.f{i} }}"), "\n")));
    src.push_str(&format!("enum E {{\n{}\n", list(&|i| format!("    C{i} {{ v: int }}"), "\n")));
    src.push_str(&format!("    Wide {{ {} }}\n}}\n\n", list(&|i| format!("b{i}: int"), ", ")));
    src.push_str(&format!("fn g({}) -> int {{\n    return p0\n}}\n\n", list(&|i| format!("p{i}: int"), ", ")));
    src.push_str(&format!("fn pick(e: E) -> int {{\n    match e {{\n{}\n", list(&|i| format!("        E.C{i} {{ v }} => return v"), "\n")));
    src.push_str(&format!("        E.Wide {{ {} }} => return b0\n    }}\n}}\n\n", list(&|i| format!("b{i}"), ", ")));
    src.push_str(&format!("on start {{\n    let s = S {{ {} }}\n", list(&|i| format!("f{i}: {i}"), ", ")));
    src.push_str(&format!("    log g({})\n", list(&|i| format!("p{i}: {i}"), ", ")));
    src.push_str(&format!("    log pick(E.Wide {{ {} }})\n", list(&|i| format!("b{i}: {i}"), ", ")));
    src.push_str(&list(&|i| format!("    log s.m{i}() + s.f{i} + pick(E.C{i} {{ v: {i} }})\n"), ""));
    src.push_str("}\n");
    src
}

/// Wide items are checked and built in time that grows with them: each use of a member (a field, a function, a case,
/// a case's field, a parameter named) looked for it through the whole item, and each field given through every one
/// given before it (N-4035: 5,000 and 20,000 fields given took 0.6 s and 7.1 s to check).
#[test]
fn wide_items_check_and_build_in_linear_time() {
    use lumascript::check::{check, ModFacts, Source};
    let game = lumascript::game::Game::new(Default::default(), None);
    let mut took = Vec::new();
    for n in [2_000usize, 8_000] {
        let (module, parsed) = parse_module(&wide_items(n));
        assert!(parsed.is_empty(), "{:?}", &parsed[..parsed.len().min(3)]);
        let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
        let start = std::time::Instant::now();
        let built = lumascript::on_compiler_stack(|| {
            let checked = check(&sources, &[], ModFacts::of_game(&game));
            let said: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
            assert!(said.is_empty(), "{:?}", &said[..said.len().min(3)]);
            eprintln!("{n}: checked in {:?}", start.elapsed());
            lumascript::emit::emit("wide", &sources, &checked, &game).map(|_| ()).map_err(|d| format!("{:?}", &d[..d.len().min(3)]))
        })
        .unwrap();
        took.push(start.elapsed());
        built.unwrap();
    }
    eprintln!("2000: {:?}, 8000: {:?}", took[0], took[1]);
    // Four times the items: linear is about four times the time; a square, sixteen.
    assert!(took[1] < took[0] * 8, "{took:?}");
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
