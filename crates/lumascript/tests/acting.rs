//! The mod a service call names: always the mod whose code makes it. A library's calls name the library, in its
//! exports, in tasks started there, in its requests and in their takes: the library's content, grant and handles are
//! its own. Acting for the caller is the kernel's explicit call for C mods; LumaScript has no form for it.

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::emit;
use lumascript::game::Game;
use std::collections::HashMap;

const GAME: &str = r#"{"types": [{"kind": "alias", "name": "s32", "of": "int", "record": null}], "globals": [], "left_out": []}"#;

/// The library's C (the run time written beside it left out) for `text` with the headers of `dir`.
fn library_c(dir: &str, headers_named: &[&str], text: &str) -> String {
    let dir = format!("{}/tests/data/svc{dir}", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in headers_named {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "kit/lib".into(), module }];
    let mut game = Game::new(HashMap::new(), Some(Types::from_json(GAME.as_bytes()).unwrap()));
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit_with("libmod", "1.0.0", &sources, &[text.to_string()], &[], &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        out.files.iter().filter(|(p, _)| p.ends_with(".c") && !p.ends_with("lumart.c")).map(|(_, c)| c.as_str()).collect()
    })
    .unwrap()
}

/// In the export's body, in a task the export starts and in the library's own start: each call names the library.
#[test]
fn a_librarys_service_calls_name_the_library() {
    let text = "use svc.ui\n\n\
        task later() {\n    wait 1 frames\n    if let s = ui.open(\"Later\", ui.Layer.Pause) {\n        ui.close(s)\n    }\n}\n\n\
        pub fn hud() -> bool {\n    start later()\n    if let s = ui.open(\"Hud\", ui.Layer.Pause) {\n        ui.close(s)\n        return true\n    }\n    return false\n}\n\n\
        on start {\n    if let s = ui.open(\"Own\", ui.Layer.Pause) {\n        ui.close(s)\n    }\n}\n";
    let all = library_c("", &["luma_services.h", "luma_ui.h"], text);
    assert_eq!(all.matches("->open(luma_rt_self, ").count(), 3, "{all}");
    assert_eq!(all.matches("->close(luma_rt_self, ").count(), 3, "{all}");
    assert!(!all.contains("lumart_caller.mod != 0 ?"), "{all}");
}

/// A request made inside an export, closure and waited: the event's take names the library, and its function is
/// given to the service as it is.
#[test]
fn a_librarys_requests_and_their_takes_name_the_library() {
    let text = "use svc.fs\nuse svc.system.clipboard\n\nvar pasted: text = \"\"\nvar size: int = 0\n\n\
        task load() {\n    if let b = wait fs.read(\"a.txt\") {\n        size = b.len()\n    }\n}\n\n\
        pub fn paste() -> bool {\n    start load()\n    let k = clipboard.get_text(fn(e: clipboard.TextEvent) {\n        if e.status == none {\n            pasted = e.data\n        }\n    })\n    return k != none\n}\n";
    let all = library_c("/c2", &["luma_services.h", "luma_fs.h", "luma_system.h"], text);
    for want in ["->get_text(luma_rt_self, ", "->take_text(luma_rt_self, ev->handle, ", "->take_read(luma_rt_self, ev->handle, ", "lumart_fn_keep(", "lumart_fn f = (lumart_fn)user;"] {
        assert!(all.contains(want), "{want} in\n{all}");
    }
}
