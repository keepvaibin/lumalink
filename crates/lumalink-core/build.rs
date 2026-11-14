//! Compiles LumaLink's own lines from `text/LumaLinkText.tsv` (id, then the game's 14 languages, the layout of
//! Nebula's `localization/NebulaText.tsv`) into the library. The build fails when a row lacks a language (EuEnglish may
//! be empty: it is then UsEnglish's), when an id is there twice, or when a language's placeholders differ from
//! UsEnglish's. That every id the code says has a row, and every row is said, is the test `text::tests`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

const LANGUAGES: [&str; 14] = [
    "UsEnglish", "EuEnglish", "UsFrench", "EuFrench", "UsSpanish", "EuSpanish", "EuGerman", "EuItalian", "EuDutch",
    "UsPortuguese", "JpJapanese", "KrKorean", "CnSimpChinese", "TwTradChinese",
];

fn placeholders(text: &str) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        match rest[open..].find('}') {
            Some(close) => {
                out.insert(&rest[open + 1..open + close]);
                rest = &rest[open + close + 1..];
            }
            None => break,
        }
    }
    out
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let tsv = manifest.join("text/LumaLinkText.tsv");
    println!("cargo:rerun-if-changed={}", tsv.display());
    let text = std::fs::read_to_string(&tsv).unwrap_or_else(|e| panic!("{}: {e}", tsv.display()));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    if header.len() != 15 || header[0] != "id" || header[1..] != LANGUAGES {
        panic!("{}: the header is not id and the 14 languages in order", tsv.display());
    }
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    let mut errors = Vec::new();
    for (number, line) in lines.enumerate() {
        let at = number + 2;
        let cells: Vec<&str> = line.split('\t').collect();
        let id = cells[0];
        if !id.starts_with("LumaLink.") {
            errors.push(format!("line {at}: {id:?} does not start with LumaLink."));
            continue;
        }
        if cells.len() != 15 {
            errors.push(format!("line {at}: {id} has {} cells, not 15", cells.len()));
            continue;
        }
        if rows.iter().any(|(other, _)| other == id) {
            errors.push(format!("line {at}: {id} is there twice"));
        }
        let english = placeholders(cells[1]);
        for (i, lang) in LANGUAGES.iter().enumerate() {
            let cell = cells[i + 1];
            if cell.is_empty() && *lang != "EuEnglish" {
                errors.push(format!("line {at}: {id} has no {lang} text"));
            } else if !cell.is_empty() && placeholders(cell) != english {
                errors.push(format!("line {at}: {id}: {lang}'s placeholders differ from UsEnglish's"));
            }
        }
        let texts = cells[1..].iter().map(|c| if c.is_empty() { cells[1].to_string() } else { c.to_string() }).collect();
        rows.push((id.to_string(), texts));
    }
    if !errors.is_empty() {
        panic!("{}:\n{}", tsv.display(), errors.join("\n"));
    }
    rows.sort();
    let mut out = String::from("pub(crate) static TEXT: &[(&str, [&str; 14])] = &[\n");
    for (id, texts) in &rows {
        let _ = write!(out, "    ({id:?}, [");
        for t in texts {
            let _ = write!(out, "{t:?}, ");
        }
        out.push_str("]),\n");
    }
    out.push_str("];\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("lumalink_text.rs");
    std::fs::write(dest, out).unwrap();
}
