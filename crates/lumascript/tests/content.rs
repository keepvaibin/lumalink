//! The checker's part (f): content literals against the mod's content (docs/modding/language/language.md, section 10) and `on option` against
//! luma.json.

use lumascript::check::{self, ContentNames, ModFacts, OptionKind, OptionSpec, Source};
use lumascript::diag::{code, Arg};
use lumascript::game::Game;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Each diagnostic as (code, line, args as text), in order.
type Said = Vec<(u16, u32, Vec<(&'static str, String)>)>;

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn content() -> ContentNames {
    ContentNames {
        names: BTreeMap::from([
            ("model".to_string(), set(&["Gold/Mario", "Spark"])),
            ("effect".to_string(), set(&["spark"])),
            ("sound".to_string(), set(&[])),
        ]),
        texts: Some(BTreeMap::from([
            ("Coop.Hosting".to_string(), set(&["address"])),
            ("Coop.Joined".to_string(), set(&["name", "count"])),
            ("Coop.Left".to_string(), set(&[])),
        ])),
        game_unlisted: BTreeSet::new(),
    }
}

fn said_with(text: &str, names: Option<&ContentNames>) -> Said {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let game = Game::new(HashMap::new(), None);
    let options = [OptionSpec { key: "port".into(), kind: OptionKind::Number { whole: true } }];
    let facts = ModFacts { needs: &[], options: &options, c_headers: &[], game: &game, content: names };
    let checked = check::check(&[Source { path: "main".into(), module }], &[], facts);
    checked
        .diags
        .into_iter()
        .map(|(_, d)| {
            let args = d.args.iter().map(|(n, a)| (*n, match a {
                Arg::Text(t) => t.clone(),
                Arg::Phrase(p) => format!("P:{p}"),
            })).collect();
            (d.code, d.at.line, args)
        })
        .collect()
}

fn said(text: &str) -> Said {
    said_with(text, Some(&content()))
}

fn codes(s: &Said) -> Vec<(u16, u32)> {
    s.iter().map(|(c, l, _)| (*c, *l)).collect()
}

#[test]
fn names_the_mod_has() {
    let src = r#"fn f() {
    let a = model "Gold/Mario"
    let b = effect "spark"
    let t = text "Coop.Hosting"(address: "here")
    let u = text "Coop.Joined"(count: 2, name: "Luigi")
    let v = text "Coop.Left"
    let l = layout "anything"
}
"#;
    assert_eq!(said(src), vec![]);
}

#[test]
fn names_the_mod_lacks() {
    let src = r#"fn f() {
    let a = model "Gold/Luigi"
    let b = effect "Spark"
    let c = sound "Jump"
    let t = text "Coop.Nope"(x: 1)
}
"#;
    let s = said(src);
    assert_eq!(codes(&s), vec![(code::CONTENT_MISSING, 2), (code::CONTENT_MISSING, 3), (code::CONTENT_MISSING, 4), (code::CONTENT_MISSING, 5)]);
    assert_eq!(s[0].2, vec![("kind", "model".into()), ("found", "Gold/Luigi".into())]);
    assert_eq!(s[3].2, vec![("kind", "text".into()), ("found", "Coop.Nope".into())]);
}

/// The build has no list of the game's model archives: with `model` among the words the game may hold, a bare name
/// the mod lacks (`GhostMario`, ObjectData/GhostMario.arc) is not refused; a name with a folder or a mod's id is still
/// checked against the mods' assets.
#[test]
fn a_bare_model_name_may_be_the_games() {
    let src = r#"fn f() {
    let a = model "GhostMario"
    let b = model "Gold/Luigi"
    let c = model "kit:Ghost"
}
"#;
    let names = ContentNames { game_unlisted: set(&["model"]), ..content() };
    assert_eq!(codes(&said_with(src, Some(&names))), vec![(code::CONTENT_MISSING, 3), (code::CONTENT_MISSING, 4)]);
    assert_eq!(codes(&said(src)), vec![(code::CONTENT_MISSING, 2), (code::CONTENT_MISSING, 3), (code::CONTENT_MISSING, 4)]);
}

#[test]
fn a_text_s_placeholders() {
    let src = r#"fn f() {
    let a = text "Coop.Joined"(name: "Luigi")
    let b = text "Coop.Hosting"(address: "here", port: 1)
    let c = text "Coop.Hosting"("here")
    let d = text "Coop.Hosting"
}
"#;
    let s = said(src);
    assert_eq!(
        codes(&s),
        vec![
            (code::TEXT_ARG_MISSING, 2),
            (code::TEXT_ARG_UNKNOWN, 3),
            (code::TEXT_ARG_MISSING, 4),
            (code::TEXT_ARG_UNNAMED, 4),
            (code::TEXT_ARG_MISSING, 5),
        ]
    );
    assert_eq!(s[0].2, vec![("found", "Coop.Joined".into()), ("name", "count".into())]);
    assert_eq!(s[1].2, vec![("found", "Coop.Hosting".into()), ("name", "port".into())]);
    assert_eq!(s[3].2, vec![("found", "Coop.Hosting".into())]);
}

#[test]
fn outside_a_mod_s_build_nothing_is_checked() {
    let src = "fn f() {\n    let a = model \"Gold/Luigi\"\n    let t = text \"Coop.Nope\"(x: 1)\n}\n";
    assert_eq!(said_with(src, None), vec![]);
}

#[test]
fn texts_unknown_to_the_build_are_not_checked() {
    // LumaLink's build has the mod's files but not the game's message tables: texts None, names still checked.
    let src = "fn f() {\n    let a = model \"Gold/Luigi\"\n    let t = text \"Coop.Nope\"(x: 1)\n    let u = text \"Coop.Hosting\"\n}\n";
    let names = ContentNames { texts: None, ..content() };
    assert_eq!(codes(&said_with(src, Some(&names))), vec![(code::CONTENT_MISSING, 2)]);
}

#[test]
fn on_option_against_luma_json() {
    let s = said("on option port { }\non option colour { }\n");
    assert_eq!(codes(&s), vec![(code::OPTION_NOT_IN_LUMA_JSON, 2)]);
    assert_eq!(s[0].2, vec![("found", "colour".into())]);
}
