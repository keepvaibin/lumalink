//! LumaLink's own lines in the game's 14 languages, compiled from `text/LumaLinkText.tsv` by build.rs. The language
//! is chosen once, before any work ([`set_language`]); every line LumaLink says and every refusal it gives is made in
//! it with [`say!`](crate::say). The validator's findings come in the same 14 from its own table.

use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};

include!(concat!(env!("OUT_DIR"), "/lumalink_text.rs"));

static LANGUAGE: AtomicUsize = AtomicUsize::new(0);

/// Chooses the language of every later line; false (and nothing changed) when `name` is not one of the 14.
pub fn set_language(name: &str) -> bool {
    match luma_validate::LANGUAGES.iter().position(|l| *l == name) {
        Some(i) => {
            LANGUAGE.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// The language chosen, by its name in the game (UsEnglish ... TwTradChinese).
pub fn language() -> &'static str {
    luma_validate::LANGUAGES[LANGUAGE.load(Ordering::Relaxed)]
}

/// Whether `id` has a row (for lines whose id is made from a name, as the command line's help is).
pub fn has(id: &str) -> bool {
    TEXT.iter().any(|(row, _)| *row == id)
}

/// The row `id` in the language at `language`, its `{name}`s unfilled.
pub fn template(language: usize, id: &str) -> Option<&'static str> {
    TEXT.iter().find(|(row, _)| *row == id).map(|(_, texts)| texts[language])
}

/// Every row's id.
pub fn ids() -> impl Iterator<Item = &'static str> {
    TEXT.iter().map(|(id, _)| *id)
}

/// The line `id` in the chosen language with each `{name}` filled. An id without a row, a placeholder left unfilled
/// or a value given that the line does not use is a LumaLink fault: tests fail on it (`text::tests` and every test
/// that reaches the line), and a release build shows the id rather than a half-made line.
pub fn say(id: &str, values: &[(&str, &dyn Display)]) -> String {
    say_in(LANGUAGE.load(Ordering::Relaxed), id, values)
}

/// [`say`] in the language at `language` in `luma_validate::LANGUAGES`.
pub fn say_in(language: usize, id: &str, values: &[(&str, &dyn Display)]) -> String {
    let Some((_, texts)) = TEXT.iter().find(|(row, _)| *row == id) else {
        debug_assert!(false, "{id}: no row in LumaLinkText.tsv");
        return id.to_string();
    };
    let template = texts[language];
    let mut out = String::with_capacity(template.len() + 32);
    let mut used = vec![false; values.len()];
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        let name = &rest[open + 1..open + close];
        out.push_str(&rest[..open]);
        match values.iter().position(|(n, _)| *n == name) {
            Some(i) => {
                used[i] = true;
                out.push_str(&values[i].1.to_string());
            }
            None => {
                debug_assert!(false, "{id}: {{{name}}} is not given");
                return id.to_string();
            }
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    debug_assert!(used.iter().all(|u| *u), "{id}: a value is given that the line does not use");
    out
}

/// `say!("LumaLink.CannotRead", path = p.display())`: the line in the chosen language (see [`say`]).
#[macro_export]
macro_rules! say {
    ($id:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::text::say($id, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every id the code of the three crates (the library, the command line, the window) says has a row, and every row is said somewhere (the help's in the lumalink
    /// crate's own test).
    #[test]
    fn every_id_has_a_row_and_every_row_is_said() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut said = BTreeSet::new();
        for dir in [root.join("lumalink-core/src"), root.join("lumalink/src"), root.join("lumalink-gui/src")] {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().and_then(|e| e.to_str()) != Some("rs") || path.ends_with("text.rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                let mut rest = source.as_str();
                while let Some(at) = rest.find("\"LumaLink.") {
                    let tail = &rest[at + 1..];
                    let end = tail.find('"').unwrap();
                    // An id ending in '.' is the start of ids made from names (the help's), and one with a '{' is
                    // made by format! (the script's); neither is a row.
                    if !tail[..end].ends_with('.') && !tail[..end].contains('{') {
                        said.insert(tail[..end].to_string());
                    }
                    rest = &tail[end..];
                }
            }
        }
        let rows: BTreeSet<String> = TEXT.iter().map(|(id, _)| id.to_string()).collect();
        let missing: Vec<_> = said.difference(&rows).collect();
        // The help's rows and placeholders are made from the commands', arguments' and values' names; the lumalink
        // crate checks each is used. The script's are made from the compiler's codes and phrases (`script::tests`).
        let unsaid: Vec<_> = rows.difference(&said).filter(|id| !id.starts_with("LumaLink.Help.") && !id.starts_with("LumaLink.Cli.Value.") && !id.starts_with("LumaLink.Script.")).collect();
        assert!(missing.is_empty(), "said without a row: {missing:?}");
        assert!(unsaid.is_empty(), "rows never said: {unsaid:?}");
    }

    #[test]
    fn fills_in_every_language() {
        for (i, lang) in luma_validate::LANGUAGES.iter().enumerate() {
            let line = say_in(i, "LumaLink.Summary", &[("errors", &2), ("notes", &1), ("unchecked", &0)]);
            assert!(!line.contains('{') && line.contains('2') && line.contains('1') && line.contains('0'), "{lang}: {line}");
        }
        // The other tests run in parallel and read the chosen language, so this one does not change it.
        assert!(!set_language("Klingon"));
    }
}
