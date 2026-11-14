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
