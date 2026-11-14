//! The one style: formatting every `.lum` of the tree changes no token and a second time changes
//! nothing; strings and comments stay as written.

use lumascript::fmt::format;
use lumascript::lex::{lex, Kind};
use std::path::{Path, PathBuf};

/// Each token as the parser sees it: its kind and text, a string's as written in the file.
fn tokens(src: &str) -> Vec<(Kind, String)> {
    let mut ignored = Vec::new();
    lex(src, &mut ignored)
        .into_iter()
        .map(|t| match &t.string {
            Some(s) => (t.kind, s.source[s.whole.clone()].iter().collect()),
            None => (t.kind, t.text),
        })
        .collect()
}

fn lum_files(folder: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Build output, and the Nebula checkout a work tree may link in, are not this tree's programs.
            if !matches!(name.as_ref(), "target" | "build" | "nebula" | ".git" | "node_modules") {
                lum_files(&path, out);
            }
        } else if name.ends_with(".lum") {
            out.push(path);
        }
    }
}

#[test]
fn every_lum_of_the_tree_keeps_its_tokens_and_formats_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    lum_files(&root, &mut files);
    files.sort();
    assert!(files.len() >= 46, "the language's 46 programs at least, found {}", files.len());
    let mut bad = String::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let once = format(&src);
        if format(&once) != once {
            bad += &format!("{}: a second format changes it\n", f.display());
        }
        if tokens(&once) != tokens(&src) {
            bad += &format!("{}: its tokens changed\n", f.display());
        }
        // Written on Windows: the same.
        if format(&src.replace("\r\n", "\n").replace('\n', "\r\n")) != once {
            bad += &format!("{}: CR LF formats otherwise\n", f.display());
        }
        if !once.is_empty() && (!once.ends_with('\n') || once.ends_with("\n\n") || once.contains("\n\n\n") || once.contains('\r')) {
            bad += &format!("{}: line ends not one LF each\n", f.display());
        }
        if once.lines().any(|l| l.ends_with(' ') || l.ends_with('\t')) {
            bad += &format!("{}: spaces at a line's end\n", f.display());
        }
    }
    assert!(bad.is_empty(), "{bad}");
}

#[test]
fn strings_and_comments_stay_as_written() {
    let src = "fn f() {\n\
               log \"a {x + \"}\"} {{ ( [ \\\" \\u{7B} \"   // ( [ {\n\
               var s = r\"\\{ (\"\n\
               /* a ( { [\n     \
               kept   as  is\n  \
               */ g()\n\
               x = `MR::f(int)` + b\"{\"\n\
               }\n";
    let want = "fn f() {\n    \
                log \"a {x + \"}\"} {{ ( [ \\\" \\u{7B} \"   // ( [ {\n    \
                var s = r\"\\{ (\"\n    \
                /* a ( { [\n     \
                kept   as  is\n  \
                */ g()\n    \
                x = `MR::f(int)` + b\"{\"\n\
                }\n";
    assert_eq!(format(src), want);
    assert_eq!(format(want), want);
}

#[test]
fn a_statement_that_goes_on_is_four_further_in() {
    let src = "on frame {\nlet far = a +\nb +\nc\nlet ok = a and\nnot b\nlet n = list<int>\nlet v = p.\nx\nenum E {\nup,\ndown,\n}\nif a {\n}\nelse {\n}\ncall(a,\nb)\n}\n";
    let want = "on frame {\n    let far = a +\n        b +\n        c\n    let ok = a and\n        not b\n    let n = list<int>\n    let v = p.\n        x\n    enum E {\n        up,\n        down,\n    }\n    if a {\n    }\n    else {\n    }\n    call(a,\n        b)\n}\n";
    assert_eq!(format(src), want);
}

#[test]
fn blank_lines_and_ends() {
    assert_eq!(format(""), "");
    assert_eq!(format("\n\n  \n"), "");
    assert_eq!(format("\u{feff}\n\nvar a = 1\n\n\n\nvar b = 2"), "var a = 1\n\nvar b = 2\n");
    // Unbalanced: a closer with no opener closes nothing; what is open stays open to the end.
    assert_eq!(format("}\nfn f() {\nx\n"), "}\nfn f() {\n    x\n");
}
