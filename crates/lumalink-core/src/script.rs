//! LumaScript's compile errors in the chosen language. The compiler (crate `lumascript`) gives each a code, a place and
//! its values by name, some of them phrases; the words are the rows `LumaLink.Script.S<code>` and
//! `LumaLink.Script.Want.*` / `Found.*`, and the line around them is `LumaLink.ScriptAt`, as a finding's is
//! `LumaLink.FindingAt`.

use crate::text;
use lumascript::{Arg, Diagnostic};
use std::fmt::Display;

fn value(language: usize, arg: &Arg) -> String {
    match arg {
        Arg::Text(text) => text.clone(),
        Arg::Phrase(id) => text::say_in(language, &format!("LumaLink.{id}"), &[]),
    }
}

/// The message alone: `found `)`, expected a name`.
pub fn message_in(language: usize, d: &Diagnostic) -> String {
    let values: Vec<(&str, String)> = d.args.iter().map(|(name, arg)| (*name, value(language, arg))).collect();
    let refs: Vec<(&str, &dyn Display)> = values.iter().map(|(name, v)| (*name, v as &dyn Display)).collect();
    text::say_in(language, &format!("LumaLink.{}", d.row()), &refs)
}

/// One error as one line: `error S0001: found `)`, expected a name (scripts/main.lum:4:12)`. A note
/// ([`lumascript::diag::is_note`]) reads `note S0033: ...`.
pub fn line_in(language: usize, file: &str, d: &Diagnostic) -> String {
    let level_row = if lumascript::diag::is_note(d.code) { "LumaLink.Level.Note" } else { "LumaLink.Level.Error" };
    let level = text::say_in(language, level_row, &[]);
    let code = format!("{:04}", d.code);
    let message = message_in(language, d);
    let place = format!("{file}:{}:{}", d.at.line, d.at.col);
    text::say_in(language, "LumaLink.ScriptAt", &[("level", &level), ("code", &code), ("message", &message), ("place", &place)])
}

/// [`line_in`] in the chosen language.
pub fn line(file: &str, d: &Diagnostic) -> String {
    let language = luma_validate::LANGUAGES.iter().position(|l| *l == text::language()).unwrap_or(0);
    line_in(language, file, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumascript::diag::{args_of, code, PHRASES};
    use std::collections::BTreeSet;

    fn placeholders(template: &str) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else { break };
            out.insert(&rest[open + 1..open + close]);
            rest = &rest[open + close + 1..];
        }
        out
    }

    /// Each code has its row, filling exactly the values the compiler gives it; each phrase has its row, filling none;
    /// and no `LumaLink.Script.` row is left that the compiler cannot say.
    #[test]
    fn every_code_and_phrase_has_its_row_and_no_row_is_left_over() {
        let mut wanted = BTreeSet::new();
        for c in code::ALL {
            let id = format!("LumaLink.Script.S{c:04}");
            let template = text::template(0, &id).unwrap_or_else(|| panic!("{id}: no row"));
            let names: BTreeSet<&str> = args_of(*c).iter().copied().collect();
            assert_eq!(placeholders(template), names, "{id}");
            wanted.insert(id);
        }
        for p in PHRASES {
            let id = format!("LumaLink.{p}");
            let template = text::template(0, &id).unwrap_or_else(|| panic!("{id}: no row"));
            assert!(placeholders(template).is_empty(), "{id} fills nothing");
            wanted.insert(id);
        }
        let rows: BTreeSet<String> = text::ids().filter(|id| id.starts_with("LumaLink.Script.")).map(String::from).collect();
        let left: Vec<_> = rows.difference(&wanted).collect();
        assert!(left.is_empty(), "rows no code or phrase says: {left:?}");
    }

    /// Real errors from the compiler, made into lines in each of the 14 languages: every value filled, the code and
    /// the place in the line.
    #[test]
    fn real_errors_read_in_every_language() {
        let (_, diags) = lumascript::parse_module("on start {\n    if x && y { log \"a\" }\n    let a = (1\n}\nfn f(_: int) { }\n");
        assert!(diags.len() >= 3, "{diags:?}");
        for (i, lang) in luma_validate::LANGUAGES.iter().enumerate() {
            for d in &diags {
                let line = line_in(i, "main.lum", d);
                assert!(!line.contains('{') && !line.contains("LumaLink."), "{lang}: {line}");
                assert!(line.contains(&format!("S{:04}", d.code)) && line.contains(&format!("main.lum:{}:{}", d.at.line, d.at.col)), "{lang}: {line}");
            }
        }
        let first = line_in(0, "main.lum", &diags[0]);
        assert_eq!(first, "error S0012: `&&` is written `and` (main.lum:2:10)");
    }

    /// A note reads as a note, its phrase filled, in every language.
    #[test]
    fn a_note_reads_as_a_note() {
        let d = Diagnostic::new(code::NOT_DESCRIBED, lumascript::ast::Pos { line: 3, col: 5 })
            .with("found", Arg::Text("svc.net".into()))
            .with("what", Arg::Phrase("Script.What.ServiceFaces"));
        assert_eq!(
            line_in(0, "main.lum", &d),
            "note S0033: `svc.net` needs the service's functions in LumaScript types, which this game's self-description \
             does not give yet; not checked (main.lum:3:5)"
        );
        for i in 0..luma_validate::LANGUAGES.len() {
            let line = line_in(i, "main.lum", &d);
            assert!(!line.contains('{') && !line.contains("LumaLink.") && line.contains("svc.net"), "{line}");
            assert!(line.starts_with(&text::say_in(i, "LumaLink.Level.Note", &[])), "{line}");
        }
    }

    /// Part (c)'s errors from the checker, phrases among them (a place that cannot be written, a name that is no
    /// value), read in every language with every value filled.
    #[test]
    fn body_errors_read_in_every_language() {
        use lumascript::check::{check, ModFacts, Source};
        let text = "struct P {\n    x: int\n}\nfn f(a: int) -> int {\n    let b = 1\n    b = 2\n    a = 3\n    let c: text = 4\n    let d = P\n    break\n    return nope\n}\n";
        let (module, parsed) = lumascript::parse_module(text);
        assert!(parsed.is_empty(), "{parsed:?}");
        let game = lumascript::game::Game::new(Default::default(), None);
        let facts = ModFacts::of_game(&game);
        let checked = check(&[Source { path: "main".into(), module }], &[], facts);
        let codes: Vec<u16> = checked.diags.iter().map(|(_, d)| d.code).collect();
        let want = [code::NOT_A_PLACE, code::NOT_A_PLACE, code::TYPE_MISMATCH, code::NOT_A_VALUE, code::OUTSIDE_LOOP, code::UNKNOWN_NAME];
        assert_eq!(codes, want);
        for (i, lang) in luma_validate::LANGUAGES.iter().enumerate() {
            for (_, d) in &checked.diags {
                let line = line_in(i, "main.lum", d);
                assert!(!line.contains('{') && !line.contains("LumaLink."), "{lang}: {line}");
                assert!(line.contains(&format!("S{:04}", d.code)), "{lang}: {line}");
            }
        }
        let lines: Vec<String> = checked.diags.iter().map(|(_, d)| message_in(0, d)).collect();
        assert_eq!(
            lines,
            [
                "this cannot be written: it is a `let`, which never changes (`var` makes one that does)",
                "this cannot be written: it is a parameter; a function writes its caller's value only through a `var` parameter",
                "a `i32` where a `text` goes",
                "`P` is a type, not a value",
                "`break` outside a loop",
                "`nope` names nothing here: no local, no name of the module or its uses, no standard name, nothing of the game",
            ]
        );
    }
}
