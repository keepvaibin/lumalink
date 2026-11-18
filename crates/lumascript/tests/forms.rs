//! The services' forms as `lumalink services --forms` prints them for the services' pages: each one, written as a
//! function that takes its parameters and gives its result, is a program the checker takes with nothing to say. So a
//! page that prints a form prints what a script can write (`LUMA_SDK`, the SDK's headers).

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::game::Game;
use lumascript::svc::Services;
use std::collections::HashMap;

const GAME: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 16, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "LiveActor", "size": 200, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": []}
  ],
  "globals": [],
  "left_out": []
}"#;

fn services() -> Option<Services> {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("skipped: LUMA_SDK is not set (the SDK's headers)");
        return None;
    };
    let mut headers = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(&sdk).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("luma") && name.ends_with(".h") {
            headers.insert(name, std::fs::read_to_string(e.path()).unwrap());
        }
    }
    let (services, errors) = Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    Some(services)
}

/// The parameters of a form, split at its top-level commas.
fn split(params: &str) -> Vec<String> {
    let (mut out, mut depth, mut cur) = (Vec::new(), 0, String::new());
    for c in params.chars() {
        match c {
            '(' | '<' => depth += 1,
            ')' | '>' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// A form as a function of the program: `fn f<n>(<parameters>) -> <result> { return <call> }`, a closure
/// (`fn(e: R)`, `fn(e: R)?`) taken as a parameter of its type (`fn(R)`).
fn as_function(n: usize, form: &str) -> String {
    // `wait <call> -> <result>`: in a task, the waited result held as its type.
    if let Some(waited) = form.strip_prefix("wait ") {
        let (call, result) = waited.split_once(") -> ").unwrap_or_else(|| panic!("{form}"));
        let open = call.find('(').unwrap();
        let params = split(&call[open + 1..]);
        let declared: Vec<String> = params.iter().map(|p| format!("p_{p}")).collect();
        let passed: Vec<String> = params.iter().map(|p| format!("p_{}", p.split_once(": ").unwrap().0)).collect();
        return format!("task w{n}({}) {{\n    let r: {result} = wait {}({})\n}}\n", declared.join(", "), &call[..open], passed.join(", "));
    }
    let (call, result) = match form.split_once(") -> ") {
        Some((c, r)) => (format!("{c})"), Some(r)),
        None => (form.to_string(), None),
    };
    let open = call.find('(').unwrap();
    let callee = &call[..open];
    let params = split(&call[open + 1..call.len() - 1]);
    let (mut declared, mut passed) = (Vec::new(), Vec::new());
    for (i, p) in params.iter().enumerate() {
        if let Some(rest) = p.strip_prefix("fn(e: ") {
            let (record, optional) = match rest.strip_suffix(")?") {
                Some(r) => (r, true),
                None => (rest.strip_suffix(')').unwrap(), false),
            };
            declared.push(format!("c{i}: fn({record}){}", if optional { "?" } else { "" }));
            passed.push(format!("c{i}"));
        } else {
            let (name, ty) = p.split_once(": ").unwrap_or_else(|| panic!("{form}: parameter {p}"));
            declared.push(format!("p_{name}: {ty}"));
            passed.push(format!("p_{name}"));
        }
    }
    let call = format!("{callee}({})", passed.join(", "));
    match result {
        Some(r) => format!("fn f{n}({}) -> {r} {{\n    return {call}\n}}\n", declared.join(", ")),
        None => format!("fn f{n}({}) {{\n    {call}\n}}\n", declared.join(", ")),
    }
}

#[test]
fn every_form_is_what_a_script_can_write() {
    let Some(services) = services() else { return };
    let forms = check::service_forms(&services);
    let mut by_service: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for (service, _, form, wait) in &forms {
        for f in form.iter().chain(wait) {
            by_service.entry(service).or_default().push(f);
        }
    }
    assert!(by_service.values().map(Vec::len).sum::<usize>() > 150, "{} forms", forms.len());
    let mut wrong = Vec::new();
    // A form names another service's type through that service's alias (`model.Model` in anim's): the program uses
    // that service too, as a script would.
    let mut by_alias: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for (service, _, _, _) in &forms {
        let list = by_alias.entry(service.rsplit('.').next().unwrap()).or_default();
        if !list.contains(&service.as_str()) {
            list.push(service);
        }
    }
    for (service, list) in &by_service {
        let own = service.rsplit('.').next().unwrap();
        let mut text = format!("use svc.{service}\n");
        for (alias, owners) in &by_alias {
            if *alias != own && owners.len() == 1 && list.iter().any(|f| f.contains(&format!("{alias}."))) {
                text += &format!("use svc.{}\n", owners[0]);
            }
        }
        text += "\n";
        for (n, f) in list.iter().enumerate() {
            text += &as_function(n, f);
        }
        let (module, d) = lumascript::parse_module(&text);
        if !d.is_empty() {
            wrong.push(format!("{service}: {d:?}\n{text}"));
            continue;
        }
        let sources = [Source { path: "main".into(), module }];
        let mut game = Game::new(HashMap::new(), Some(Types::from_json(GAME.as_bytes()).unwrap()));
        game.services = services.clone();
        let said = lumascript::on_compiler_stack(|| {
            let checked = check::check(&sources, &[], ModFacts::of_game(&game));
            checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).map(|d| format!("{d:?}")).collect::<Vec<_>>()
        })
        .unwrap();
        if !said.is_empty() {
            wrong.push(format!("{service}: {said:?}\n{text}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

#[test]
fn a_form_names_its_closure_by_its_record_and_gives_the_handle() {
    let Some(services) = services() else { return };
    let forms = check::service_forms(&services);
    let form = |s: &str, f: &str| forms.iter().find(|(a, b, _, _)| a == s && b == f).and_then(|(_, _, x, _)| x.clone());
    assert_eq!(
        form("net.local", "tcp_connect").as_deref(),
        Some("local.tcp_connect(host: text, port: u16, fn(e: local.Event)) -> local.Request?")
    );
    assert_eq!(form("draw.view", "create").as_deref(), Some("view.create(w: u32, h: u32, clear: color) -> view.Region"));
    // draw.native is C's alone: no form.
    assert!(forms.iter().filter(|(s, _, _, _)| s == "draw.native").all(|(_, _, f, w)| f.is_none() && w.is_none()));
    // A request closed after its one event: waited in a task, its event's data the result.
    let wait = forms.iter().find(|(a, b, _, _)| a == "fs" && b == "read").and_then(|(_, _, _, w)| w.clone());
    assert_eq!(wait.as_deref(), Some("wait fs.read(path: text) -> bytes?"));
}
