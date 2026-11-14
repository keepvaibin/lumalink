//! The checker's part (f): services in a body (docs/modding/language/language.md), against faces read from the SDK headers copied in
//! tests/data/svc.

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::{code, Arg};
use lumascript::game::Game;
use lumascript::svc::Services;
use std::collections::{BTreeMap, HashMap};

const TYPES: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 16, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "LiveActor", "size": 200, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": []}
  ],
  "globals": [],
  "left_out": []
}"#;

const CATALOG: [&str; 8] = ["time", "system", "system.clipboard", "system.links", "mods", "mods.ids", "mods.install", "content.fx"];

fn services() -> Services {
    let mut headers = BTreeMap::new();
    for f in ["luma_services.h", "luma_time.h", "luma_system.h", "luma_mods.h", "luma_content.h", "luma_content_fx.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{}/tests/data/svc/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap());
    }
    Services::read(&CATALOG, &headers).unwrap()
}

fn game(described: bool) -> Game {
    let mut g = Game::new(HashMap::new(), Some(Types::from_json(TYPES.as_bytes()).unwrap_or_else(|e| panic!("{e}"))));
    if described {
        g.services = services();
    }
    g
}

/// Each diagnostic as (code, line, args as text), in order.
type Said = Vec<(u16, u32, Vec<(&'static str, String)>)>;

fn said_with(text: &str, described: bool) -> Said {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    let g = game(described);
    let facts = ModFacts::of_game(&g);
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
    said_with(text, true)
}

fn codes(text: &str) -> Vec<(u16, u32)> {
    said(text).into_iter().map(|(c, l, _)| (c, l)).collect()
}

#[test]
fn time_reads_as_its_header_says() {
    let src = r#"use svc.time
fn f() {
    let n: time.Now = time.now()
    let s: time.State = n.state
    let frame: u64 = n.frame
    let busy: bool = time.step(4)
    let t: time.Timer? = time.timer(time.Clock.Frames, 60, 0, fn(e: time.TimerEvent) { let c: u64 = e.value })
    let ns: i64 = time.monotonic_ns()
    let most: u32 = time.TIMERS_MOST
    time.set_rate(500)
    time.hold()
    let w: time.Wall = time.wall()
    let y: i32 = w.year
    if s == time.State.Play { time.release() }
}
"#;
    assert_eq!(said(src), vec![]);
}

#[test]
fn ranges_handlers_and_names() {
    let src = r#"use svc.time
fn f() {
    time.step(17)
    time.set_rate(0)
    time.set_rate(16000)
    time.on_frame()
    let h = time.on_step
    time.nope()
    let c = time.Clock.Hours
    let n = time.now()
    let x = n.nothing
    let k = time.Now
}
"#;
    let s = said(src);
    assert_eq!(
        s.iter().map(|(c, l, _)| (*c, *l)).collect::<Vec<_>>(),
        vec![
            (code::SVC_RANGE, 3),
            (code::SVC_RANGE, 4),
            (code::SVC_SET_BY, 6),
            (code::SVC_SET_BY, 7),
            (code::NO_NAME_IN_MODULE, 8),
            (code::NO_MEMBER, 9),
            (code::NO_MEMBER, 11),
            (code::NOT_A_VALUE, 12),
        ]
    );
    assert_eq!(s[0].2, vec![("name", "time.step".into()), ("param", "count".into()), ("found", "17".into()), ("low", "1".into()), ("high", "16".into())]);
    assert_eq!(s[2].2, vec![("name", "time.on_frame".into()), ("what", "frame".into())]);
    assert_eq!(s[4].2, vec![("name", "nope".into()), ("module", "svc.time".into())]);
}

#[test]
fn an_event_of_a_request_is_no_on_event() {
    let s = said("use svc.time\non time.timer(e) { }\non time.tick(e) { }\n");
    assert_eq!(s.iter().map(|(c, l, _)| (*c, *l)).collect::<Vec<_>>(), vec![(code::SVC_PER_REQUEST, 2), (code::NO_NAME_IN_MODULE, 3)]);
    assert_eq!(s[0].2, vec![("event", "time.timer".into()), ("call", "`time.timer`".into())]);
}

#[test]
fn requests_with_a_closure_or_waited() {
    let src = r#"use svc.system.clipboard
use svc.mods.install
on start {
    let r: clipboard.Request? = clipboard.get_text(fn(e: clipboard.TextEvent) {
        let t: text = e.data
        if e.status == none { }
    })
    let t: text? = wait clipboard.get_text()
    let i: bytes? = wait clipboard.get_image()
    let d: u64? = wait install.request("a.luma")
    let ok: install.Request? = install.request("a.luma", fn(e: install.DoneEvent) {
        if e.status == install.Answer.Denied { }
    })
}
"#;
    assert_eq!(said(src), vec![]);
}

#[test]
fn what_a_script_cannot_call() {
    let src = r#"use svc.system.clipboard
use svc.time
fn f() {
    clipboard.take_text()
    clipboard.set_image()
    clipboard.get_text()
}
fn g() {
    use_timer()
}
fn use_timer() {
    let t = time.timer(time.Clock.Steps, 1, 1)
}
"#;
    let s = said(src);
    assert_eq!(s.iter().map(|(c, l, _)| (*c, *l)).collect::<Vec<_>>(), vec![(code::SVC_NOT_CALLABLE, 4), (code::SVC_PARAM, 5), (code::PENDING_DROPPED, 6), (code::ARG_MISSING, 12)]);
    assert_eq!(s[0].2, vec![("found", "system.clipboard.take_text".into()), ("why", "P:Script.Why.SvcTake".into())]);
    assert_eq!(s[1].2, vec![("found", "system.clipboard.set_image".into()), ("param", "rgba".into()), ("why", "P:Script.Why.SvcBuffer".into())]);
}

#[test]
fn effects_with_the_shared_place() {
    let src = r#"use svc.content.fx
fn f(a: LiveActor) {
    let e: fx.Effect = fx.start(a, effect "spark", none, fx.Place { offset: vec3(0, 50, 0), turn: vec3(0, 0, 0) }, true)
    let p = fx.Place { offset: vec3(0, 1, 0) }
    fx.tint(e, 2.0, 0xFF0000FF)
    fx.stop(e, false)
    let n: u32 = fx.MOST
}
"#;
    assert_eq!(codes(src), vec![(code::FIELD_MISSING, 4)]);
}

#[test]
fn a_service_the_game_lacks_and_one_not_described() {
    assert_eq!(codes("use svc.net\n"), vec![(code::NO_MODULE, 1)]);
    // No SDK headers given: the old note, and nothing of the service checked.
    let s = said_with("use svc.time\nfn f() { time.anything(1) }\n", false);
    assert_eq!(s.iter().map(|(c, l, _)| (*c, *l)).collect::<Vec<_>>(), vec![(code::NOT_DESCRIBED, 1)]);
}

#[test]
fn types_of_a_face() {
    let src = r#"use svc.time
use svc.system
fn f(c: time.Clock, t: time.Timer, n: time.Now, e: time.TimerEvent, a: time.Answer, x: time.Nope) { }
fn g() -> system.Renderer { return system.facts().renderer }
"#;
    assert_eq!(codes(src), vec![(code::NO_NAME_IN_MODULE, 3)]);
}
