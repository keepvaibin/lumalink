//! How much stack each form of nesting needs at the deepest the parser takes (lib.rs, COMPILER_STACK): for each form,
//! the least stack, to 16 KB, on which lexing, parsing and dropping the tree ends. Each try runs in a child process,
//! since a stack overflow ends the process.
//!
//! `cargo run -p lumascript --example depth_stack` (a debug build) or with `--release`.

use lumascript::{lex, parse::Parser};
use std::process::Command;

/// Each form and the deepest of it the parser takes (tests/depth.rs FORMS).
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
        _ => panic!("no form {form}"),
    };
    format!("on start {{\n{body}\n}}\n")
}

/// In the child: parse `form` `n` deep on a thread of `kb` KB, here and not on the compiler's own stack; exit 0 when
/// it is clean, 1 when not.
fn child(form: &str, n: usize, kb: usize) {
    let src = source(form, n);
    let clean = std::thread::Builder::new()
        .stack_size(kb << 10)
        .spawn(move || {
            let mut diags = Vec::new();
            let toks = lex::lex(&src, &mut diags);
            drop(Parser::new(toks, &mut diags).module());
            diags.is_empty()
        })
        .unwrap()
        .join()
        .unwrap();
    std::process::exit(if clean { 0 } else { 1 });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 {
        return child(&args[1], args[2].parse().unwrap(), args[3].parse().unwrap());
    }
    let exe = std::env::current_exe().unwrap();
    let ends = |form: &str, n: usize, kb: usize| {
        Command::new(&exe).args([form, &n.to_string(), &kb.to_string()]).output().is_ok_and(|o| o.status.success())
    };
    for &(form, n) in FORMS {
        assert!(ends(form, n, 1 << 18), "{form} {n} is not clean on 256 MB");
        let (mut lo, mut hi) = (16, 1 << 18);
        while hi - lo > 16 {
            let mid = (lo + hi) / 2;
            if ends(form, n, mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        println!("{form:>9} {n:>4} deep: {hi:>6} KB");
    }
}
