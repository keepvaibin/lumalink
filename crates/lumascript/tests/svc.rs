//! Service headers in the registry's one form read into LumaScript faces (docs/modding/language/language.md). The headers in
//! tests/data/svc are copied from the SDK as they are (luma_services.h, luma_time.h,
//! luma_system.h, luma_mods.h, luma_content.h, luma_content_fx.h).

use lumascript::svc::{case_name, Face, Gives, Header, ParamWhy, SParam, STy, Services, Shape, Unfit, CT};

fn header(name: &str) -> Header {
    let text = std::fs::read_to_string(format!("{}/tests/data/svc/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    Header::read(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

const CATALOG: [&str; 7] = ["mods", "mods.ids", "mods.install", "save", "system", "system.clipboard", "system.links"];

fn face(service: &str, file: &str) -> Face {
    let mut catalog = CATALOG.to_vec();
    catalog.push("time");
    Face::of(service, &header(file), &header("luma_services.h"), &catalog).unwrap_or_else(|e| panic!("{service}: {e}"))
}

fn n(t: &str) -> STy {
    STy::Number(t.into())
}

fn v(name: &str, ty: STy) -> SParam {
    SParam::Value { name: name.into(), ty }
}

#[test]
fn the_services_header_reads() {
    let h = header("luma_services.h");
    let status = h.enums.iter().find(|e| e.name.is_none()).unwrap();
    assert_eq!(status.cases[0], ("LUMA_OK".into(), 0, String::new()));
    assert!(status.cases.iter().any(|c| c.0 == "LUMA_E_UNREACHABLE" && c.1 == -17));
    let u = |b: &str| CT { base: b.into(), konst: false, ptrs: 0 };
    assert_eq!(h.aliases, vec![("LumaHandle".to_string(), u("uint64_t")), ("LumaColor".to_string(), u("uint32_t"))]);
    let event = h.record("LumaEvent").unwrap();
    assert_eq!(event.members.len(), 8);
    assert_eq!(h.fn_types[0].name, "LumaEventFn");
    assert_eq!(h.numbers["LUMA_EVENTS_PER_FRAME"], 256);
}

#[test]
fn time_in_the_one_form() {
    let f = face("time", "luma_time.h");
    assert_eq!(f.constants["TIMERS_MOST"], 64);
    assert_eq!(f.constants["REAL"], 1000);
    assert!(!f.constants.contains_key("MAJOR"));
    assert_eq!(f.events["timer"].0, 1);
    // named enums, typed in fields and parameters.
    assert_eq!(f.enums["Clock"], vec![("Frames".into(), 0), ("Steps".into(), 1), ("Nanoseconds".into(), 2)]);
    assert_eq!(f.enums["State"].len(), 6);
    assert!(f.structs["Now"].contains(&("state".to_string(), STy::Enum("State".into()))));
    assert!(!f.structs["Now"].iter().any(|(name, _)| name == "size" || name == "reserved"));
    // the timer's handle kind.
    assert_eq!(f.handles, vec!["Timer"]);
    let shape = |name: &str| f.function(name).unwrap().shape.clone();
    assert_eq!(shape("now"), Shape::Callable { params: vec![], gives: Gives::Outs(vec![STy::Struct("Now".into())]) });
    assert_eq!(shape("monotonic_ns"), Shape::Callable { params: vec![], gives: Gives::Number("int64_t".into()) });
    assert!(f.function("monotonic_ns").unwrap().any_thread);
    assert!(!f.function("now").unwrap().any_thread);
    assert_eq!(shape("set_rate"), Shape::Callable { params: vec![v("rate", n("uint32_t"))], gives: Gives::Outs(vec![]) });
    assert_eq!(shape("hold"), Shape::Callable { params: vec![], gives: Gives::Outs(vec![]) });
    // BUSY is an answer: `step` is a bool.
    assert_eq!(shape("step"), Shape::Callable { params: vec![v("count", n("uint32_t"))], gives: Gives::Answered(vec![]) });
    assert_eq!(f.function("step").unwrap().answers, vec!["Busy"]);
    // Status ranges.
    assert_eq!(f.function("step").unwrap().ranges, vec![("count".to_string(), 1, 16)]);
    assert_eq!(f.function("set_rate").unwrap().ranges, vec![("rate".to_string(), 1, 16000)]);
    // a TIMER event follows; not waitable (a timer may repeat).
    assert_eq!(
        shape("timer"),
        Shape::Callable {
            params: vec![v("clock", STy::Enum("Clock".into())), v("after", n("uint64_t")), v("every", n("uint64_t")), SParam::Closure { optional: false }],
            gives: Gives::Request { handle: STy::Handle("Timer".into()), waitable: false, kind: "timer".into() },
        }
    );
    assert_eq!(shape("cancel"), Shape::Callable { params: vec![v("timer", STy::Handle("Timer".into()))], gives: Gives::Outs(vec![]) });
    // set by on frame / on step.
    assert_eq!(shape("on_frame"), Shape::SetBy("frame".into()));
    assert_eq!(shape("on_step"), Shape::SetBy("step".into()));
}

#[test]
fn system_and_its_dotted_services() {
    let s = face("system", "luma_system.h");
    // The clipboard's and links' names are theirs, not system's.
    assert_eq!(s.enums.keys().collect::<Vec<_>>(), vec!["Arch", "Platform", "Region", "Renderer"]);
    assert_eq!(s.enums["Renderer"], vec![("D3d12".into(), 0), ("Vulkan".into(), 1), ("Metal".into(), 2)]);
    assert!(s.events.is_empty());
    assert!(s.constants.is_empty());
    assert!(s.structs["Facts"].contains(&("gpu".to_string(), STy::Text)));
    assert!(s.structs["Facts"].contains(&("platform".to_string(), STy::Enum("Platform".into()))));
    assert!(s.structs["Display"].contains(&("height".to_string(), n("uint32_t"))));
    assert_eq!(s.function("facts").unwrap().shape, Shape::Callable { params: vec![], gives: Gives::Outs(vec![STy::Struct("Facts".into())]) });
    // display names its statuses now.
    assert_eq!(s.function("display").unwrap().shape, Shape::Callable { params: vec![], gives: Gives::Outs(vec![STy::Struct("Display".into())]) });

    let c = face("system.clipboard", "luma_system.h");
    assert_eq!(c.events.keys().collect::<Vec<_>>(), vec!["image", "set", "text"]);
    let get = c.function("get_text").unwrap();
    assert_eq!(get.shape, Shape::Callable { params: vec![SParam::Closure { optional: false }], gives: Gives::Request { handle: STy::Handle("Request".into()), waitable: true, kind: "text".into() } });
    assert_eq!(c.function("get_image").unwrap().answers, vec!["Absent"]);
    assert_eq!(c.function("take_text").unwrap().shape, Shape::Take { kind: "text".into(), data: STy::Text });
    assert_eq!(c.function("take_image").unwrap().shape, Shape::Take { kind: "image".into(), data: STy::Bytes });
    // Text with its length: one argument; a SET event follows.
    assert_eq!(
        c.function("set_text").unwrap().shape,
        Shape::Callable { params: vec![v("text", STy::Text), SParam::Closure { optional: false }], gives: Gives::Request { handle: STy::Handle("Request".into()), waitable: true, kind: "set".into() } }
    );
    // Pixels whose size is width x height, not a size parameter: not callable (the reason names the parameter).
    assert_eq!(
        c.function("set_image").unwrap().shape,
        Shape::NotCallable(Unfit::Param { param: "rgba".into(), why: ParamWhy::Buffer, c_type: "uint8_t*".into() })
    );
    assert_eq!(c.function("set_image").unwrap().ranges, vec![("width".to_string(), 1, 8192), ("height".to_string(), 1, 8192)]);

    let l = face("system.links", "luma_system.h");
    assert_eq!(l.function("open").unwrap().answers, vec!["Busy"]);
    assert!(matches!(&l.function("open").unwrap().shape, Shape::Callable { params, gives: Gives::Request { kind, .. } } if params[0] == v("url", STy::Text) && kind == "link"));
    assert_eq!(l.function("cancel").unwrap().shape, Shape::Callable { params: vec![v("request", STy::Handle("Request".into()))], gives: Gives::Outs(vec![]) });
}

#[test]
fn mods_and_the_services_its_header_names() {
    // luma_mods.h: three tables in one header. `mods` takes none of mods.ids' or mods.install's names, whether or
    // not the catalog has them (the header's own LUMA_*_NAME texts say so).
    let m = Face::of("mods", &header("luma_mods.h"), &header("luma_services.h"), &["mods"]).unwrap();
    assert_eq!(m.enums.keys().collect::<Vec<_>>(), vec!["Kind", "State"]);
    assert!(m.constants.is_empty());
    assert_eq!(m.function("count").unwrap().shape, Shape::Callable { params: vec![], gives: Gives::Number("uint32_t".into()) });
    assert_eq!(m.function("find").unwrap().shape, Shape::Callable { params: vec![v("id", STy::Text)], gives: Gives::Answered(vec![STy::Struct("Info".into())]) });
    let i = face("mods.install", "luma_mods.h");
    assert_eq!(i.events.keys().collect::<Vec<_>>(), vec!["done"]);
    let r = i.function("request").unwrap();
    assert_eq!(
        r.shape,
        Shape::Callable { params: vec![v("path", STy::Text), SParam::Closure { optional: false }], gives: Gives::Request { handle: STy::Handle("Request".into()), waitable: true, kind: "done".into() } }
    );
    let ids = face("mods.ids", "luma_mods.h");
    assert_eq!(ids.function("get").unwrap().shape, Shape::Callable { params: vec![v("space", STy::Text), v("name", STy::Text)], gives: Gives::Outs(vec![n("uint32_t")]) });
    assert!(matches!(&ids.function("of").unwrap().shape, Shape::Callable { params, gives: Gives::Answered(o) } if params.len() == 3 && o == &vec![n("uint32_t")]));
    assert_eq!(i.function("cancel").unwrap().shape, Shape::Callable { params: vec![v("handle", STy::Handle("Request".into()))], gives: Gives::Outs(vec![]) });
}

/// The registry's one form, as a header written to them reads.
const R9: &str = r#"
#define LUMA_DEMO_NAME "demo"
#define LUMA_DEMO_STEPS_MOST 16u
typedef enum LumaDemoClock { LUMA_DEMO_CLOCK_FRAMES = 0, LUMA_DEMO_CLOCK_STEPS = 1 } LumaDemoClock;
typedef LumaHandle LumaDemoTimer;
typedef enum LumaDemoEvent {
    LUMA_DEMO_EVENT_TIMER = 1, /* data: none; value: the times it came due */
    LUMA_DEMO_EVENT_TEXT = 2, /* data: none; value: the text's bytes */
    LUMA_DEMO_EVENT_IMAGE = 3 /* data: none; value: 0 */
} LumaDemoEvent;
typedef struct LumaDemoNow {
    uint32_t size;
    LumaDemoClock clock; /* the clock */
    uint32_t reserved[3];
    float rates[2];
} LumaDemoNow;
typedef struct LumaDemoService {
    LumaServiceTable base;
    /* 1.0. Game thread. LUMA_OK; count: 1 to 16. LUMA_E_BUSY (an answer), LUMA_E_ARGUMENT reported. */
    int32_t (*step)(const LumaMod* mod, uint32_t count);
    /* 1.0. Game thread. LUMA_PENDING with the timer in *out; a TIMER event follows. LUMA_E_ARGUMENT, LUMA_E_LIMIT. */
    int32_t (*timer)(const LumaMod* mod, LumaDemoClock clock, uint64_t after, LumaEventFn fn, void* user, LumaDemoTimer* out);
    /* 1.0. Game thread. LUMA_OK; LUMA_E_HANDLE reported. */
    int32_t (*cancel)(const LumaMod* mod, LumaDemoTimer timer);
    /* 1.0. Game thread. LUMA_PENDING; a TEXT event follows; closed after its event. LUMA_E_ARGUMENT. */
    int32_t (*get_text)(const LumaMod* mod, LumaEventFn fn, void* user, LumaHandle* out);
    /* 1.0. Game thread, inside the TEXT event's call. Returns the whole length. LUMA_E_HANDLE. */
    int64_t (*take_text)(const LumaMod* mod, LumaHandle request, char* buf, uint64_t size);
    /* 1.0. Game thread. The value; LUMA_OK, LUMA_E_NOT_FOUND, LUMA_E_ABSENT. */
    int64_t (*get)(const LumaMod* mod, const char* key, void* buf, uint64_t size);
    /* 1.0. Game thread. LUMA_OK. This mod's frame handler: set by on frame. */
    int32_t (*on_frame)(const LumaMod* mod, LumaDemoFn fn, void* user);
    /* 1.0. Game thread. Both outs. LUMA_OK; LUMA_E_ABSENT. */
    int32_t (*file)(const LumaMod* mod, uint32_t* number, LumaDemoClock* clock);
    /* 1.0. Game thread. LUMA_PENDING; an IMAGE event follows. */
    int32_t (*image)(const LumaMod* mod, LumaEventFn fn, void* user, LumaHandle* out);
    /* 1.0. Game thread. LUMA_PENDING; a SOUND event follows. */
    int32_t (*sound)(const LumaMod* mod, LumaEventFn fn, void* user, LumaHandle* out);
    /* 1.0. Game thread. LUMA_PENDING and *out. */
    int32_t (*quiet)(const LumaMod* mod, LumaEventFn fn, void* user, LumaHandle* out);
    /* 1.0. Game thread. LUMA_OK; LUMA_E_HANDLE. A colour. */
    int32_t (*paint)(const LumaMod* mod, LumaDemoTimer timer, LumaColor rgba);
    /* 1.0. LUMA_OK, LUMA_E_NOWHERE. */
    int32_t (*bad)(const LumaMod* mod);
} LumaDemoService;
"#;

#[test]
fn the_form_with_r9() {
    let f = Face::of("demo", &Header::read(R9).unwrap(), &header("luma_services.h"), &["demo"]).unwrap();
    assert_eq!(f.enums["Clock"], vec![("Frames".into(), 0), ("Steps".into(), 1)]);
    assert_eq!(f.handles, vec!["Timer"]);
    assert_eq!(f.constants["STEPS_MOST"], 16);
    assert_eq!(f.structs["Now"], vec![("clock".to_string(), STy::Enum("Clock".into())), ("rates".to_string(), STy::Vec(2))]);
    assert!(f.unreadable.is_empty());
    let step = f.function("step").unwrap();
    assert_eq!(step.ranges, vec![("count".to_string(), 1, 16)]);
    assert_eq!(
        f.function("timer").unwrap().shape,
        Shape::Callable {
            params: vec![v("clock", STy::Enum("Clock".into())), v("after", n("uint64_t")), SParam::Closure { optional: false }],
            gives: Gives::Request { handle: STy::Handle("Timer".into()), waitable: false, kind: "timer".into() },
        }
    );
    assert_eq!(f.function("cancel").unwrap().shape, Shape::Callable { params: vec![v("timer", STy::Handle("Timer".into()))], gives: Gives::Outs(vec![]) });
    assert_eq!(f.function("get_text").unwrap().shape, Shape::Callable { params: vec![SParam::Closure { optional: false }], gives: Gives::Request { handle: STy::Handle("Request".into()), waitable: true, kind: "text".into() } });
    assert_eq!(f.function("take_text").unwrap().shape, Shape::Take { kind: "text".into(), data: STy::Text });
    // A length with answers: the out buffer's `T?`.
    assert_eq!(f.function("get").unwrap().shape, Shape::Callable { params: vec![v("key", STy::Text)], gives: Gives::Answered(vec![STy::Bytes]) });
    assert_eq!(f.function("get").unwrap().answers, vec!["NotFound", "Absent"]);
    assert_eq!(f.function("on_frame").unwrap().shape, Shape::SetBy("frame".into()));
    assert_eq!(f.function("file").unwrap().shape, Shape::Callable { params: vec![], gives: Gives::Answered(vec![n("uint32_t"), STy::Enum("Clock".into())]) });
    assert!(matches!(&f.function("bad").unwrap().shape, Shape::NotCallable(why) if why.to_string().contains("LUMA_E_NOWHERE")));
    // `an` before a vowel; a kind the event enum lacks, or none said, is not callable.
    assert!(matches!(&f.function("image").unwrap().shape, Shape::Callable { gives: Gives::Request { kind, .. }, .. } if kind == "image"));
    assert!(matches!(&f.function("sound").unwrap().shape, Shape::NotCallable(why) if why.to_string().contains("SOUND")));
    assert!(matches!(&f.function("quiet").unwrap().shape, Shape::NotCallable(why) if why.to_string().contains("event follows")));
    // LumaColor is `color`.
    assert_eq!(
        f.function("paint").unwrap().shape,
        Shape::Callable { params: vec![v("timer", STy::Handle("Timer".into())), v("rgba", STy::Color)], gives: Gives::Outs(vec![]) }
    );
}

#[test]
fn what_is_not_the_form_is_named_by_line() {
    let e = Header::read("typedef struct A {\n    uint32_t size;\n} B;\n").unwrap_err();
    assert_eq!((e.line, e.why.as_str()), (3, "typedef'd as B, not as its tag A"));
    let e = Header::read("int x;\n").unwrap_err();
    assert_eq!(e.line, 1);
    let e = Header::read("typedef struct A {\n    int32_t (*f)(const LumaMod*);\n} A;\n").unwrap_err();
    assert_eq!((e.line, e.why.as_str()), (2, "f: a parameter with no name"));
    let e = Header::read("/* open").unwrap_err();
    assert_eq!(e.why, "a comment that does not end");
    let no_table = Face::of("demo", &Header::read("").unwrap(), &header("luma_services.h"), &["demo"]).unwrap_err();
    assert_eq!(no_table.0, "no table LumaDemoService");
}

#[test]
fn case_names() {
    assert_eq!(case_name("NOT_FOUND"), "NotFound");
    assert_eq!(case_name("D3D12"), "D3d12");
    assert_eq!(case_name("X64"), "X64");
}

#[test]
fn content_fx_as_j_wrote_it() {
    // luma_content.h and luma_content_fx.h at j/3.5-fx 8e04bb177, read with luma_content.h shared.
    let content = header("luma_content.h");
    assert_eq!(content.record("LumaPlace").unwrap().members.len(), 3);
    let f = Face::of_with("content.fx", &header("luma_content_fx.h"), &[&header("luma_services.h"), &content], &["content.fx"]).unwrap();
    assert_eq!(f.constants["MOST"], 256);
    assert_eq!(f.handles, vec!["Effect"]);
    assert_eq!(f.shared_structs["LumaPlace"], vec![("offset".to_string(), STy::Vec(3)), ("turn".to_string(), STy::Vec(3))]);
    // A handle the call opens at once is an out: start gives content.fx.Effect.
    assert_eq!(
        f.function("start").unwrap().shape,
        Shape::Callable {
            params: vec![
                v("actor", STy::Actor),
                v("name", STy::ContentName("effect".into())),
                v("joint", STy::Opt(Box::new(STy::Text))),
                v("place", STy::Opt(Box::new(STy::Shared("LumaPlace".into())))),
                v("follow", STy::Bool),
            ],
            gives: Gives::Outs(vec![STy::Handle("Effect".into())]),
        }
    );
    // every member names LUMA_OK.
    for name in ["start", "start_on_model", "stop", "tint"] {
        assert!(f.function(name).unwrap().statuses.contains(&"LUMA_OK".to_string()), "{name}");
    }
    assert_eq!(f.function("stop").unwrap().shape, Shape::Callable { params: vec![v("effect", STy::Handle("Effect".into())), v("now", STy::Bool)], gives: Gives::Outs(vec![]) });
    // tint's colour is a number until the header takes LumaColor.
    assert_eq!(
        f.function("tint").unwrap().shape,
        Shape::Callable { params: vec![v("effect", STy::Handle("Effect".into())), v("scale", n("float")), v("rgba", n("uint32_t"))], gives: Gives::Outs(vec![]) }
    );
}

#[test]
fn shared_types_and_nullable_parameters() {
    let shared = Header::read("typedef void* LumaActor;\ntypedef const char* LumaEffectName;\ntypedef struct LumaPlace {\n    uint32_t size;\n    float offset[3];\n} LumaPlace;\n").unwrap();
    let h = Header::read(r#"
typedef struct LumaDemoService {
    LumaServiceTable base;
    /* 1.0. Game thread. `joint` (null: the base), `place` (null: none). LUMA_OK; LUMA_E_ARGUMENT. */
    int32_t (*show)(const LumaMod* mod, LumaActor actor, LumaEffectName name, const char* joint, const LumaPlace* place);
} LumaDemoService;
typedef struct LumaDemoSpot {
    uint32_t size;
    float at[3];
    float w[5];
} LumaDemoSpot;
"#).unwrap();
    let f = Face::of_with("demo", &h, &[&header("luma_services.h"), &shared], &["demo"]).unwrap();
    assert_eq!(
        f.function("show").unwrap().shape,
        Shape::Callable {
            params: vec![
                v("actor", STy::Actor),
                v("name", STy::ContentName("effect".into())),
                v("joint", STy::Opt(Box::new(STy::Text))),
                v("place", STy::Opt(Box::new(STy::Shared("LumaPlace".into())))),
            ],
            gives: Gives::Outs(vec![]),
        }
    );
    assert_eq!(f.structs["Spot"], vec![("at".to_string(), STy::Vec(3))]);
    assert_eq!(f.unreadable, vec![("Spot.w".to_string(), "an array of 5 float".to_string())]);
}

#[test]
fn a_service_set_from_the_sdk_headers() {
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_time.h", "luma_system.h", "luma_mods.h", "luma_content.h", "luma_content_fx.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{}/tests/data/svc/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap());
    }
    let catalog = ["time", "system", "system.clipboard", "system.links", "mods", "mods.ids", "mods.install", "content.fx"];
    let s = Services::read(&catalog, &headers).unwrap();
    assert_eq!(s.faces.len(), 8);
    assert!(s.faces["content.fx"].shared_structs.contains_key("LumaPlace"));
    assert_eq!(s.faces["mods.install"].events.keys().collect::<Vec<_>>(), vec!["done"]);
    let names: Vec<&str> = s.answers.iter().map(|a| a.0.as_str()).collect();
    assert_eq!(names, vec!["Denied", "Absent", "NotFound", "Exists", "Io", "Timeout", "Closed", "Memory", "Busy", "Format", "TooLarge", "Unreachable"]);
    assert_eq!(s.answers.iter().find(|a| a.0 == "Unreachable").unwrap().1, -17);
    let e = Services::read(&["net"], &headers).unwrap_err();
    assert_eq!(e.0, "net: no luma_net.h");
}

/// LumaLink's mirror test: every function of today's headers is in the one form; a function that is not
/// callable is so for a parameter of no LumaScript type, never for the form.
#[test]
fn the_headers_are_in_the_one_form() {
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_time.h", "luma_system.h", "luma_mods.h", "luma_content.h", "luma_content_fx.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{}/tests/data/svc/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap());
    }
    let catalog = ["time", "system", "system.clipboard", "system.links", "mods", "mods.ids", "mods.install", "content.fx"];
    let s = Services::read(&catalog, &headers).unwrap();
    for (name, face) in &s.faces {
        assert!(face.unreadable.is_empty(), "{name}: {:?}", face.unreadable);
        for f in &face.functions {
            assert!(!matches!(&f.shape, Shape::NotCallable(Unfit::Form(_))), "{name}.{}: {:?}", f.name, f.shape);
        }
    }
}

/// An enum value is a constant expression (J 2026-11-12: roll 4's `LUMA_INPUT_BUTTONS_ALL = (1u << 13) - 1u`), and a
/// section comment (`/* ---- ... ---- */`) is every function's below it (draw.rect_outline's statuses are its
/// section's).
#[test]
fn enum_values_fold_and_sections_carry_statuses() {
    let text = "enum {\n    LUMA_DEMO_A = 1u << 4,\n    LUMA_DEMO_ALL = (1u << 13) - 1u,\n    LUMA_DEMO_MIX = (1 | 2) << 2,\n    LUMA_DEMO_NEG = -(3 + 4)\n};\n\n#define LUMA_DEMO_NAME \"demo\"\n\ntypedef struct LumaDemoService {\n    LumaServiceTable base;\n    /* ---- 2-D ---- LUMA_OK. A null pointer: LUMA_E_ARGUMENT, reported. */\n    /* 1.0. One. */\n    int32_t (*one)(const LumaMod* mod, float a);\n    /* 1.0. Two, `a` above 0. */\n    int32_t (*two)(const LumaMod* mod, float a);\n} LumaDemoService;\n";
    let h = Header::read(text).unwrap_or_else(|e| panic!("{e}"));
    let cases: Vec<(String, i64)> = h.enums.iter().flat_map(|e| e.cases.iter().map(|c| (c.0.clone(), c.1))).collect();
    assert_eq!(cases, vec![("LUMA_DEMO_A".into(), 16), ("LUMA_DEMO_ALL".into(), 8191), ("LUMA_DEMO_MIX".into(), 12), ("LUMA_DEMO_NEG".into(), -7)]);
    let f = Face::of("demo", &h, &header("luma_services.h"), &["demo"]).unwrap();
    for name in ["one", "two"] {
        assert!(matches!(f.function(name).unwrap().shape, Shape::Callable { .. }), "{name}: {:?}", f.function(name).unwrap().shape);
    }
    assert!(Header::read("enum {\n    LUMA_DEMO_X = (1 << 70)\n};\n").is_err());
}

/// Order 1 (C1h): every header of the SDK the game ships (`LUMA_SDK`, the folder of luma.h) reads, and its services
/// are 32 faces with no error: a header the reader refuses takes its services away from every script.
#[test]
fn every_sdk_header_reads() {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("the SDK's headers are not read: LUMA_SDK is not set");
        return;
    };
    let mut headers = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(&sdk).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with("luma") && name.ends_with(".h") {
            headers.insert(name, std::fs::read_to_string(&p).unwrap());
        }
    }
    let (s, errors) = Services::of_sdk(&headers);
    if std::env::var_os("LUMA_SDK_LIST").is_some() {
        for (name, face) in &s.faces {
            for f in &face.functions {
                if let Shape::NotCallable(why) = &f.shape {
                    eprintln!("{name}.{}: {why}", f.name);
                }
            }
        }
    }
    // Order 4 (C1g): every `typedef LumaHandle X;` of the SDK is a handle kind of the service it is named for, the
    // service's last word when X is the prefix itself (LumaMovie, LumaContentModel, LumaDrawInst, LumaDrawLight).
    let mut handles = 0;
    for text in headers.values() {
        for line in text.lines() {
            let Some(name) = line.trim().strip_prefix("typedef LumaHandle ").and_then(|r| r.strip_suffix(';')) else { continue };
            handles += 1;
            let found = s.faces.values().any(|f| {
                let prefix = format!("Luma{}", lumascript::svc::camel(&f.name));
                let last = lumascript::svc::camel(f.name.rsplit('.').next().unwrap());
                f.handles.iter().any(|k| name == format!("{prefix}{k}") || (name == prefix && *k == last))
            });
            assert!(found, "{name}: no face has it as a handle kind");
        }
    }
    assert!(handles >= 14, "{handles} handle typedefs");
    // assets.cook: its event kind is the enum LumaAssetsEvent (it was a #define the reader could not take kinds from).
    if let Some(assets) = s.faces.get("assets") {
        assert!(assets.events.contains_key("cooked"), "{:?}", assets.events);
        assert!(matches!(&assets.function("cook").unwrap().shape, Shape::Callable { gives: Gives::Request { kind, .. }, .. } if kind == "cooked"), "{:?}", assets.function("cook").unwrap().shape);
    }
    assert!(errors.is_empty(), "{}", errors.iter().map(|e| e.0.clone()).collect::<Vec<_>>().join("\n"));
    // 33 with player (luma_player.h, svc.player 1.0).
    assert_eq!(s.faces.len(), 33, "{:?}", s.faces.keys().collect::<Vec<_>>());
    assert!(s.faces.contains_key("player"), "{:?}", s.faces.keys().collect::<Vec<_>>());
}

/// A `#define`'s block comment may go on over lines (luma_assets.h's event kind); its value is still the number, and
/// the comment's words (an apostrophe among them) are never read as code. A define's value may be a constant
/// expression (`(256ull * 1024ull * 1024ull)`).
#[test]
fn a_define_with_a_long_comment_and_a_folded_value() {
    let text = "#define LUMA_DEMO_EVENT_X 1u /* the cooker's words,\n   on two lines */\n#define LUMA_DEMO_MOST (256ull * 1024ull * 1024ull)\n#define LUMA_DEMO_SUM (1u << 4) + 2 // a line comment\n#define LUMA_DEMO_NOTE \"a\"\nenum { LUMA_DEMO_Y = 3 };\n";
    let h = Header::read(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.numbers["LUMA_DEMO_EVENT_X"], 1);
    assert_eq!(h.numbers["LUMA_DEMO_MOST"], 256 * 1024 * 1024);
    assert_eq!(h.numbers["LUMA_DEMO_SUM"], 18);
    assert_eq!(h.texts["LUMA_DEMO_NOTE"], "a");
    assert_eq!(h.enums[0].cases[0].1, 3);
}

/// An array's length may be a `#define` of the header (`const char* names[LUMA_MODLOG_ARGS]`), in a field and in a
/// parameter; a function-pointer parameter (`void (*fn)(void* user)`) reads, and only its function is not callable.
#[test]
fn macro_lengths_and_function_pointer_parameters() {
    let text = "#define LUMA_DEMO_NAME \"demo\"\n#define LUMA_DEMO_MAJOR 1u\n#define LUMA_DEMO_MINOR 0u\n#define LUMA_DEMO_ARGS 8u\ntypedef struct LumaDemoRow {\n    uint32_t count;\n    const char* names[LUMA_DEMO_ARGS];\n} LumaDemoRow;\ntypedef struct LumaDemoService {\n    LumaServiceTable base;\n    /* 1.0. LUMA_OK. */\n    int32_t (*other)(const LumaMod* mod, void (*fn)(void* user), void* user);\n    /* 1.0. LUMA_OK. */\n    int32_t (*rows)(const LumaMod* mod, const float m[LUMA_DEMO_ARGS]);\n    /* 1.0. LUMA_OK. */\n    int32_t (*plain)(const LumaMod* mod, uint32_t x);\n} LumaDemoService;\n";
    let h = Header::read(text).unwrap_or_else(|e| panic!("{e}"));
    let row = h.record("LumaDemoRow").unwrap();
    assert!(row.members.iter().any(|m| matches!(m, lumascript::svc::Member::Field(d) if d.name == "names" && d.array == Some(8))));
    let f = Face::of("demo", &h, &header("luma_services.h"), &["demo"]).unwrap();
    assert!(matches!(&f.function("other").unwrap().shape, Shape::NotCallable(Unfit::Param { param, why: ParamWhy::Type, .. }) if param == "fn"), "{:?}", f.function("other").unwrap().shape);
    assert_eq!(f.function("rows").unwrap().c.params[1].array, Some(8));
    assert!(matches!(f.function("plain").unwrap().shape, Shape::Callable { .. }));
    assert!(Header::read("typedef struct LumaDemoRow {\n    const char* names[LUMA_UNKNOWN];\n} LumaDemoRow;\n").is_err());
}

/// A C name belongs to the service whose prefix it starts with as a word of its own: `LumaInputInjection` is input's
/// `Injection`, not input.inject's `ion`; `LumaInputInjectStep` is input.inject's `Step`.
#[test]
fn a_prefix_ends_at_a_word() {
    let dir = format!("{}/tests/data/svc/c3", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_input.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let input = headers["luma_input.h"].replace(
        "typedef struct LumaInputInjectService {",
        "typedef LumaHandle LumaInputInjection;\ntypedef LumaHandle LumaInputInjectStep;\n\ntypedef struct LumaInputInjectService {",
    );
    headers.insert("luma_input.h".into(), input);
    let (s, errors) = Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(s.faces["input"].handles.contains(&"Injection".to_string()), "{:?}", s.faces["input"].handles);
    assert_eq!(s.faces["input.inject"].handles, vec!["Step".to_string()]);
    assert_eq!(s.faces["input.inject"].foreign_structs["LumaInputGame"], ("input".to_string(), "Game".to_string()));
}
