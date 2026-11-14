//! The back end against a small game: the C it writes for constructs the game itself has too few of to
//! prove in a run (an enum's bit-field of unsigned storage, a bit-field over 32 bits), and for calls through `?.`
//!. Each case reads the generated C; the game runs prove the rest.

use luma_validate::names::{signature_params, FunctionFacts, Types};
use lumascript::check::{self, ModFacts, Source};
use lumascript::diag::{code, Arg};
use lumascript::emit;
use lumascript::game::Game;
use std::collections::HashMap;

const GAME: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 8, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "Kuribo", "size": 96, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "Actor", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "class", "name": "Actor", "size": 64, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": [
      {"name": "mMode", "type": "Mode : 3", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": {"width": 3, "offset": 64}},
      {"name": "mKind", "type": "Kind : 3", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": {"width": 3, "offset": 67}},
      {"name": "mWide", "type": "u64 : 40", "offset": 16, "count": 1, "extents": null, "access": "public", "bits": {"width": 40, "offset": 128}},
      {"name": "mCount", "type": "s32", "offset": 24, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mName", "type": "const char *", "offset": 32, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mBox", "type": "Box", "offset": 40, "count": 1, "extents": null, "access": "public", "bits": null}],
     "virtuals": [{"name": "Actor::control", "signature": "void()", "pure": false, "row": true}, {"name": "Actor::receive", "signature": "bool(s32)", "pure": false, "row": true}]},
    {"kind": "class", "name": "LiveActor", "size": 64, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": []},
    {"kind": "class", "name": "PlanetGravity", "size": 48, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [],
     "virtuals": [{"name": "PlanetGravity::calcOwnGravityVector", "signature": "bool(Vec3f *, f32 *, const Vec3f &) const", "pure": false, "row": true}]},
    {"kind": "struct", "name": "Box", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "mW", "type": "s32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null},
      {"name": "mH", "type": "f32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Vec3<f32>", "size": 12, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": [
      {"name": "x", "type": "f32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "y", "type": "f32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null}, {"name": "z", "type": "f32", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "alias", "name": "Vec3f", "of": "Vec3<f32>", "record": "Vec3<f32>"},
    {"kind": "alias", "name": "MtxPtr", "of": "f32 (*)[4]", "record": null},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "alias", "name": "u32", "of": "unsigned int", "record": null},
    {"kind": "alias", "name": "u64", "of": "unsigned long long", "record": null},
    {"kind": "alias", "name": "f32", "of": "float", "record": null},
    {"kind": "enum", "name": "Mode", "underlying": "int", "scoped": true, "values": [{"name": "Walk", "value": 0}, {"name": "Last", "value": 3}]},
    {"kind": "enum", "name": "Kind", "underlying": "unsigned int", "scoped": true, "values": [{"name": "None", "value": 0}, {"name": "Top", "value": 5}]}
  ],
  "globals": [],
  "left_out": []
}"#;

const FUNCTIONS: &[(&str, &str, &str)] = &[
    ("MR::actor", "Actor *()", "free"),
    ("Actor::isAlive", "bool() const", "method"),
    ("Actor::kill", "void()", "method"),
    ("Actor::count", "s32(s32)", "method"),
    ("Actor::next", "Actor *()", "method"),
    // Virtuals a mod's class overrides (docs/modding/language/language.md, section 5.5).
    ("Actor::control", "void()", "method"),
    ("Actor::receive", "bool(s32)", "method"),
    ("MR::stageName", "const char *()", "free"),
    ("MR::isEqualString", "bool(const char *, const char *)", "free"),
    ("MR::useBox", "void(Box)", "free"),
    ("MR::makeBox", "Box(s32)", "free"),
    ("MR::readBox", "void(const Box &)", "free"),
    ("MR::say", "void(const char *)", "free"),
    ("MR::sayTwo", "void(s32, const char *, const char *)", "free"),
    ("MR::lookup", "const char16_t *(const char *, const char **)", "free"),
    ("MR::play", "void(const Actor *, const char *)", "free"),
    ("MR::keepBox", "void(Actor *, Box *)", "free"),
    ("MR::calcHost", "bool(const Actor *, const void *)", "free"),
    ("MR::up", "const Vec3f &()", "free"),
    ("MR::gravityAt", "bool(const Actor *, Vec3f *)", "free"),
    ("MR::scaleBy", "void(const f32 *)", "free"),
    ("MR::setMtx", "void(MtxPtr)", "free"),
    ("MR::pullAt", "bool(const Actor *, Vec3f *, f32 *)", "free"),
    // `text "Key"` reads the mod's merged line through MessageData.
    ("MR::getGameMessageDirect", "const char16_t *(const char *)", "free"),
];

/// The parameter marks of the rows that have any; every other row's are all null. `MR::say` and `MR::sayTwo` read their text
/// during the call only (`in`); `MR::keepBox` keeps its box, `MR::calcHost` compares its host, both taking null.
const MARKS: &[(&str, &str)] = &[
    ("MR::say", r#"{"way": "in", "count": null, "fixed": null, "nullable": false}"#),
    ("MR::sayTwo", r#"null, {"way": "in", "count": null, "fixed": null, "nullable": true}, null"#),
    ("MR::keepBox", r#"null, {"way": "kept", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::calcHost", r#"null, {"way": "in", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::gravityAt", r#"null, {"way": "out", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::scaleBy", r#"{"way": "in", "count": null, "fixed": null, "nullable": true}"#),
    ("MR::setMtx", r#"{"way": "in", "count": null, "fixed": null, "nullable": false}"#),
];

fn game() -> Game {
    let mut functions: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (name, sig, _) in FUNCTIONS {
        functions.entry(name.to_string()).or_default().push((sig.to_string(), false));
    }
    let mut game = Game::new(functions, Some(Types::from_json(GAME.as_bytes()).unwrap_or_else(|e| panic!("{e}"))));
    let rows: Vec<String> = FUNCTIONS
        .iter()
        .map(|(name, sig, kind)| {
            let owner = if *kind == "method" { "\"Actor\"" } else { "null" };
            let konst = sig.ends_with(" const");
            let marks = MARKS.iter().find(|(n, _)| n == name).map_or_else(|| vec!["null"; signature_params(sig).unwrap()].join(", "), |(_, m)| m.to_string());
            let is_virtual = matches!(*name, "Actor::control" | "Actor::receive");
            format!(r#"{{"name": "{name}", "signature": "{sig}", "kind": "{kind}", "owner": {owner}, "virtual": {is_virtual}, "pure": false, "const": {konst}, "overrides": [], "marks": [{marks}]}}"#)
        })
        .collect();
    game.facts = Some(FunctionFacts::from_json(format!(r#"{{"functions": [{}]}}"#, rows.join(", ")).as_bytes()).unwrap_or_else(|e| panic!("{e}")));
    game
}

/// The generated C of one module (`main`), or the back end's S0800 arguments.
fn build(text: &str) -> Result<String, Vec<String>> {
    build_file(text, "lum/main.c")
}

/// One generated file of the mod.
fn build_file(text: &str, file: &str) -> Result<String, Vec<String>> {
    let (module, diags) = lumascript::parse_module(text);
    assert!(diags.is_empty(), "does not parse: {diags:?}");
    // As a build gives them: the standard module after the mod's own.
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        match emit::emit("small", &sources, &checked, &game) {
            Ok(out) => {
                compiles(&out.files);
                Ok(out.files.iter().find(|(p, _)| p == file).map(|(_, c)| c.clone()).unwrap())
            }
            Err(diags) => Err(diags
                .iter()
                .map(|(_, d)| {
                    assert_eq!(d.code, code::BACKEND_NOT_YET);
                    d.args.iter().map(|(_, a)| match a {
                        Arg::Text(t) => t.clone(),
                        Arg::Phrase(p) => p.to_string(),
                    }).collect::<Vec<_>>().join(" ")
                })
                .collect()),
        }
    })
    .unwrap()
}

/// Every generated C file compiled to an object as `lumalink build` compiles it (`emit::C_FLAGS`, the frame limit with
/// them), when `LUMA_SDK` names luma.h's folder: reading the C alone passed C that did
/// not compile. A mod's own headers (`use c`) are not here; each is an empty file, the prototypes being in lm.h.
fn compiles(files: &[(String, String)]) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("the generated C is not compiled: LUMA_SDK is not set");
        return;
    };
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-emit-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let own = dir.join("source");
    std::fs::create_dir_all(&own).unwrap();
    for (rel, text) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    for (_, text) in files {
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("#include \"").and_then(|r| r.strip_suffix('"')) {
                if !files.iter().any(|(rel, _)| rel.rsplit('/').next() == Some(name)) {
                    std::fs::write(own.join(name), "").unwrap();
                }
            }
        }
    }
    for (i, (rel, _)) in files.iter().enumerate().filter(|(_, (rel, _))| rel.ends_with(".c")) {
        let out = std::process::Command::new(&clang)
            .current_dir(&dir)
            .arg("--target=x86_64-pc-windows-msvc")
            .args(emit::C_FLAGS)
            .arg("-I").arg(dir.join("lum")).arg("-I").arg(&sdk).arg("-I").arg(&own)
            .arg("-c").arg(rel).arg("-o").arg(dir.join(format!("{i}.o")))
            .output()
            .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
        let text = files.iter().find(|(r, _)| r == rel).map(|(_, c)| c.as_str()).unwrap_or("");
        assert!(out.status.success(), "{rel} does not compile:\n{}\n{text}", String::from_utf8_lossy(&out.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// an enum's bit-field takes its signedness from the enum's storage. `Kind` is unsigned: 5 in 3 bits reads as
/// 5 (no sign extension) and its write is checked against 0..7; `Mode` is signed: sign-extended, checked -4..3.
#[test]
fn an_enum_bit_field_is_signed_as_its_storage() {
    // The checker gives an enum's bit-field its storage's integer type (game.rs `field`): Kind's u32, Mode's i32.
    let c = build("on start {\n    guard let a = MR::actor() else { return }\n    a.mKind = 5\n    let k = a.mKind\n    a.mMode = 3\n    let m = a.mMode\n    log \"{k} {m}\"\n}\n").unwrap();
    let kind_write = c.find("LUMA_BITS_SET(").expect("a bit-field write");
    let before_kind = &c[..kind_write];
    assert!(before_kind.contains("<= 7ull"), "Kind's write checked as unsigned 3 bits:\n{c}");
    assert!(!before_kind.contains(">= -4LL"), "Kind's write not checked as signed:\n{c}");
    // Kind read: no shift pair; Mode read: shifted left then right by 29.
    let reads: Vec<&str> = c.lines().filter(|l| l.contains("LUMA_BITS_GET(")).collect();
    assert_eq!(reads.len(), 2, "{c}");
    assert!(!reads[0].contains("<< 29u"), "Kind is not sign-extended: {}", reads[0]);
    assert!(reads[1].contains("<< 29u") && reads[1].contains(">> 29"), "Mode is sign-extended: {}", reads[1]);
    assert!(c.contains(">= -4LL && ") && c.contains("<= 3LL"), "Mode's write checked as signed 3 bits:\n{c}");
}

/// a bit-field over 32 bits is not written (LUMA_BITS_GET/SET carry 32), never cut silently.
#[test]
fn a_bit_field_over_32_bits_is_refused() {
    let said = build("on start {\n    guard let a = MR::actor() else { return }\n    let w = a.mWide\n    log \"{w}\"\n}\n").unwrap_err();
    assert_eq!(said, vec!["a bit-field wider than 32 bits".to_string()]);
}

/// `a?.f()` calls nothing when `a` is none, gives none then, and an option of the result otherwise; on a
/// mod struct, `self` is the option's value (or its place's, for `var self`).
#[test]
fn a_call_through_question_dot_branches_on_presence() {
    let c = build(
        "struct Counter {\n    n: i32\n    fn twice(self) -> i32 { return self.n * 2 }\n    fn bump(var self) { self.n += 1 }\n}\n\
         on start {\n    let a = MR::actor()\n    let alive = a?.isAlive()\n    let n = a?.count(3)\n    a?.kill()\n    var c: Counter? = Counter { n: 4 }\n    let t = c?.twice()\n    c?.bump()\n    log \"{alive ?? false} {n ?? 0} {t ?? 0}\"\n}\n",
    )
    .unwrap();
    // Every game method call sits inside an `if (<object> != 0)`, with the object as args[0].
    let calls: Vec<usize> = c.match_indices("->invoke(").map(|(i, _)| i).collect();
    assert_eq!(calls.len(), 4, "MR::actor, isAlive, count, kill:\n{c}");
    for &i in &calls[1..] {
        let before = &c[..i];
        let guard = before.rfind(" != 0) {").expect("a presence test before the call");
        let close = before.rfind("\n        }").unwrap_or(0);
        assert!(guard > close, "the call is inside its presence test:\n{c}");
    }
    // The results are options: none in the else branch.
    let nones = c.match_indices("} else {\n").filter(|(i, _)| c[i + 9..].trim_start().starts_with('t')).count();
    assert!(nones >= 3, "each result is set to none when the object is none:\n{c}");
    // The struct's functions get the option's value: `.value` for self, `&<place>.value` for var self.
    assert!(c.contains(".some) {"), "a struct option is tested by .some:\n{c}");
    assert!(c.contains("lm_main__Counter__twice(t") && c.contains(".value, &t"), "twice gets the value:\n{c}");
    assert!(c.contains("lm_main__Counter__bump(&l"), "bump gets the place's value:\n{c}");
}

/// a left operand is read before the right one, whatever its type (a vector here).
#[test]
fn a_left_operand_is_read_first() {
    let c = build("var v = vec3(1, 2, 3)\nfn grow() -> vec3 {\n    v = vec3(9, 9, 9)\n    return v\n}\non start {\n    let s = v + grow()\n    log \"{s}\"\n}\n").unwrap();
    let start = c.find("#line 7").unwrap();
    let body = &c[start..];
    let read = body.find("= lm_g0_v;").expect("v read into a temporary");
    let call = body.find("lm_main__grow(").expect("grow called");
    assert!(read < call, "v is read before grow runs:\n{body}");
}

/// docs/modding/language/compiled-form.md, section 5: a text literal is a static block of the module counted LUMART_FOREVER, one per distinct bytes; every
/// variable of a counted type is written by `lm_cp_` and given back at `lm_out`; an interpolation makes a new text.
#[test]
fn text_is_counted_and_literals_are_static() {
    let c = build("on start {\n    var a = \"hi\"\n    let b = \"hi\"\n    a = \"{b} there\"\n    let n = a.len()\n    let same = a == b\n    let less = a < b\n    log \"{a} {n} {same} {less} {hash32(b)}\"\n}\n").unwrap();
    assert_eq!(c.matches("= {LUMART_FOREVER, 0, 2u, \"hi\"}").count(), 1, "one block for both \"hi\":\n{c}");
    assert!(c.find("static const struct").unwrap() < c.find("lm_init_0").unwrap(), "literals come before the code:\n{c}");
    assert!(c.contains("lm_cp_lumart_text(&l"), "a text local is written with its count taken:\n{c}");
    assert!(c.contains("lumart_text_from_sb(&t"), "an interpolation makes a text:\n{c}");
    let out = c.find("lm_out:").unwrap();
    assert!(c[out..].contains("lm_rel_lumart_text(&l"), "text locals are given back on the way out:\n{c}");
    assert!(c.contains("lumart_text_len(&t") && c.contains("lumart_text_eq(") && c.contains("lumart_text_cmp(") && c.contains("lumart_hash32("), "{c}");
}

/// A struct, option or tuple holding text gets its own ret/rel/cp, and those are what its places use.
#[test]
fn aggregates_holding_text_are_counted_whole() {
    let src = "struct Named {\n    name: text\n    n: i32\n}\nstruct Plain {\n    n: i32\n}\non start {\n    var x = Named { name: \"a\", n: 1 }\n    x = Named { name: \"{x.n}\", n: 2 }\n    let o: text? = x.name\n    let p = Plain { n: 3 }\n    log \"{x.name} {o ?? \"\"} {p.n} {x == x}\"\n}\n";
    let h = build_file(src, "lum/lm.h").unwrap();
    let c = build(src).unwrap();
    assert!(h.contains("static inline void lm_rel_lm_main__Named(lm_main__Named* p) {\n    lm_rel_lumart_text(&p->name);"), "{h}");
    assert!(h.contains("if (p->some) {\n        lm_ret_lumart_text(&p->value);"), "an option of text retains its value when present:\n{h}");
    assert!(!h.contains("lm_rel_lm_main__Plain"), "a struct of no text has no helpers:\n{h}");
    assert!(c.contains("lm_cp_lm_main__Named(&l"), "{c}");
    assert!(!c.contains("lm_cp_lm_main__Plain"), "{c}");
}

/// docs/modding/language/compiled-form.md: a game `const char *` read (a field, a result) is a new text, none for null; one passed is
/// the text's kept copy, null for none.
#[test]
fn game_text_crosses_by_copy_and_kept_copy() {
    let c = build("on start {\n    guard let a = MR::actor() else { return }\n    let name = a.mName ?? \"\"\n    let stage = MR::stageName()\n    let same = MR::isEqualString(name, stage ?? \"x\")\n    log \"{name} {same}\"\n}\n").unwrap();
    assert_eq!(c.matches("lumart_text_from_game(&t").count(), 2, "the field and the result:\n{c}");
    assert!(c.contains("(*(const char* const*)"), "the field's pointer read in place:\n{c}");
    assert_eq!(c.matches("lumart_text_keep(").count(), 2, "both arguments:\n{c}");
    assert!(c.contains("\"MR::isEqualString\", \"2\")"), "S0918 names the function and the argument:\n{c}");
}

/// `a?.f()` where `f` gives a view: the result is the pointer itself (none is null), never an option struct
/// written over a pointer.
#[test]
fn a_question_dot_call_giving_a_view_keeps_the_pointer() {
    let c = build("on start {
    let a = MR::actor()
    let n = a?.next()
    let has = n != none
    log \"{has}\"
}
").unwrap();
    assert!(!c.contains("(void*){1,"), "no option struct for a view:
{c}");
    let call = c.rfind("->invoke(").unwrap();
    let after = &c[call..];
    let set = after.find(" = t").expect("the result stored");
    assert!(!after[..set + 40].contains("{1,"), "the view stored as it is:
{c}");
    assert!(after.contains("} else {
") , "none written when a is none:
{c}");
}

/// `xs[i] = f(xs)` where `f` may keep a copy: the list made this variable's own again after the value is read and
/// before the store (a copy taken by the value must not see the write); the call's argument copy given back as soon
/// as the call returns, so that second check finds one owner when nothing kept it.
#[test]
fn a_write_into_a_container_checks_again_after_its_value() {
    let c = build("var saved: list<i32> = []\nfn keep(xs: list<i32>) -> i32 {\n    saved = xs\n    return 1\n}\non start {\n    var xs: list<i32> = [1, 2]\n    xs[0] = keep(xs)\n    log \"{xs}\"\n}\n").unwrap();
    let start = c.find("#line 8").unwrap();
    let body = &c[start..c[start..].find("#line 9").map_or(c.len(), |e| start + e)];
    let owns: Vec<usize> = body.match_indices("lumart_list_own(&l").map(|(i, _)| i).collect();
    let call = body.find("lm_main__keep(").unwrap_or_else(|| panic!("keep called:\n{c}"));
    let store = body.find(")) = t").expect("the element stored");
    assert_eq!(owns.len(), 2, "made its own before and after the value:\n{body}");
    assert!(owns[0] < call && call < owns[1] && owns[1] < store, "own, call, own, store:\n{body}");
    let given_back = body[call..].find("lm_rel_lumart_list(&t").map(|i| call + i).expect("the argument given back");
    assert!(given_back < owns[1], "the argument's count given back before the second check:\n{body}");
}

/// A tuple's parts are read where written: `(xs, xs.pop())` holds the list as it was before the pop.
#[test]
fn tuple_parts_are_read_in_order() {
    let c = build("on start {\n    var xs: list<i32> = [1, 2]\n    let t = (xs, xs.pop())\n    let (a, p) = t\n    let q = p ?? 0\n    log \"{a} {q}\"\n}\n").unwrap();
    let start = c.find("#line 3").unwrap();
    let body = &c[start..];
    let held = body.find("lm_cp_lumart_list(&t").expect("the list part held in a temporary");
    let pop = body.find("lumart_list_remove(&l").expect("the pop");
    assert!(held < pop, "the first part read before the pop:\n{body}");
}

/// K3b: a closure given to a list method is written in place: no function value, a label for its `return`; sort_by
/// is driven by lumart's sorter with the closure as the comparison.
#[test]
fn closures_of_list_methods_are_written_in_place() {
    let c = build("on start {\n    var xs: list<i32> = [3, 1, 2]\n    let k = 1\n    let big = xs.filter(fn(x: i32) -> bool { return x > k })\n    let any = xs.any(fn(x: i32) -> bool { return x == 2 })\n    xs.sort_by(fn(a: i32, b: i32) -> bool { return a > b })\n    log \"{big} {any} {xs}\"\n}\n").unwrap();
    assert_eq!(c.matches("goto lm_cl").count(), 3, "each closure's return leaves its body:\n{c}");
    assert!(c.contains("lumart_sorter_begin(&t") && c.contains("while (lumart_sorter_next(&t"), "{c}");
    assert!(c.contains(" && !t"), "any stops at the first true:\n{c}");
}

/// Bytes is a counted u8 block; `b[i] = x` and `write_*` make it this variable's own first; a read
/// works on a held copy given back at once; `write_*` takes (at, width, big, bits) in that order.
#[test]
fn bytes_are_a_counted_block_written_in_place() {
    let c = build("on start {\n    var b = \"ab\".bytes()\n    b[0] = 1\n    b.write_u32(0, 7, true)\n    let v = b.read_u32(0, false)\n    let f = b.read_f32(0, true)\n    let n = b.len()\n    log \"{n} {v} {f}\"\n}\n").unwrap();
    assert!(c.contains("lumart_bytes_of_text(&t"), "{c}");
    assert!(c.contains("lumart_list_own(&l") && c.contains(", &lumart_u8, 0u,"), "b[0] made its own:\n{c}");
    assert!(c.contains("lumart_bytes_p(l"), "the byte written in place:\n{c}");
    let w = c.find("lumart_bytes_write(&l").expect("write_u32");
    assert!(c[w..].starts_with("lumart_bytes_write(&l") && c[w..c[w..].find('\n').unwrap() + w].contains(", 4u, t") && c[w..].contains(", (uint64_t)t"), "at, width, big, bits:\n{c}");
    assert!(c.contains("lumart_f32_of(t"), "f32 from its bits:\n{c}");
    assert!(c.contains("lm_rel_lumart_bytes(&t"), "a read gives its copy back:\n{c}");
}

/// K4 (11.1): a task is a frame and a step function; each `wait` parks (step set, frame kept) and resumes at its own
/// label through the gotos at the top; `start` calls the start function, which takes the handle's address last.
#[test]
fn a_task_parks_at_each_wait_and_resumes_at_its_label() {
    let c = build("var n = 0\n\ntask count(times: i32, name: text) {\n    for i in 0..times {\n        n += 1\n        wait 2 frames\n    }\n    wait until n > 9\n    log \"{name} {n}\"\n}\n\non start {\n    let t = start count(3, \"a\")\n    wait t\n    t.stop()\n}\n").unwrap();
    assert!(c.contains("typedef struct lm_task_0_frame {\n    lumart_task_head h;"), "{c}");
    assert!(c.contains("static int32_t lm_task_0_step(void* lm_fp)"), "{c}");
    assert!(c.contains("int32_t lm_task_0(int32_t l") && c.contains(", lumart_task* lm_ret) {"), "the start function:\n{c}");
    assert!(c.contains("lm_cp_lumart_text(&lm_f->l"), "a counted parameter gets a count of its own:\n{c}");
    for k in 1..=2 {
        assert!(c.contains(&format!("case {k}u:\n        goto lm_w{k};")) && c.contains(&format!("lm_w{k}:;")), "wait {k}:\n{c}");
    }
    assert_eq!(c.matches("lm_park:").count(), 2, "one park per waiting body:\n{c}");
    assert!(c.contains("lumart_wait_frames((int64_t)(") && c.contains("if (lumart_now_frame < t"), "{c}");
    let until = c.find("lm_w2:;").unwrap();
    assert!(c[until..].contains("goto lm_park;"), "until's condition read after the label:\n{c}");
    assert!(c.contains("if (lm_task_0(") && c.contains(", &t"), "start passes the handle's address:\n{c}");
    assert!(c.contains("lumart_task_alive(t"), "wait on a task:\n{c}");
    assert!(c.contains("lumart_task_stop(t"), "t.stop():\n{c}");
    assert!(c.contains("lumart_task_first(t) == 1"), "a waiting on start runs its first step now:\n{c}");
    let m = build_file("var n = 0\n\ntask t() {\n    wait 1 seconds\n    n = 1\n}\n", "lum/mod.c").unwrap();
    assert!(m.contains("lumart_tasks_pass(now->frame, now->step);") && m.contains("lumart_tasks_run(lm_n);") && m.contains("lumart_tasks_stop_all();"), "{m}");
}

/// K4: `stop` in a task goes to its way out, where its defers run and its counts are given back.
#[test]
fn stop_runs_the_tasks_way_out() {
    let c = build("task t(s: text) {\n    defer log \"bye {s}\"\n    wait 1 frames\n    stop\n}\n").unwrap();
    let step = c.find("lm_task_0_step").unwrap();
    let out = c[step..].find("lm_out:\n").map(|i| step + i).unwrap();
    assert!(c[step..out].contains("goto lm_out;"), "{c}");
    assert!(c[out..].contains("lumart_log(") &&c[out..].contains("lm_rel_lumart_text(&l"), "defer and release at the way out:\n{c}");
}

/// A frame over 4 KiB does not link, so a statement's temporaries are used again by the statements after
/// it, so a long body declares what one statement needs, not one per use; each is zero where it is made.
#[test]
fn temporaries_are_used_again_after_their_statement() {
    let mut src = String::from("var n = 1\n\non start {\n");
    for i in 0..200 {
        src.push_str(&format!("    n = (n + {i}) * 2 % 1000\n    log \"{{n}}\"\n"));
    }
    src.push_str("}\n");
    let c = build(&src).unwrap();
    let top = &c[c.find("int32_t lm_on_start_0_0(void) {").unwrap()..];
    let top = &top[..top.find("#line").unwrap()];
    let decls = top.lines().filter(|l| l.ends_with(" = {0};") || l.contains("lumart_sb_init(")).count();
    assert!(decls < 12, "200 statements, a handful of temporaries:\n{top}");
}

/// K5 (docs/modding/language/language.md, section 13, docs/modding/language/compiled-form.md, section 9): `extern c fn` is a prototype in lm.h and a direct call (text and bytes as pointer and
/// length); `export c fn` is the script's function plus a C function of its own name, declared in lm_exports.h, which
/// copies a text argument into a text of the script's own and gives it back after the call.
#[test]
fn c_functions_cross_both_ways() {
    let src = "extern c fn c_len(s: text, b: bytes) -> i64\nextern c fn c_odd(x: i32) -> bool\n\nexport c fn twice(s: text, n: i32) -> i32 {\n    return s.len() * n\n}\n\non start {\n    let n = c_len(\"ab\", \"c\".bytes())\n    let o = c_odd(3)\n    let t = twice(\"x\", 2)\n    log \"{n} {o} {t}\"\n}\n";
    let h = build_file(src, "lum/lm.h").unwrap();
    assert!(h.contains("int64_t c_len(const char* p0, uint64_t p0_len, const uint8_t* p1, uint64_t p1_len);"), "{h}");
    assert!(h.contains("_Bool c_odd(int32_t p0);") && h.contains("#include \"lm_exports.h\""), "{h}");
    let x = build_file(src, "lum/lm_exports.h").unwrap();
    assert!(x.contains("int32_t twice(const char* p0, uint64_t p0_len, int32_t p1);"), "{x}");
    let c = build(src).unwrap();
    assert!(c.contains("c_len(lumart_text_p(t") && c.contains("lumart_bytes_p(t"), "{c}");
    assert!(c.contains("(uint8_t)(c_odd(t"), "{c}");
    assert!(c.contains("int32_t twice(const char* p0, uint64_t p0_len, int32_t p1) {"), "{c}");
    assert!(c.contains("lumart_text_make(&a0, p0, p0_len,") && c.contains("(void)lm_main__twice(a0, p1, &r);") && c.contains("lm_rel_lumart_text(&a0);"), "{c}");
    assert!(c.contains("if (lm_main__twice("), "the script calls its own function:\n{c}");
}

/// C gets a copy, so a `var` parameter on either side would lose its write-back without a word: refused.
#[test]
fn a_c_var_parameter_is_refused() {
    let said = build("extern c fn bump(var n: i32)\n\non start {\n    var n = 1\n    bump(var n)\n}\n").unwrap_err();
    assert!(said.contains(&"an extern C function's var parameter".to_string()), "{said:?}");
    let said = build("export c fn bump(var n: i32) {\n    n += 1\n}\n").unwrap_err();
    assert!(said.contains(&"an exported C function's var parameter".to_string()), "{said:?}");
}

/// A plain game struct read from the game is the script's copy (its size row's bytes in a counted block);
/// a field written into a copy writes the variable's own block (copy on write), never the game's object; a copy
/// passes by value as a pointer at its bytes, the game's object itself when the argument is one; a struct result is
/// made in a new block.
#[test]
fn a_game_struct_by_value_is_the_scripts_copy() {
    let src = "on start {\n    guard let a = MR::actor() else { return }\n    var b = a.mBox\n    b.mW = 3\n    b.mH += 1.5\n    let h = b.mH\n    MR::useBox(b)\n    MR::useBox(a.mBox)\n    let c = MR::makeBox(2)\n    let w = c.mW\n    let n = a.mBox.mW\n    log \"{h} {w} {n}\"\n}\n";
    let c = build(src).unwrap();
    assert!(c.contains("lumart_record_copy(&t"), "a.mBox copied:\n{c}");
    assert!(c.contains(", 1u, ") && c.contains("lumart_record_at(&"), "b written in its own block:\n{c}");
    assert!(c.contains(", 0u, "), "b read without a copy of its own:\n{c}");
    assert!(c.contains("lumart_record_new(&t"), "makeBox's result made in a new block:\n{c}");
    let i = build_file(src, "lum/imports.c").unwrap();
    assert!(i.contains("LUMA_SIZE(") && i.contains("\"Box\""), "the size row:\n{i}");
    // After `?.` too (k3-structs on int0000: the game's address was taken as a block, its count written into the
    // game's struct and given back at the way out).
    let q = build("on start {\n    guard let b = MR::actor()?.mBox else { return }\n    MR::useBox(b)\n}\n").unwrap();
    assert!(q.contains("lumart_record_copy(&l0, (const void*)(t"), "a?.mBox copied into b:\n{q}");
    assert!(!q.contains("lm_cp_lumart_bytes(&l0"), "the game's address is no block:\n{q}");
    // Through `?.` kept as an optional: copied when there is one.
    let o = build("on start {\n    let b = MR::actor()?.mBox\n    if let x = b { MR::useBox(x) }\n}\n").unwrap();
    assert!(o.contains("lumart_record_copy(&t"), "{o}");
}

/// json: a json.Value literal known when compiling is one static text in json.encode's form (a
/// colour's channels as f32s); one holding values is written into a builder when it runs.
#[test]
fn a_json_value_literal_is_static_or_made() {
    let c = build("on start {\n    let n = 3\n    let v: json.Value = {\"a\": [1, 2.5, true, none, #ff000080], \"b\": \"x\\ny\", \"c\": -0x10}\n    let w: json.Value = {\"n\": n, \"t\": \"n={n}\"}\n    log \"{v} {w}\"\n}\n").unwrap();
    assert!(c.contains(r#""{\"a\": [1, 2.5, true, null, [1.0, 0.0, 0.0, 0.5019608]], \"b\": \"x\\u000ay\", \"c\": -16}""#), "the static text:\n{c}");
    assert!(c.contains("lumart_sb_json_str(&t") && c.contains("lumart_text_from_sb(&t"), "w made when it runs:\n{c}");
}

/// json: json.decode<T> calls one reader per type, a struct's or enum's own function (a struct holding a list of
/// itself calls its own reader, and json.encode its own writer: no endless code); a missing field takes its default.
#[test]
fn json_reads_and_writes_by_one_function_per_type() {
    let src = "struct Node {\n    name: text\n    n: i32 = 7\n    kids: list<Node> = []\n}\n\nenum Move { Stand, Walk(speed: f32) }\n\non start {\n    let a = json.decode<Node>(\"\\{\\}\")\n    let b = json.decode<list<Node>>(\"[]\")\n    let m = json.decode<map<i32, Move>>(\"\\{\\}\")\n    let t = json.encode(a ?? Node { name: \"x\" })\n    let u = json.encode(m ?? {})\n    log \"{t} {u} {b == none}\"\n}\n";
    let c = build(src).unwrap();
    let readers = c.matches("int32_t lm_jd_").count();
    assert_eq!(readers, 4, "Node, list<Node>, map<i32, Move>, Move: one each:\n{c}");
    assert!(c.contains("LUMART_JR(lm_jd_"), "a nested struct by its reader:\n{c}");
    assert!(c.contains("lumart_jr_key_i64(r, "), "an integer key as encode writes it:\n{c}");
    assert!(c.contains("lumart_text_is(") && c.contains("lumart_jr_field(r, "), "fields by name:\n{c}");
    assert!(c.contains("int32_t lm_jw_") && c.contains("if (lm_jw_"), "a struct written by its writer:\n{c}");
    let node_writer = c.split("int32_t lm_jw_").find(|f| f.contains("\\\"kids\\\": ")).expect("Node's writer");
    assert!(node_writer.contains("if (lm_jw_"), "the kids by calls of the writer:\n{node_writer}");
}

/// json: json.Value's methods give options; `?.` reads its receiver once and calls the method on its value.
#[test]
fn json_value_methods_through_question_dot() {
    let c = build("on start {\n    let v: json.Value = {\"l\": [1, \"a\"]}\n    let n = v.get(\"l\")?.at(0)?.int() ?? -1\n    let t = v.get(\"l\")?.list() ?? []\n    let k = \"ab\".find(\"b\")\n    let s: text? = \"x\"\n    let l = s?.len() ?? 0\n    log \"{n} {t.len()} {k ?? 0} {l}\"\n}\n").unwrap();
    assert!(c.contains("lumart_json_get(") && c.contains("lumart_json_at(") && c.contains("lumart_json_int("), "{c}");
    assert!(c.contains("lumart_json_list(") && c.contains("lumart_text_len("), "{c}");
}

/// The docs/modding/language/classes.md's make (order 8): the place's fields and the count go to lumart_make with LiveActor's class
/// slot; the standard module is its own C file, lum/lm_std.c, and the whole compiles against the SDK.
#[test]
fn make_calls_the_kernel_with_the_place() {
    let text = "on start {
    let a = make(\"Kuribo\", MakePlace { at: vec3(1.0, 2.0, 3.0), args: [4] })
    var p = MakePlace { at: vec3(0.0, 0.0, 0.0), dead: true }
    p.scale = vec3(2.0, 2.0, 2.0)
    let b = make(\"K{a.len()}\", p, 3)
    log \"{a.len()} {b.len()} {p.rotation.y}\"
}
";
    let c = build(text).unwrap();
    assert_eq!(c.matches("lumart_make(").count(), 2, "{c}");
    assert!(c.contains(".rotation, ") && c.contains(".args, ") && c.contains(".dead, 1, lm_class_"), "{c}");
    assert!(c.contains(".dead, (int64_t)"), "the count given:
{c}");
    let std = build_file(text, "lum/lm_std.c").unwrap();
    assert!(std.contains("int32_t lm_init_1(void)"), "{std}");
}

/// Order 9 (C4, C6): a matrix by its rows, a row and a column (an index outside S0902), and bytes as text when they
/// are well-formed UTF-8.
#[test]
fn matrices_and_bytes_as_text() {
    let c = build("on start {\n    let m = mat34(vec4(1.0, 0.0, 0.0, 5.0), vec4(0.0, 1.0, 0.0, 6.0), vec4(0.0, 0.0, 1.0, 7.0))\n    let r = m.row(1)\n    let t = m.column(3)\n    let s = \"h\u{e9}\".bytes().text() ?? \"\"\n    log \"{r.w} {t.z} {s}\"\n}\n").unwrap();
    assert!(c.contains("(lm_mat34){{{"), "{c}");
    assert!(c.contains(" >= 3) && lumart_index_error(") && c.contains(" >= 4) && lumart_index_error("), "{c}");
    assert!(c.contains("lumart_bytes_text(&"), "{c}");
}

/// Order 9 (C6): `T()` is the struct of its defaults; a list's element and an element's field are written in place;
/// a handle to a mod's class where a handle to its base goes is the same handle.
#[test]
fn defaults_elements_and_handles_of_a_base() {
    let c = build("struct Pair { a: i32 = 1, b: text = \"x\" }\nclass W : Actor {\n    var n = 0\n}\n\nvar pairs: list<Pair> = []\n\non start {\n    pairs.push(Pair())\n    pairs[0] = Pair { a: 2 }\n    pairs[0].a = 3\n    let ws: list<ref<W>> = []\n    var bases: list<ref<Actor>> = []\n    for w in ws {\n        bases.push(w)\n    }\n    let one: ref<Actor>? = ws.first()\n    log \"{pairs[0].a} {pairs[0].b} {bases.len()} {one == none}\"\n}\n").unwrap();
    assert!(c.contains("lumart_list_push(&"), "{c}");
}

/// K1 (`nullable` where nothing else crosses): a kept address and a `void *` are left out with `none`, passed as
/// null; the arguments around them as usual.
#[test]
fn nullable_kept_and_void_pointers_go_as_null() {
    let c = build("on start {\n    if let a = MR::actor() {\n        MR::keepBox(a, none)\n        let b = MR::calcHost(a, (none))\n        log \"{b}\"\n    }\n}\n").unwrap();
    assert_eq!(c.matches(" = (void*)0;").count(), 2 + c.matches("[0] = (void*)0;").count(), "two nulls passed:\n{c}");
    assert_eq!(c.matches("->invoke(").count(), 3, "MR::actor, keepBox, calcHost:\n{c}");
}

/// Text made for one use (docs/modding/language/language.md, section 6.4, docs/modding/language/compiled-form.md, section 7.2): a log line, an interpolated text before its copy, and a parameter
/// marks `in` are written in the scratch and given back right after; the function gives back to its own mark on its
/// way out (a script error between). A literal goes as its static bytes; an unmarked parameter still gets a kept copy.
#[test]
fn text_for_one_use_is_made_in_the_scratch() {
    let c = build("on start {\n    let n = 3\n    let o: text? = \"o{n}\"\n    log \"n is {n}\"\n    MR::say(\"x{n}\")\n    MR::say(\"lit\")\n    MR::sayTwo(n, o ?? \"d\", \"k{n}\")\n    let same = MR::isEqualString(\"a{n}\", \"b\")\n    log \"{same}\"\n}\n").unwrap();
    assert!(c.contains("const uint64_t lm_sm = lumart_scratch_mark();"), "the function's mark:\n{c}");
    assert!(c.contains("lumart_scratch_release(lm_sm);\n    return lm_st;"), "given back on the way out:\n{c}");
    // Builders in the scratch: "o{n}", the two log lines, "x{n}", o's bytes, "k{n}" and "a{n}" (text values, then kept).
    assert_eq!(c.matches("lumart_sb_scratch(&").count(), 7, "{c}");
    assert_eq!(c.matches("lumart_sb_cstr(&").count(), 2, "x{{n}} and o ?? d:\n{c}");
    assert!(c.contains("\"MR::say\", \"1\")"), "S0918 names the function and the argument:\n{c}");
    assert!(c.contains("\"MR::sayTwo\", \"2\")"), "{c}");
    // Through lumart_text_keep: sayTwo's unmarked third argument and isEqualString's first (kept copies), "lit" and
    // "b" (their static bytes, nothing kept). A text? is refused for a `const char *` by the checker (S0067).
    assert_eq!(c.matches("lumart_text_keep(").count(), 2 + 2, "{c}");
    assert_eq!(c.matches("lumart_log(lumart_sb_c(&").count(), 2, "{c}");
    // Every mark taken is given back in the body (and lm_sm at the end).
    assert_eq!(c.matches(" = lumart_scratch_mark();").count() - 1, c.matches("lumart_scratch_release(t").count(), "{c}");
}

/// A mod's class (docs/modding/language/language.md, section 5.5, docs/modding/language/classes.md): its data struct and `made` (first values), each override a wrap's
/// body run on the class's objects with `self`'s data `lm_d`, `base.f(...)` the kernel's proceed, `return v` the call's
/// result, another object's field through `class_data`, and `define_class` before the modules' initialisers.
#[test]
fn a_class_is_defined_at_start_with_its_overrides() {
    let src = "class Walker : Actor made by Actor replaces Kuribo {\n    var steps = 0\n    var speed: f32 = 1.5\n    override fn control() {\n        steps += 1\n        if isAlive() { base.control() }\n    }\n    override fn receive(n: i32) -> bool {\n        if n > self.steps { return base.receive(n - 1) }\n        kill()\n        return true\n    }\n}\nhook before Actor::kill() {\n    log \"kill\"\n}\n";
    let c = build(src).unwrap();
    for want in [
        "typedef struct lm_main__Walker_d {",
        "d->steps = ((int32_t)0x0ull);",
        "lm_d = (lm_main__Walker_d*)luma_rt_core->class_data(luma_rt_self, call->args[0], lm_main__Walker_n);",
        "lm_d->steps",
        "*lm_proceeded = 1;",
        "= *lm_ret;",
        "luma_rt_core->define_class(luma_rt_self, &def);",
        "{\"Actor::control\", \"void()\", lm_main__Walker__control_ov, 0},",
        "\"Walker\", \"Actor\", \"Actor\", \"Kuribo\"",
    ] {
        assert!(c.contains(want) || build_file(src, "lum/lm.h").unwrap().contains(want), "no {want:?} in:\n{c}");
    }
    let mod_c = build_file(src, "lum/mod.c").unwrap();
    let define = mod_c.find("lm_main__Walker_define() != 0").expect("the define call");
    let init = mod_c.find("lm_init_0() != 0").expect("the initialiser");
    assert!(define < init, "{mod_c}");
    // A field of another object is read through class_data; what the back end cannot write yet is named.
    let other = build("class W : Actor {\n    var n = 0\n    override fn control() {\n        let w = self\n        n = w.n + 2\n    }\n}\n").unwrap();
    assert!(other.contains("luma_rt_core->class_data(luma_rt_self, t") && other.contains("lm_d->n = "), "{other}");
    assert_eq!(build("class W : Actor {\n    fn f() { }\n}\n").unwrap_err(), vec!["a class's own fn".to_string()]);
}

/// A class's state machine (docs/modding/language/language.md, section 11.2): kept in the class's data, run from `control` (an override the back
/// end adds when the class has none, else at the start of the class's own); a waiting state's task parks in its
/// frame there; `go` from the machine leaves the block, from the class's override only asks; `is_state` reads it.
#[test]
fn a_state_machine_runs_from_control_in_the_objects_data() {
    let machine = "states Phases for Ball {\n    state Fly {\n        enter { hits = 0 }\n        update { if isAlive() { go Burst } }\n    }\n    state Burst {\n        enter { hits += 1 }\n        wait 30 frames\n        kill()\n        stop\n    }\n    start Fly\n}\n";
    let added = build(&format!("class Ball : Actor {{\n    var hits = 0\n}}\n{machine}")).unwrap();
    let h = build_file(&format!("class Ball : Actor {{\n    var hits = 0\n}}\n{machine}"), "lum/lm.h").unwrap();
    for want in ["typedef struct lm_main__Phases_m {", "lm_main__Phases_f1 s1;", "lm_main__Phases_m lm_mach_0;"] {
        assert!(h.contains(want), "no {want:?} in lm.h:\n{h}");
    }
    for want in [
        "static int32_t lm_main__Ball__machines_ov(LumaCall* call, void* user) {",
        "{\"Actor::control\", \"void()\", lm_main__Ball__machines_ov, 0},",
        "lm_m->next = 1;",
        "case 1: rc = lm_main__Phases_t1(call, lm_d, lm_m, &lm_m->f.s1); break;",
        "lm_f->h.step = 1u;",
        "lm_m->stopped = 1;",
        "lm_d->hits",
    ] {
        assert!(added.contains(want), "no {want:?} in:\n{added}");
    }
    // The waits need the frame handler, which keeps lumart's clock.
    let mod_c = build_file(&format!("class Ball : Actor {{\n    var hits = 0\n}}\n{machine}"), "lum/mod.c").unwrap();
    assert!(mod_c.contains("lumart_tasks_pass("), "{mod_c}");
    // The class's own control: the machines run before its body; `go M.S` asks, `is_state` reads.
    let own = build(&format!("class Ball : Actor {{\n    var hits = 0\n    override fn control() {{\n        if is_state(Phases.Fly) and hits > 2 {{ go Phases.Burst }}\n        base.control()\n    }}\n}}\n{machine}")).unwrap();
    assert!(own.contains("    lm_main__Ball_machines(call);\n    uint8_t proceeded = 0;"), "{own}");
    assert!(own.contains("((&lm_d->lm_mach_0)->state == 0)") && own.contains("(&lm_d->lm_mach_0)->next = 1;"), "{own}");
    assert!(!own.contains("lm_main__Ball__machines_ov"), "{own}");
    // A waiting state holding text: if the object goes while it waits, the class's gone gives the parked frame's
    // text back (no script code runs); a machine whose waiting frames hold nothing counted needs no gone.
    let held = "class Ball : Actor {\n    var hits = 0\n}\nstates P for Ball {\n    state A {\n        let t = \"{hits}\"\n        wait 1 frames\n        log \"{t}\"\n    }\n    start A\n}\n";
    let c = build(held).unwrap();
    for want in [
        "static void lm_main__P_gone(lm_main__Ball_d* lm_d) {",
        "    if (!lm_m->running) return;",
        "        if (lm_f->h.step == 0) break;",
        "        lm_rel_lumart_text(&lm_f->",
        "static void lm_main__Ball_gone(void* object, void* data, void* user) {",
        "    lm_main__P_gone(d);",
        ", .gone = lm_main__Ball_gone};",
    ] {
        assert!(c.contains(want), "no {want:?} in:\n{c}");
    }
    assert!(!added.contains("_gone"), "{added}");
}

/// A class's fields may hold text and containers: `made` writes a counted first value with its count taken, `gone`
/// gives each back before the kernel frees the data (docs/modding/language/classes.md); a class holding none has no
/// gone and its LumaClassDef ends where it did.
#[test]
fn a_class_holding_text_gives_it_back_when_its_object_goes() {
    let c = build("class Ball : Actor {\n    var label = \"ball\"\n    var seen: list<i32>\n    var hits = 0\n    override fn control() {\n        seen.push(hits)\n        label = \"{label}!\"\n        base.control()\n    }\n}\n").unwrap();
    for want in [
        "lm_cp_lumart_text(&d->label, ",
        "static void lm_main__Ball_gone(void* object, void* data, void* user) {",
        "    lm_rel_lumart_text(&d->label);",
        ", .gone = lm_main__Ball_gone};",
    ] {
        assert!(c.contains(want), "no {want:?} in:\n{c}");
    }
    assert!(c.lines().any(|l| l.starts_with("    lm_rel_") && l.ends_with("(&d->seen);")), "{c}");
    assert!(!c.contains("lm_rel_int32_t(&d->hits)"), "{c}");
    let plain = build("class Ball : Actor {\n    var hits = 0\n}\n").unwrap();
    assert!(!plain.contains("gone"), "{plain}");
}

/// A library mod's `pub fn`s are exported (wrapper, `LumaModInfo.exports` sorted, `lumi1`
/// signatures in its interface file); a mod that needs it reads the interface and calls each through a kind 6 import
/// with the same signature: a number, text (copied by the export), the library's struct (size first) and enum (case
/// number and its fields' address), an option of its enum as the result, a literal default filled in by the caller.
/// Bytes cross deep too (copied into the receiver's memory), so `size` is exported and called like the rest.
#[test]
fn a_library_exports_and_a_caller_calls_through_kind_6() {
    let lib_text = "pub enum Gives { Coins(n: u32), Lives(n: u32) }\npub enum Attack { Spin, Trample }\npub struct Size { w: f32, h: f32 }\n\n\
        pub fn give(g: Gives, times: i32 = 1) -> i32 {\n    match g {\n        Gives.Coins(n) => return n as i32 * times\n        Gives.Lives(n) => return 0 - n as i32\n    }\n}\n\n\
        pub fn attack_of(msg: u32) -> Attack? {\n    if msg == 1 {\n        return Attack.Spin\n    }\n    return none\n}\n\n\
        pub fn area(s: Size, label: text) -> f32 {\n    log \"{label}\"\n    return s.w * s.h\n}\n\n\
        pub fn all(xs: list<i32>) -> i32 {\n    return xs.len()\n}\n\n\
        pub fn label(s: Size, tags: list<text>) -> text {\n    return \"{s.w} {tags}\"\n}\n\n\
        pub fn size(b: bytes) -> i32 {\n    return 0\n}\n";
    let (module, d) = lumascript::parse_module(lib_text);
    assert!(d.is_empty(), "{d:?}");
    let lib_sources = [Source { path: "kit/one".into(), module }];
    let game = game();
    let (lumi, exports, lib_files) = lumascript::on_compiler_stack(|| {
        let checked = check::check(&lib_sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().all(|(_, d)| lumascript::diag::is_note(d.code)), "{:?}", checked.diags);
        let out = emit::emit_with("libmod", "1.0.0", &lib_sources, &[lib_text.to_string()], &[], &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        (out.lumi.unwrap(), out.exports, out.files)
    })
    .unwrap();
    let names: Vec<&str> = exports.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["kit.one.all", "kit.one.area", "kit.one.attack_of", "kit.one.give", "kit.one.label", "kit.one.size"]);
    let sig = |n: &str| exports.iter().find(|(e, _)| e == n).unwrap().1.clone();
    assert_eq!(sig("kit.one.give"), lumascript::lumi::signature("kit.one.give(libmod.kit.one.Gives,i32)->i32"));
    assert!(lumi.contains(&format!("pub fn give(g: Gives, times: i32 = 1) -> i32  // {}", sig("kit.one.give"))), "{lumi}");
    assert!(lumi.contains(&format!("pub fn size(b: bytes) -> i32  // {}", sig("kit.one.size"))), "{lumi}");
    let mod_c = &lib_files.iter().find(|(p, _)| p == "lum/mod.c").unwrap().1;
    assert!(mod_c.contains("lm_exports, 6, 0};"), "{mod_c}");
    let exports_c = &lib_files.iter().find(|(p, _)| p == "lum/exports.c").unwrap().1;
    // A text row made before the game's messages exist stays a row across the call (the host test reads it later).
    assert!(exports_c.contains("lumart_text_cross(&a1, p1, lm_file, lm_line)"), "{exports_c}");
    assert!(exports_c.contains("p0_n < sizeof a0 ? p0_n : sizeof a0"), "{exports_c}");
    assert!(exports_c.contains("if (p0_case >= 2u)") && exports_c.contains("__builtin_memcpy(&a0.u.Lives, p0, sizeof a0.u.Lives)"), "{exports_c}");
    // Deep: a list copied into the library's memory with its own lm_in_<n>; a text result copied by the caller's.
    assert!(exports_c.contains("if (p0_n != (uint32_t)sizeof a0)") && exports_c.contains("(&a1, p1, lm_file, lm_line, lm_peer) != 0) goto lm_done;"), "{exports_c}");
    // The caller's copy function reads handles through the library's own table.
    assert!(exports_c.contains("if (lm_r == 0) lm_r = lm_in(lm_out, &lm_res, lm_file, lm_line, (const void*)lumart_ref_get);"), "{exports_c}");
    assert!(exports_c.contains("lm_rel_lumart_list(&a1);"), "{exports_c}");

    // The caller, from the interface alone.
    let modules = lumascript::lumi::read(&lumi).unwrap_or_else(|e| panic!("{e:?}\n{lumi}"));
    let libraries = [check::Library { id: "libmod".into(), modules }];
    let call_text = "use libmod.kit.one\n\non start {\n    let n = one.give(one.Gives.Coins(3))\n    let a = one.attack_of(1)\n    let s = one.area(one.Size { w: 2.0, h: 3.0 }, \"box\")\n    let k = one.all([1, 2])\n    let t = one.label(one.Size { w: 2.0, h: 3.0 }, [\"a\", \"b\"])\n    let z = one.size(\"ab\".bytes())\n    log \"{n} {s} {a == one.Attack.Spin} {k} {t} {z}\"\n}\n";
    let (module, d) = lumascript::parse_module(call_text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let needs = ["libmod".to_string()];
    let (files, imports) = lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &libraries, ModFacts { needs: &needs, ..ModFacts::of_game(&game) });
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let out = emit::emit_with("caller", "1.0.0", &sources, &[], &libraries, &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        assert!(out.lumi.is_none() && out.exports.is_empty());
        (out.files, out.imports)
    })
    .unwrap();
    let rows: Vec<(&str, &str)> = imports.iter().filter(|(_, _, k)| *k == "mod function").map(|(n, s, _)| (n.as_str(), s.as_str())).collect();
    assert_eq!(rows.len(), 6, "{imports:?}");
    for (n, s) in &rows {
        assert_eq!(*s, sig(n.strip_prefix("libmod/").unwrap()), "{n}");
    }
    let main = &files.iter().find(|(p, _)| p == "lum/main.c").unwrap().1;
    assert!(main.contains("(lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self), (lumart_caller.mod != 0 ? lumart_caller.file : \"main.lum\"), (lumart_caller.mod != 0 ? lumart_caller.line : 4u), "), "the first caller and its place pass through an export:
{main}");
    assert!(main.contains(".tag, (const void*)&"), "{main}");
    assert!(main.contains("(uint32_t)sizeof "), "{main}");
    // `times` left out: the caller fills in the literal 1.
    assert!(main.contains("((int32_t)(1))"), "{main}");
    // A deep result: this mod's own copy function passed with the place it fills.
    assert!(main.contains("int32_t (*)(void*, const void*, const char*, uint32_t, const void*)") && main.contains(", lm_in_"), "{main}");
    assert!(main.contains("(const void*)lumart_ref_get"), "{main}");

}

/// Service calls (docs/modding/language/language.md): `ui` read from its header (tests/data/svc/luma_ui.h). The table is a kind 5 import with its header included; a `text` goes as `const char*`
/// (lumart_text_cstr, a NUL inside is S0918), the handle outs come back, and a status with answers gives `T?` or `bool`.
/// A function with a closure but no LUMA_PENDING (`play`) is not callable, and the checker says so.
#[test]
fn a_service_is_called_through_its_kind_5_table() {
    let dir = format!("{}/tests/data/svc", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_ui.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(services.faces.keys().collect::<Vec<_>>(), ["ui"]);
    assert_eq!(services.faces["ui"].header, "luma_ui.h");
    assert_eq!(services.faces["ui"].version, "1.0");
    let text = "use svc.ui\n\nvar count: u32 = 0\n\non frame {\n    count += 1\n    if let s = ui.open(\"Hud\", ui.Layer.Pause) {\n        if let p = ui.pane(s, \"Count\") {\n            ui.set_text(p, \"x {count}\")\n            ui.set_visible(p, false)\n        }\n        ui.show(s, true)\n        ui.close(s)\n    }\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let file = |name: &str| out.files.iter().find(|(p, _)| p == name).map(|(_, c)| c.clone()).unwrap();
        let (h, imports, main) = (file("lum/lm.h"), file("lum/imports.c"), file("lum/main.c"));
        assert!(h.contains("#include \"luma_ui.h\""), "{h}");
        assert!(h.contains("extern const LumaUiService* lm_svc_0;"), "{h}");
        assert!(imports.contains("LUMA_SERVICE(lm_svc_0, \"ui\", \"1.0\")"), "{imports}");
        for call in ["lm_svc_0->open(luma_rt_self, ", "lm_svc_0->pane(luma_rt_self, ", "lm_svc_0->set_text(luma_rt_self, ", "lm_svc_0->close(luma_rt_self, "] {
            assert!(main.contains(call), "{call} in\n{main}");
        }
        assert!(main.contains("lumart_text_cstr("), "{main}");
        assert!(main.contains("== LUMA_OK) {"), "{main}");
        let mut files = out.files.clone();
        files.push(("lum/luma_ui.h".into(), headers["luma_ui.h"].clone()));
        compiles(&files);
    })
    .unwrap();
    // `ui.play` takes a LumaEventFn without LUMA_PENDING: not callable (S0135, why SvcForm).
    let (module, _) = lumascript::parse_module("use svc.ui\n\non frame {\n    if let s = ui.open(\"Hud\", ui.Layer.Hud) {\n        ui.play(s, \"In\", false)\n    }\n}\n");
    let sources = [Source { path: "main".into(), module }];
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().any(|(_, d)| d.code == code::SVC_NOT_CALLABLE), "{:?}", checked.diags);
    })
    .unwrap();
}

/// Lumakit rule 4: `caller()` inside an export is the calling mod (lumart_caller, else this mod), and `on mods.stopped(m)`
/// is called by the library's dependent_gone (docs/modding/lumalink/loader.md) with the same number for a mod taken out.
#[test]
fn a_library_knows_its_caller_and_hears_it_go() {
    let text = "var owners: list<u64> = []\n\npub fn mark() -> u32 {\n    owners.push(caller())\n    return owners.len() as u32\n}\n\non mods.stopped(gone) {\n    var kept: list<u64> = []\n    for o in owners {\n        if o != gone {\n            kept.push(o)\n        }\n    }\n    owners = kept\n}\n";
    let c = build_file(text, "lum/mod.c").unwrap_or_else(|e| panic!("{e:?}"));
    assert!(c.contains("static void lm_dependent_gone(const LumaMod* dependent) {"), "{c}");
    assert!(c.contains("lm_exports, 1, lm_dependent_gone}"), "{c}");
    let main = build(text).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(main.contains("lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self"), "{main}");
    assert!(main.contains("(uint64_t lm_gone)"), "{main}");
}

/// Language 5.4: function values. A closure where a value goes is a counted block with copies of the locals it reads
/// (one reading none, and a mod function used as a value, a static block); such values sit in locals, struct fields,
/// list elements and enum cases, cross library exports, and are called with the block first and their status passed on.
#[test]
fn function_values_are_counted_blocks_called_with_themselves() {
    let text = "pub enum Unlock { Always, Code(check: fn() -> bool) }\npub struct Rule { name: text, ok: fn(i32) -> bool }\n\nvar rules: list<Rule> = []\n\nfn big(n: i32) -> bool {\n    return n > 10\n}\n\npub fn add(limit: i32, label: text, u: Unlock) -> u32 {\n    let f = fn(n: i32) -> bool { return n > limit and label != \"\" }\n    rules.push(Rule { name: label, ok: f })\n    rules.push(Rule { name: \"big\", ok: big })\n    var n: u32 = 0\n    for r in rules {\n        if r.ok(limit + 1) {\n            n += 1\n        }\n    }\n    let always = fn() -> bool { return true }\n    if always() {\n        n += 1\n    }\n    match u {\n        Unlock.Code(c) => {\n            if c() {\n                n += 1\n            }\n        }\n        Unlock.Always => {}\n    }\n    return n\n}\n";
    let c = build(text).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(c.contains("lm_rel_lumart_text(&lm_env->c"), "a captured text is given back by the block's drop:\n{c}");
    assert!(c.contains("_mk(l"), "the block is made with the locals' values:\n{c}");
    assert!(c.contains("LUMART_FOREVER, 0, 0, (const void*)lm_cl_"), "a closure reading nothing is static:\n{c}");
    assert!(c.contains("_fv0_b"), "a mod function as a value is a static block:\n{c}");
    assert!(c.contains("->code))("), "a value is called through its block:\n{c}");
}

/// Language 11.1a: a task with a result. Its call starts it and gives `pending<T>`; its `return v` keeps the value
/// under its handle (lumart_task_result_keep); `wait p` polls through the pending's operations, and a pending whose task
/// was stopped (`p.stop()`) ends the waiting task as stopped. A path without a value is S-coded as a function's.
#[test]
fn a_task_with_a_result_gives_a_pending() {
    let text = "task twice(n: u32) -> u32 {\n    wait 1 frames\n    return n * 2\n}\n\non start {\n    let d = wait twice(3)\n    let p = twice(4)\n    p.stop()\n    let e = wait p\n    log \"{d} {e}\"\n}\n";
    let c = build(text).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(c.contains("lumart_task_result_keep(lm_f, &lm_resv, &lm_desc_"), "{c}");
    assert!(c.contains("&lumart_pending_own})"), "{c}");
    assert!(c.contains(".ops->poll(") && c.contains("== 2) { lm_f->h.stop = 1; goto lm_out; }"), "{c}");
    assert!(c.contains(".ops->stop("), "{c}");
    // A task giving a value on one path only.
    let (module, d) = lumascript::parse_module("task half(n: u32) -> u32 {\n    if n > 2 {\n        return n / 2\n    }\n}\n");
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().any(|(_, d)| d.code == code::MISSING_RETURN), "{:?}", checked.diags);
    })
    .unwrap();
}

/// 11.1a across mods: a library's `pub task f(...) -> T` is exported (its interface line `pub task`, its lumi1
/// signature with `pending<T>`); its export starts it and gives the caller the handle with the library's operations;
/// the caller's `wait` copies a counted result into its own memory with its own lm_in_<n>.
#[test]
fn a_library_task_gives_its_caller_a_pending() {
    let lib_text = "pub struct Ware { name: text, price: u32 }\n\npub task pick(wares: list<Ware>) -> Ware? {\n    wait 1 frames\n    if wares.len() == 0 {\n        return none\n    }\n    return wares[0]\n}\n";
    let (module, d) = lumascript::parse_module(lib_text);
    assert!(d.is_empty(), "{d:?}");
    let lib_sources = [Source { path: "shop".into(), module }];
    let game = game();
    let (lumi, exports_c) = lumascript::on_compiler_stack(|| {
        let checked = check::check(&lib_sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().all(|(_, d)| lumascript::diag::is_note(d.code)), "{:?}", checked.diags);
        let out = emit::emit_with("libmod", "1.0.0", &lib_sources, &[lib_text.to_string()], &[], &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        let ex = out.files.iter().find(|(p, _)| p == "lum/exports.c").unwrap().1.clone();
        (out.lumi.unwrap(), ex)
    })
    .unwrap();
    let sig = lumascript::lumi::signature("shop.pick(list<libmod.shop.Ware>)->pending<libmod.shop.Ware?>");
    assert!(lumi.contains(&format!("pub task pick(wares: list<Ware>) -> Ware?  // {sig}")), "{lumi}");
    assert!(exports_c.contains("lm_out->ops = &lumart_pending_own;"), "{exports_c}");
    let modules = lumascript::lumi::read(&lumi).unwrap_or_else(|e| panic!("{e:?}\n{lumi}"));
    let libraries = [check::Library { id: "libmod".into(), modules }];
    let call_text = "use libmod.shop\n\non start {\n    let w = wait shop.pick([shop.Ware { name: \"Mushroom\", price: 10 }])\n    if let x = w {\n        log \"{x.name}\"\n    }\n}\n";
    let (module, d) = lumascript::parse_module(call_text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let needs = ["libmod".to_string()];
    let main = lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &libraries, ModFacts { needs: &needs, ..ModFacts::of_game(&game) });
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let out = emit::emit_with("caller", "1.0.0", &sources, &[], &libraries, &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        out.files.iter().find(|(p, _)| p == "lum/main.c").unwrap().1.clone()
    })
    .unwrap();
    assert!(main.contains(".ops->poll(") && main.contains(", lm_in_"), "{main}");
}

/// A result-less `pub task`: exported as `pending<()>`; the call marks the task
/// (lumart_task_watch) and its end by itself keeps `()` (lumart_task_done), so `wait` goes on once it ended.
#[test]
fn a_result_less_task_gives_a_pending_of_nothing() {
    let lib_text = "pub task message(line: text) {\n    wait 1 frames\n    log \"{line}\"\n}\n\ntask local() {\n    wait 1 frames\n}\n\non start {\n    local()\n}\n\ntask both() {\n    wait local()\n    let p = local()\n    wait p\n}\n";
    let (module, d) = lumascript::parse_module(lib_text);
    assert!(d.is_empty(), "{d:?}");
    let lib_sources = [Source { path: "kit".into(), module }];
    let game = game();
    let (lumi, exports_c, lib_c) = lumascript::on_compiler_stack(|| {
        let checked = check::check(&lib_sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().all(|(_, d)| lumascript::diag::is_note(d.code)), "{:?}", checked.diags);
        let out = emit::emit_with("libmod", "1.0.0", &lib_sources, &[lib_text.to_string()], &[], &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        let ex = out.files.iter().find(|(p, _)| p == "lum/exports.c").unwrap().1.clone();
        let c = out.files.iter().find(|(p, _)| p == "lum/kit.c").unwrap().1.clone();
        (out.lumi.unwrap(), ex, c)
    })
    .unwrap();
    let sig = lumascript::lumi::signature("kit.message(text)->pending<()>");
    assert!(lumi.contains(&format!("pub task message(line: text)  // {sig}")), "{lumi}");
    assert!(exports_c.contains("lumart_task_watch(lm_t);"), "{exports_c}");
    assert!(lib_c.contains("lumart_task_done(lm_f, "), "{lib_c}");
    assert_eq!(lib_c.matches("lumart_task_watch(").count(), 3, "{lib_c}");
    let modules = lumascript::lumi::read(&lumi).unwrap_or_else(|e| panic!("{e:?}\n{lumi}"));
    let libraries = [check::Library { id: "libmod".into(), modules }];
    let call_text = "use libmod.kit\n\non start {\n    kit.message(\"Hi\")\n}\n\ntask t() {\n    wait kit.message(\"Bye\")\n}\n";
    let (module, d) = lumascript::parse_module(call_text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let needs = ["libmod".to_string()];
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &libraries, ModFacts { needs: &needs, ..ModFacts::of_game(&game) });
        assert!(checked.diags.is_empty(), "{:?}", checked.diags);
        let out = emit::emit_with("caller", "1.0.0", &sources, &[], &libraries, &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
    })
    .unwrap();
}

/// Out buffers (order 2, C1b): the length first with a null buffer of size 0, then the fill. Three forms of the
/// SDK (tests/data/svc/c2: Nebula-IntS 1cc348af3's headers): an `int64_t` length result (`mods.option` text with its
/// NUL, `save.get` bytes, `net.local.address`), a status with the length in `uint64_t* length` (`mods.kit_read`
/// bytes, `mods.kit_names` names each followed by a 0 byte: `list<text>`). Each has answers, so each is `T?`.
#[test]
fn out_buffers_ask_the_length_then_fill() {
    let dir = format!("{}/tests/data/svc/c2", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_mods.h", "luma_save.h", "luma_net.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    use lumascript::svc::{Gives, STy, Shape};
    let gives = |s: &str, f: &str| match &services.faces[s].function(f).unwrap().shape {
        Shape::Callable { gives, .. } => gives.clone(),
        other => panic!("{s}.{f}: {other:?}"),
    };
    assert_eq!(gives("mods", "kit_names"), Gives::Answered(vec![STy::List(Box::new(STy::Text))]));
    assert_eq!(gives("mods", "kit_read"), Gives::Answered(vec![STy::Bytes]));
    assert_eq!(gives("mods", "option"), Gives::Answered(vec![STy::Text]));
    let text = "use svc.mods\nuse svc.save\nuse svc.net.local\n\nvar seen: int = 0\n\non start {\n    if let v = mods.option(\"x\") {\n        seen += v.len()\n    }\n    if let names = mods.kit_names(caller(), \"shop\") {\n        for n in names {\n            seen += n.len()\n        }\n    }\n    if let b = mods.kit_read(caller(), \"shop\", \"a\") {\n        seen += b.len()\n    }\n    if let s = save.get(\"k\") {\n        seen += s.len()\n    }\n    let at = local.address() ?? \"127.0.0.1\"\n    log \"{at} {seen}\"\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in [
            "->option(luma_rt_self, ",
            ", 0, 0);",
            ", 0, 0, &",
            "== LUMA_E_TOO_LARGE)",
            "lumart_list_own(&",
            " + 1u;",
            " + 0u;",
            "lumart_text_make(&",
            "lumart_list_push(&",
            "->address(luma_rt_self, 0, 0)",
        ] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        // Two calls each: the length, then the fill.
        for f in ["option", "kit_names", "kit_read", "get", "address"] {
            assert_eq!(main.matches(&format!("->{f}(luma_rt_self")).count(), 2, "{f}:\n{main}");
        }
        let mut files = out.files.clone();
        for f in ["luma_mods.h", "luma_save.h", "luma_net.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// Arrays with a count (order 3, C1c): `const T* x, uint32_t count` is one `list<T>` argument (`ui.menu_rows`
/// of structs, `draw.post.params` of floats), its elements passed as C has them, null for none; `T* out, uint32_t
/// capacity, uint32_t* count` is a `list<T>` out (`mods.needing`), asked with a capacity of 0 first.
#[test]
fn arrays_with_a_count_are_lists() {
    let dir = format!("{}/tests/data/svc/c2", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_mods.h", "luma_ui.h", "luma_draw_post.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    use lumascript::svc::{Gives, SParam, STy, Shape};
    let shape = |s: &str, f: &str| services.faces[s].function(f).unwrap().shape.clone();
    assert!(matches!(shape("mods", "needing"), Shape::Callable { gives: Gives::Outs(o), .. } if o == vec![STy::List(Box::new(STy::Number("uint64_t".into())))]));
    assert!(matches!(shape("ui", "menu_rows"), Shape::Callable { params, .. } if params.iter().any(|p| matches!(p, SParam::Value { name, ty: STy::List(_) } if name == "rows"))));
    let text = "use svc.mods\nuse svc.ui\nuse svc.draw.post\n\nvar seen: int = 0\n\non start {\n    for m in mods.needing() {\n        seen += 1\n    }\n    if let s = ui.open(\"Hud\", ui.Layer.Pause) {\n        if let p = ui.pane(s, \"Row\") {\n            ui.menu_rows(s, [ui.MenuRow { pane: p, enabled: true }])\n        }\n        ui.menu_rows(s, [])\n    }\n    post.params(post.Stage.Frame, [1.0, 0.5])\n    log \"{seen}\"\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in [
            "->needing(luma_rt_self, 0, 0, &",
            "->needing(luma_rt_self, (uint64_t*)lumart_list_at(",
            "(const LumaUiMenuRow*)lumart_list_at(",
            "(const float*)lumart_list_at(",
            "(uint32_t)lumart_list_n(",
        ] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        for f in ["luma_mods.h", "luma_ui.h", "luma_draw_post.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// Order 4 (C1d, C1g): a handle typedef named as its service is that service's last word (`typedef LumaHandle
/// LumaMovie;` -> `movie.Movie`), another service's handle is that service's (`content.fx.start_on_model` takes a
/// `content.model.Model`), and bytes go in as their block's bytes and length (`save.set`).
#[test]
fn handles_named_as_their_service_and_bytes_in() {
    let dir = format!("{}/tests/data/svc/c2", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_save.h", "luma_movie.h", "luma_content.h", "luma_content_model.h", "luma_content_fx.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    use lumascript::svc::{SParam, STy, Shape};
    assert_eq!(services.faces["movie"].handles, vec!["Movie".to_string()]);
    assert_eq!(services.faces["content.model"].handles, vec!["Model".to_string()]);
    let Shape::Callable { params, .. } = &services.faces["content.fx"].function("start_on_model").unwrap().shape else { panic!("start_on_model is not callable") };
    assert_eq!(params[0], SParam::Value { name: "model".into(), ty: STy::HandleOf("content.model".into(), "Model".into()) });
    let text = "use svc.movie\nuse svc.save\n\nvar seen: int = 0\n\nfn halt(m: movie.Movie) {\n    let (at, all) = movie.position(m)\n    seen = at + all\n    movie.stop(m)\n}\n\non start {\n    let b = \"abc\".bytes()\n    save.set(\"k\", b)\n    save.set(\"e\", \"\".bytes())\n    log \"{seen}\"\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in ["->stop(luma_rt_self, ", "->position(luma_rt_self, ", "(const void*)lumart_bytes_p(", "lumart_list_n("] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        for f in ["luma_save.h", "luma_movie.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// Content values (order 7): `model effect sound layout font movie` are types whose value is the name, so a
/// literal is its text, `name()` gives it, two compare as their names, `<kind>_named(t)` gives `none` for an empty
/// name, and a name goes to a service's `Luma*Name` parameter as its C text (`content.fx.start_on_model`).
#[test]
fn content_values_are_their_names() {
    let dir = format!("{}/tests/data/svc/c2", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_content.h", "luma_content_model.h", "luma_content_fx.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let text = "use svc.content.fx as fx\nuse svc.content.model as cm\n\nvar sounds: list<sound> = [sound \"jump\", sound \"land\"]\nvar last: effect? = none\nvar seen: text = \"\"\n\nfn shine(m: cm.Model, e: effect) {\n    let f: model = model \"mario/gold\"\n    if let g = model_named(\"mario/{seen}gold\") {\n        if g == f and g != model \"mario/red\" {\n            seen = g.name()\n        }\n    }\n    last = effect_named(\"\")\n    fx.start_on_model(m, e, none, none, true)\n    seen = \"{seen}{sounds[1].name()}{f.name()}\"\n}\n\non start {\n    log \"{sounds.len()} {seen}\"\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in ["\"mario/gold\"", "\"mario/red\"", "\"jump\"", "->len > 0", "lumart_text_eq(", "->start_on_model(luma_rt_self, "] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        for f in ["luma_content.h", "luma_content_model.h", "luma_content_fx.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// Requests (order 5, C1a): with a closure (`time.timer`, `clipboard.get_text`) the call gives the handle and
/// a trampoline fills the event's record, its take (take_text) called inside the event's call; waited (`fs.read`,
/// `fs.list`) a one-step task parks until the event and gives the data. Headers: Nebula-IntS's of 2026-11-12.
#[test]
fn requests_with_a_closure_or_waited() {
    let dir = format!("{}/tests/data/svc/c2", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_fs.h", "luma_time.h", "luma_system.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let text = "use svc.fs\nuse svc.time\nuse svc.system.clipboard\n\nvar count: u64 = 0\nvar size: int = 0\nvar timer: time.Timer? = none\nvar pasted: text = \"\"\n\ntask load() {\n    if let b = wait fs.read(\"a.txt\") {\n        size = b.len()\n    }\n    if let names = wait fs.list(\"\", 0) {\n        pasted = names\n    }\n    let n = wait fs.read(\"b.txt\")\n    log \"{n == none}\"\n}\n\non start {\n    timer = time.timer(time.Clock.Frames, 10, 0, fn(e: time.TimerEvent) {\n        count = count + e.value\n    })\n    let k = clipboard.get_text(fn(e: clipboard.TextEvent) {\n        if e.status == none {\n            pasted = e.data\n        }\n        log \"{e.flags}\"\n    })\n    log \"{k == none}\"\n    start load()\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in [
            "->timer(luma_rt_self, ",
            "->take_text(luma_rt_self, ev->handle, (char*)lumart_bytes_p(b), ev->value + 1u)",
            "->take_read(luma_rt_self, ev->handle, (void*)lumart_bytes_p(b), ev->value + 0u)",
            "->take_list(luma_rt_self, ev->handle, (char*)lumart_bytes_p(b), ev->value + 1u)",
            "lumart_task_new(sizeof(lm_rqf_",
            "&lumart_pending_own}",
            "== LUMA_PENDING || ",
            "lumart_fn_keep(",
        ] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        for f in ["luma_fs.h", "luma_time.h", "luma_system.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// A service's structs, colours and callbacks: `draw` read from its
/// header (tests/data/svc/r4: Nebula-K h/roll4's, with its luma_services.h). A struct of plain fields is the header's own (`size` set
/// wherever one is made); `const T*` passes its address, `T?` null for none; a colour goes as 0xRRGGBBAA; `draw.at`'s
/// `LumaDrawFn fn, void* user` takes a `fn(draw.Screen)` value, kept until the mod ends, called through a trampoline.
#[test]
fn a_service_takes_structs_colours_and_callbacks() {
    let dir = format!("{}/tests/data/svc/r4", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_draw.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let draw = &services.faces["draw"];
    assert_eq!(draw.struct_c["Rect"], "LumaDrawRect");
    assert!(draw.sized.iter().any(|s| s == "LumaDrawScreen"));
    assert!(draw.callbacks.contains_key("LumaDrawFn"));
    let text = "use svc.draw\n\nvar seen: f32 = 0.0\nvar pic: draw.Texture? = none\n\nfn paint(s: draw.Screen) {\n    seen = s.w\n    draw.rect(draw.Rect { x: s.safe_x, y: 10.0, w: s.w * 0.5, h: 20.0 }, #FF8000)\n    if let p = pic {\n        draw.image(p, draw.Rect { x: 0.0, y: 0.0, w: 64.0, h: 64.0 }, none, #FFFFFF80)\n    }\n}\n\non start {\n    draw.at(draw.Point.Loading, paint)\n    let k = 2.0\n    draw.at(draw.Point.HudAfter, fn(s: draw.Screen) {\n        draw.rect(draw.Rect { x: 0.0, y: 0.0, w: s.w / k, h: 4.0 }, #000000)\n    })\n    let s = draw.screen()\n    seen = s.h\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        for want in [
            "((LumaDrawFn)lm_cb_0_",
            "lumart_fn_keep(",
            "void lm_cb_0_0(void* user, const LumaDrawScreen* a0) {",
            ".size = (uint32_t)sizeof(LumaDrawScreen);",
            "lumart_color_rgba8(",
            ".some ? &",
            "->rect(luma_rt_self, &",
        ] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        files.push(("lum/luma_draw.h".into(), headers["luma_draw.h"].clone()));
        compiles(&files);
    })
    .unwrap();
}

/// Director 2026-11-12: a hook's `const char *` parameter is `text?`, read at the hook's start; a text written to it
/// goes to the call as its kept copy (a wrap's before `proceed()`, a before's on its way out), only when changed. A
/// parameter that does not cross is bound as `_` (by a name: S0060); a `const char16_t *` result takes a text, its
/// UTF-16 kept copy. `use` paths take reserved words, and a reserved word before `.` names that module.
#[test]
fn hooks_read_and_write_texts_and_pass_what_does_not_cross() {
    let text = "var words: text = \"Hello\"\n\nhook after MR::lookup(id, _) -> w {\n    if id == \"Greeting\" {\n        result = words\n    }\n}\n\nwrap MR::play(a, name) {\n    name = \"Run\"\n    proceed()\n}\n\nhook before MR::play(a, name) {\n    if let n = name {\n        if n == \"Wait\" {\n            name = \"Walk\"\n        }\n    }\n}\n";
    let c = build(text).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(c.contains("LUMA_RESULT(call, const uint16_t*) = (const uint16_t*)"), "{c}");
    assert!(c.contains("lumart_text_keep16("), "{c}");
    assert!(c.contains("= *(const char**)call->args[1];") && c.contains("lumart_text_from_game(&"), "{c}");
    assert_eq!(c.matches("*(const char**)call->args[2] = lm_k;").count(), 2, "{c}");
    let proceed = c.find("*lm_proceeded = 1;").unwrap();
    assert!(c[..proceed].rfind("*(const char**)call->args[2] = lm_k;").is_some(), "{c}");
    // Named, the parameter that does not cross is S0060.
    let (module, d) = lumascript::parse_module("hook after MR::lookup(id, like) -> w {\n}\n");
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let game = game();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().any(|(_, d)| d.code == code::GAME_PARAM_NOT_CROSSING), "{:?}", checked.diags);
    })
    .unwrap();
    // `use kit.player.state` and `state.f()`.
    let (module, d) = lumascript::parse_module("use kit.player.state\n\non start {\n    state.enter(1)\n}\n");
    assert!(d.is_empty(), "{d:?}");
    let _ = module;
}

/// A service's enum compares with `==`, is matched by its cases (`draw.Point.Loading`, every case covering it) and
/// converts with `as` (J, FB 2026-11-12). A type on an `on` event's second name is a
/// parse error.
#[test]
fn a_service_enum_compares_matches_and_converts() {
    let dir = format!("{}/tests/data/svc/r4", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_draw.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let text = std::fs::read_to_string(format!("{}/tests/data/svc_enum.lum", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let (module, d) = lumascript::parse_module(&text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        assert!(main.contains("== (int32_t)5"), "{main}");
        let mut files = out.files.clone();
        files.push(("lum/luma_draw.h".into(), headers["luma_draw.h"].clone()));
        compiles(&files);
        // Without `else`, a match missing a case names it.
        let (module, d) = lumascript::parse_module("use svc.draw\n\nfn f(p: draw.Point) -> i32 {\n    match p {\n        draw.Point.Loading => return 1\n    }\n    return 0\n}\n");
        assert!(d.is_empty(), "{d:?}");
        let sources = [Source { path: "main".into(), module }];
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().any(|(_, d)| d.code == code::MATCH_MISSING), "{:?}", checked.diags);
    })
    .unwrap();
    let (_, d) = lumascript::parse_module("on mods.stopped(a, b: u64) {\n}\n");
    assert!(d.iter().any(|d| d.code == code::UNEXPECTED), "{d:?}");
}

/// Order 6 (C1f): streams. net.local's udp_open and tcp_listen take `fn(local.Event)`, matched by kind with
/// their fields (a datagram's `from` a `local.Peer`, an accepted socket from the event's value); net.internet's
/// tcp_connect takes its DATA through `take`, whose comment it has by "As net.local's send, take and close"; a watch
/// (input.watch_device, mods.watch_mods) gives no handle; audio.play's closure and `how` may be none; input's TEXT
/// carries its text in the event (no take), so text_request is waited as `text?`; fs.exchange has fs's events.
#[test]
fn streams_take_service_events() {
    let dir = format!("{}/tests/data/svc/c6", env!("CARGO_MANIFEST_DIR"));
    let names = ["luma_services.h", "luma_net.h", "luma_audio.h", "luma_input.h", "luma_mods.h", "luma_fs.h"];
    let mut headers = std::collections::BTreeMap::new();
    for f in names {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let text = r#"use svc.net.local
use svc.net.internet
use svc.audio
use svc.input
use svc.mods
use svc.fs.exchange

var size: int = 0
var host: text = ""
var port: u16 = 0
var last: text = ""
var typed: text = ""

fn on_local(e: local.Event) {
    match e {
        local.Event.Accepted(socket) => log "accepted"
        local.Event.Data(socket, data) => { size = data.len() }
        local.Event.Closed(socket, status, value, data) => log "{value} {status == none}"
        local.Event.Datagram(socket, data, from) => {
            host = from.host
            port = from.port
        }
        else => return
    }
}

task ask() {
    if let t = wait input.text_request(input.TextRequest { kind: 0, max_length: 16 }) {
        typed = t
    }
    if let b = wait exchange.read("x.bin") {
        size = b.len()
    }
}

on start {
    let u = local.udp_open(7000, 1, fn(e: local.Event) {
        on_local(e)
    })
    let l = local.tcp_listen(7001, 1, fn(e: local.Event) {
        on_local(e)
    })
    let c = internet.tcp_connect("example.com", 80, fn(e: internet.Event) {
        match e {
            internet.Event.Data(socket, data) => { size = data.len() }
            internet.Event.Connected(socket, data) => log "connected"
            else => return
        }
    })
    let v = audio.play("sfx/ding", none, none)
    input.watch_device(fn(e: input.Event) {
        match e {
            input.Event.Device(value, flags) => log "{value} {flags}"
            else => return
        }
    })
    mods.watch_mods(fn(e: mods.Event) {
        match e {
            mods.Event.Stopped(data) => { last = data }
            else => return
        }
    })
    mods.watch_options(none)
    start ask()
    log "{u == none} {l == none} {c == none} {v == none}"
}
"#;
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in [
            "->udp_open(luma_rt_self, ",
            "->take_datagram(luma_rt_self, ev->handle, (void*)lumart_bytes_p(b), 65507u, &from)",
            "r->socket = (uint64_t)ev->value;",
            "->take(luma_rt_self, ev->handle, 0, 0)",
            "(LumaEventFn)0",
            "->watch_options(luma_rt_self, (",
            "lumart_text_make(&r->data, (const char*)ev->data, ev->data_size, file, line)",
            "->text_request(luma_rt_self, ",
            "&lumart_pending_own}",
        ] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        for f in &names[1..] {
            files.push((format!("lum/{f}"), headers[*f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// Fixed arrays and another service's struct (C order 9b): `const float scale[3]` takes a vec3, `const LumaDrawVec2
/// points[3]` and `const float m[6]` a tuple, each passed as an array made in the call; input.inject's `const
/// LumaInputGame* state` takes input's own `input.Game`. Headers: Nebula-IntS's of 2026-11-12 (data/svc/c3).
#[test]
fn fixed_arrays_and_a_parents_struct() {
    use lumascript::svc::{SParam, STy, Shape};
    let dir = format!("{}/tests/data/svc/c3", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_content.h", "luma_draw.h", "luma_draw_inst.h", "luma_input.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let param = |svc: &str, f: &str, k: usize| match &services.faces[svc].function(f).unwrap().shape {
        Shape::Callable { params, .. } => params[k].clone(),
        other => panic!("{svc}.{f}: {other:?}"),
    };
    assert_eq!(param("draw.inst", "set_scale", 1), SParam::Value { name: "scale".into(), ty: STy::Vec(3) });
    assert_eq!(param("draw", "transform_push", 0), SParam::Value { name: "m".into(), ty: STy::Fixed(Box::new(STy::Number("float".into())), 6) });
    assert_eq!(param("draw", "tri", 0), SParam::Value { name: "points".into(), ty: STy::Fixed(Box::new(STy::Struct("Vec2".into())), 3) });
    assert_eq!(param("draw", "tri", 1), SParam::Value { name: "colours".into(), ty: STy::Fixed(Box::new(STy::Color), 3) });
    assert_eq!(param("input.inject", "set", 1), SParam::Value { name: "state".into(), ty: STy::StructOf("input".into(), "Game".into()) });
    let text = "use svc.draw as draw\nuse svc.draw.inst as di\nuse svc.input as input\nuse svc.input.inject as inject\n\nfn paint() {\n    let a = draw.Vec2 { x: 0.0, y: 0.0 }\n    draw.tri((a, draw.Vec2 { x: 8.0, y: 0.0 }, draw.Vec2 { x: 0.0, y: 8.0 }), (#FF0000, #00FF00, #0000FFFF))\n    draw.transform_push((1.0, 0.0, 4.0, 0.0, 1.0, 2.0))\n    draw.transform_pop()\n}\n\nfn grow(i: di.Inst, s: f32) {\n    di.set_scale(i, vec3(s, s * 2.0, 1.0))\n}\n\nfn feed(h: inject.Request, b: u32) {\n    var g = input.Game { buttons: b, stick_x: 0.5, stick_y: 0.0, pointer_x: 0.0, pointer_y: 0.0, pointer_on: 0, shake: 1, device: 0, injected: 0 }\n    g.pointer_on = 1\n    g.stick_x += 0.25\n    inject.set(h, g)\n}\n\non start {\n    log \"ready\"\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        for want in ["(const float[3]){", "(const float[6]){", "(const LumaDrawVec2[3]){", "(const LumaColor[3]){lumart_color_rgba8(", "->set(luma_rt_self, ", ".pointer_on = "] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        for f in ["luma_content.h", "luma_draw.h", "luma_draw_inst.h", "luma_input.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// the script's copy of a record to a `const T &` passes the copy's bytes for the call
/// (lumart_record_at, not made its own: the game reads them only); the C compiles.
#[test]
fn a_copy_goes_to_a_const_reference() {
    let c = build("on start {\n    var b = MR::makeBox(3)\n    MR::readBox(b)\n    MR::readBox(MR::makeBox(4))\n}\n").unwrap();
    assert_eq!(c.matches("lumart_record_at(&").count(), 2, "{c}");
    assert!(c.contains(", 0, \"main.lum\", 3u)"), "not made its own, at its line:\n{c}");
}

/// A service struct holding text and an actor (draw.view's Camera: vectors, an enum, `LumaActor actor`,
/// `char joint[32]`) is the script's own copy: made by a literal, its actor kept as a handle (read back as the view,
/// S0903 once gone), its fields read, `T?` of it, and written as the header's struct for a call (`rig`, a const
/// pointer), its text checked to fit the array (S0908). The C compiles against the SDK's headers of 2026-11-12.
#[test]
fn a_service_struct_with_text_and_an_actor() {
    let dir = format!("{}/tests/data/svc/c4", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_content.h", "luma_draw.h", "luma_draw_view.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let text = "use svc.draw.view as view\n\nfn aim(r: view.Region, c: view.Camera, j: text) -> view.Camera {\n    let d = view.Camera { rig: view.Rig.Follow, eye: c.eye, at: c.at, up: vec3(0.0, 1.0, 0.0), fov_y: 45.0, actor: c.actor,\n        offset: vec3(0.0, 100.0, -300.0), joint: j, near_z: 0.0 }\n    view.rig(r, d)\n    return d\n}\n\nfn joint_of(c: view.Camera?) -> text {\n    if let k = c {\n        return \"{k.joint} {k.fov_y} {k.offset.y}\"\n    }\n    return \"\"\n}\n\non start {\n    log \"ready\"\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        if let Some(to) = std::env::var_os("LUMA_EMIT_DUMP") {
            std::fs::write(to, &main).unwrap();
        }
        let all: String = out.files.iter().filter(|(p, _)| p.ends_with(".c") || p.ends_with(".h")).map(|(_, c)| c.as_str()).collect();
        for want in ["lumart_ref_make(", "lumart_ref_get(", "903, 1", "908, 2", "->rig(luma_rt_self, ", "lm_sw_", "lm_sc_"] {
            assert!(all.contains(want), "{want} in\n{all}");
        }
        let mut files = out.files.clone();
        for f in ["luma_content.h", "luma_draw.h", "luma_draw_view.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// `as?` and `fail` (C9): the narrowing asks the kernel's as_class (null stays none), `fail` raises S0921 with the
/// mod's words and leaves the function; the C compiles.
#[test]
fn narrowing_and_fail_write_c() {
    let c = build("fn which(a: Actor?) -> int {\n    if let k = a as? Kuribo {\n        return 1\n    }\n    return 0\n}\n\nfn checked(n: int) -> int {\n    if n < 0 {\n        fail \"below zero: {n}\"\n    }\n    return n\n}\n\non start {\n    log \"{checked(which(MR::actor()))}\"\n}\n").unwrap();
    for want in ["->as_class(luma_rt_self, ", "921, 1, lm_words", "goto lm_out;"] {
        assert!(c.contains(want), "{want} in\n{c}");
    }
}

/// A marked pointer takes the address of a pointer (docs/modding/language/compiled-form.md, section 5): `var g` to an `out` the place's own address, a value
/// to an `in` a copy's (`MtxPtr` too), `none` null. A `const Vec3f &` result is read through the address the call writes. A view
/// taken out of a list or a map into `T?` is the pointer itself. The C compiles.
#[test]
fn marked_pointers_place_results_and_views_out_of_containers() {
    let c = build(r#"on start {
    let u = MR::up()
    var g = vec3(0.0, 0.0, 0.0)
    if let me = MR::actor() {
        if MR::gravityAt(me, var g) {
            log "{g.y} {u.x}"
        }
        MR::gravityAt(me, none)
    }
    MR::scaleBy(2.0)
    MR::scaleBy(none)
    MR::setMtx(mat34(vec4(1.0, 0.0, 0.0, 0.0), vec4(0.0, 1.0, 0.0, 0.0), vec4(0.0, 0.0, 1.0, 0.0)))
    let xs: list<Actor> = []
    if let a = xs.first() {
        a.kill()
    }
    var m: map<int, Actor> = {}
    if let b = m.get(1) {
        b.kill()
    }
}
"#)
    .unwrap();
    // The matrix to `MtxPtr` (a typedef of a pointer, marked in): a pointer holds its address, args[n] that pointer's.
    let mats: Vec<&str> = c.lines().filter_map(|l| l.trim().strip_prefix("lm_mat34 ")).filter_map(|l| l.split(' ').next()).collect();
    assert!(mats.iter().any(|m| c.contains(&format!(" = (void*)&{m};"))), "{c}");
    assert!(!mats.iter().any(|m| c.contains(&format!("] = (void*)&{m};"))), "a matrix is no pointer:\n{c}");
    for want in ["(void*)&l", "(*(const lm_vec3*)"] {
        assert!(c.contains(want), "{want} in\n{c}");
    }
    assert!(!c.contains("(void*){1, "), "a view in an option is the pointer itself:\n{c}");
}

/// A mod class's fields first values folded from constants (a negative number, `none`, a vector of them), `ref(self)`
/// in its override, `get()` of a `ref<Class>` (the class's own number to as_class) and `?.get()` of a `ref<Class>?`.
/// The C compiles.
#[test]
fn class_first_values_and_refs_of_self() {
    let c = build(r#"class W : Actor {
    var speed: f32 = -1.5
    var at = vec3(0.0, -2.0, (1.0))
    var mark: int? = none
    var me: ref<W>? = none
    override fn control() {
        me = ref(self)
        if let w = me?.get() {
            w.speed += 1.0
        }
        if let r = me {
            if let v = r.get() {
                log "{v.at.y} {v.mark == none}"
            }
        }
    }
}
"#)
    .unwrap();
    for want in ["lumart_ref_make(", "lumart_ref_get(", "-1.5"] {
        assert!(c.contains(want), "{want} in\n{c}");
    }
}

/// The SDK headers of a test folder (`tests/data/svc/<dir>`) as services, and the mod's C built against them.
fn build_svc(dir: &str, names: &[&str], text: &str) -> String {
    let dir = format!("{}/tests/data/svc/{dir}", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in names {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let all: String = out.files.iter().filter(|(p, _)| p.ends_with(".c") || p.ends_with(".h")).map(|(_, c)| c.as_str()).collect();
        let mut files = out.files.clone();
        for f in names.iter().filter(|f| **f != "luma_services.h") {
            files.push((format!("lum/{f}"), headers[*f].clone()));
        }
        compiles(&files);
        all
    })
    .unwrap()
}

/// `same()` of service handles and enums, options of them too (both none is the same, one none is not); the C
/// compiles.
#[test]
fn same_of_service_handles_and_enums() {
    let all = build_svc(
        "c4",
        &["luma_services.h", "luma_content.h", "luma_draw.h", "luma_draw_view.h"],
        r#"use svc.draw.view as view

fn one(a: view.Region, b: view.Region) -> bool {
    return same(a, b)
}

fn maybe(a: view.Region?, b: view.Region?) -> bool {
    return same(a, b)
}

fn rig(a: view.Rig, b: view.Rig?) -> bool {
    return same(a, b)
}

on start {
    log "ready"
}
"#,
    );
    assert!(all.contains(".some) == ("), "{all}");
}

/// `on step` and `on step(now: time.Now)`: the step body registered with the time service's on_step at start and
/// taken back at stop, the parameter the kernel's LumaTimeNow; with `on frame` beside it both are registered. The C
/// compiles.
#[test]
fn on_step_registers_with_the_time_service() {
    let names = ["luma_services.h", "luma_content.h", "luma_time.h"];
    let all = build_svc("c2", &names, "use svc.time as time\n\non step(now: time.Now) {\n    log \"{now.step_in_frame}\"\n}\n\non frame {\n    log \"f\"\n}\n");
    for want in ["lm_on_step_", "->on_step(luma_rt_self, ", "(*lm_now)", "->on_frame("] {
        assert!(all.contains(want), "{want} in\n{all}");
    }
    let bare = build_svc("c2", &names, "on step {\n    log \"s\"\n}\n");
    assert!(bare.contains("->on_step(luma_rt_self, "), "{bare}");
}

/// The docs/modding/language/classes.md's alias, placement and gravity as std functions (alias_class, placement_of, placed_at,
/// make_gravity<C>), a gravity class on PlanetGravity whose virtual has no function row (only the record's
/// `virtuals`), and a hook's and an override's `T *` to a vector or number read as `T?` through the pointer and
/// written back through it. The C compiles.
#[test]
fn aliases_placements_gravities_and_pointer_parameters() {
    let src = "class Pull : PlanetGravity {
    var power: f32 = 2.0
    override fn calcOwnGravityVector(dest, scalar, at) -> bool {
        dest = vec3(0.0, -1.0, 0.0)
        scalar = power
        return true
    }
}

hook after MR::pullAt(actor, gravity, _) -> found {
    if let g = gravity {
        gravity = vec3(g.x, g.y * 2.0, g.z)
    }
}

on start {
    let stands = alias_class(\"Thing\", \"Pull\", [3])
    for a in make(\"Kuribo\", MakePlace { at: vec3(0.0, 0.0, 0.0) }) {
        if let p = placement_of(a) {
            log \"{stands} {p.zone} {p.layer} {p.index}\"
            if let b = placed_at(Placement { stage: \"\", zone: p.zone, layer: p.layer, index: p.index }) {
                log \"{same(a, b)}\"
            }
        }
    }
    if let g = make_gravity<Pull>(GravityPlace { range: 500.0, priority: 2 }) {
        g.power = 3.0
    }
}
";
    let c = build(src).unwrap();
    for want in [
        "lumart_alias_class(",
        "lumart_placement_of(",
        ".value.stage, &",
        "lumart_placed_at(",
        "lumart_make_gravity(\"Pull\", ",
        "{\"PlanetGravity::calcOwnGravityVector\", \"bool(Vec3f *, f32 *, const Vec3f &) const\", ",
        "lm_pp1 = *(void**)call->args[1];",
        "*(lm_vec3*)lm_pp1 = ",
        "*(float*)lm_pp2 = ",
        "lm_pp2 = *(void**)call->args[2];",
    ] {
        assert!(c.contains(want), "no {want:?} in:\n{c}");
    }
    let h = build_file(src, "lum/lm.h").unwrap();
    assert!(c.contains("\"Pull\", \"PlanetGravity\", 0, 0") || h.contains("\"Pull\", \"PlanetGravity\", 0, 0"), "{c}");
}

/// The docs/modding/language/numbers.md's set_number: a file's constant by its name, and a function's place-th
/// number by its signature and place, set for the calling mod; a whole number and a bool's 1 go as the double the
/// kernel takes. The C compiles.
#[test]
fn set_number_sets_a_constant_and_a_functions_number() {
    let src = "on start {
    let health = set_number(\"Game/System/GameRules.cpp#cPlayerHealth\", 1)
    let warn = set_number(\"Game/Player/MarioEffect.cpp#cLowHealthWarning\", 0.0)
    let jump = set_number(\"Mario::doJump\", \"void(f32)\", 2, 22.5)
    log \"{health} {warn} {jump}\"
}
";
    let c = build(src).unwrap();
    for want in [
        "lumart_set_number(",
        ", 0, 0u, (double)",
        ", (uint32_t)",
    ] {
        assert!(c.contains(want), "no {want:?} in:
{c}");
    }
}

/// D 21:51 items 1 and 2: a pub struct's function crosses as `<module>.<S>.<fn>` (its `self` first, by value), and a
/// field default the caller cannot write (a function value, a list with elements) is the library's own export
/// `<module>.<S>.<field>.default`, called where the caller leaves the field out.
#[test]
fn a_library_struct_gives_its_functions_and_made_defaults() {
    let lib_text = "pub fn burst_always(n: i32) -> bool {\n    return n > 0\n}\n\npub struct Shot {\n    power: i32 = 2\n    on_hit: fn(i32) -> bool = burst_always\n    tags: list<text> = [\"a\", \"b\"]\n    fn scaled(self, k: i32) -> i32 { return self.power * k }\n}\n";
    let (module, d) = lumascript::parse_module(lib_text);
    assert!(d.is_empty(), "{d:?}");
    let lib_sources = [Source { path: "kit/one".into(), module }];
    let game = game();
    let (lumi, exports) = lumascript::on_compiler_stack(|| {
        let checked = check::check(&lib_sources, &[], ModFacts::of_game(&game));
        assert!(checked.diags.iter().all(|(_, d)| lumascript::diag::is_note(d.code)), "{:?}", checked.diags);
        let out = emit::emit_with("libmod", "1.0.0", &lib_sources, &[lib_text.to_string()], &[], &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        (out.lumi.unwrap(), out.exports)
    })
    .unwrap();
    let names: Vec<&str> = exports.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["kit.one.Shot.on_hit.default", "kit.one.Shot.scaled", "kit.one.Shot.tags.default", "kit.one.burst_always"] {
        assert!(names.contains(&want), "{want} not in {names:?}");
    }
    assert!(!names.contains(&"kit.one.Shot.power.default"), "a literal default is the caller's: {names:?}");
    let sig = |n: &str| exports.iter().find(|(e, _)| e == n).unwrap().1.clone();

    let modules = lumascript::lumi::read(&lumi).unwrap_or_else(|e| panic!("{e:?}\n{lumi}"));
    let libraries = [check::Library { id: "libmod".into(), modules }];
    let call_text = "use libmod.kit.one\n\non start {\n    let s = one.Shot { power: 3 }\n    let f = s.on_hit\n    log \"{s.scaled(2)} {f(1)} {s.tags.len()}\"\n}\n";
    let (module, d) = lumascript::parse_module(call_text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let needs = ["libmod".to_string()];
    let (files, imports) = lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &libraries, ModFacts { needs: &needs, ..ModFacts::of_game(&game) });
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let out = emit::emit_with("caller", "1.0.0", &sources, &[], &libraries, &checked, &game).unwrap_or_else(|e| panic!("{e:?}"));
        compiles(&out.files);
        (out.files, out.imports)
    })
    .unwrap();
    let rows: Vec<(&str, &str)> = imports.iter().filter(|(_, _, k)| *k == "mod function").map(|(n, s, _)| (n.as_str(), s.as_str())).collect();
    for want in ["kit.one.Shot.scaled", "kit.one.Shot.on_hit.default", "kit.one.Shot.tags.default"] {
        let row = rows.iter().find(|(n, _)| n.strip_prefix("libmod/") == Some(want)).unwrap_or_else(|| panic!("no import of {want}: {rows:?}"));
        assert_eq!(row.1, sig(want), "{want}");
    }
    let main = &files.iter().find(|(p, _)| p == "lum/main.c").unwrap().1;
    // The two made defaults and `scaled`, each through its import; the fields written from them.
    for i in 0..3 {
        assert!(main.contains(&format!("lm_mf_{i})(")), "lm_mf_{i} not called: {main}");
    }
    assert!(main.contains("lm_cp_lumart_fn(&t3.on_hit, ") && main.contains("lm_cp_lumart_list(&t3.tags, "), "{main}");
}

/// D 21:51 item 3: `draw.texture_from_pixels(w, h, rgba8, flags)` takes the script's bytes for `rgba8`, whose length
/// is its w x h x 4 rather than a size after it; a wrong length is a script error before the call.
#[test]
fn pixels_are_bytes_checked_against_w_and_h() {
    let dir = format!("{}/tests/data/svc/r4", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_draw.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let text = "use svc.draw\n\non start {\n    let b = \"abcd\".bytes()\n    if let t = draw.texture_from_pixels(1, 1, b, 0) {\n        log \"made\"\n    }\n}\n";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        for want in ["* 4u && lumart_wrong_call(", "\"bytes not w x h x 4 long\"", "(const void*)lumart_bytes_p(", "->texture_from_pixels(luma_rt_self, "] {
            assert!(main.contains(want), "{want} in\n{main}");
        }
        let mut files = out.files.clone();
        files.push(("lum/luma_draw.h".into(), headers["luma_draw.h"].clone()));
        compiles(&files);
    })
    .unwrap();
}

/// D 21:51 item 4 (section 10): options are globals read at start through mods `option` (a number parsed, a
/// toggle "1", a key option input's action handle) and again on their OPTION event, which runs `on option`; `on load`
/// runs in start after the modules' values and before `on start`; `on draw` is draw's `at` for its point (checked);
/// `on layers` is LumaModInfo's `layers` (D's line), the mod's values made first, once.
#[test]
fn options_load_draw_and_layers_are_the_mods() {
    let root = format!("{}/tests/data/svc", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for (f, from) in [("luma_services.h", "c6"), ("luma_mods.h", "c6"), ("luma_input.h", "c6"), ("luma_draw.h", "r4")] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{root}/{from}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    let mut game = game();
    game.services = services;
    use lumascript::check::{OptionKind, OptionSpec};
    let options = [
        OptionSpec { key: "speed".into(), kind: OptionKind::Number { whole: false } },
        OptionSpec { key: "hard".into(), kind: OptionKind::Toggle },
        OptionSpec { key: "count".into(), kind: OptionKind::Number { whole: true } },
        OptionSpec { key: "colour".into(), kind: OptionKind::Text },
        OptionSpec { key: "jump".into(), kind: OptionKind::Key },
    ];
    let run = |text: &str| {
        let (module, d) = lumascript::parse_module(text);
        assert!(d.is_empty(), "{d:?}");
        let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
        lumascript::on_compiler_stack(|| {
            let checked = check::check(&sources, &[], ModFacts { options: &options, ..ModFacts::of_game(&game) });
            let errors: Vec<u16> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).map(|(_, d)| d.code).collect();
            if !errors.is_empty() {
                return Err(errors);
            }
            Ok(emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}")).files)
        })
        .unwrap()
    };
    let text = "use svc.input\n\noption speed: f32\noption hard: bool\noption count: int\noption colour: text\noption jump: input.Action\n\nvar seen: f32 = 0.0\n\non start {\n    seen = speed\n    log \"{colour} {hard} {count}\"\n}\non option speed {\n    seen = speed\n}\non load {\n    seen = 1.0\n}\non draw draw.Point.HudAfter {\n    seen = seen + 1.0\n}\non layers -> list<text> {\n    if hard {\n        return [\"xmas\"]\n    }\n    return []\n}\n";
    let files = run(text).unwrap_or_else(|e| panic!("the checker refuses: {e:?}"));
    let file = |name: &str| files.iter().find(|(p, _)| p == name).map(|(_, c)| c.clone()).unwrap();
    let (main, modc) = (file("lum/main.c"), file("lum/mod.c"));
    assert!(main.contains("float lm_opt_speed;") && main.contains("lumart_text lm_opt_colour;"), "{main}");
    assert!(main.contains("= lm_opt_speed;"), "an option read by its name: {main}");
    for want in [
        "lumart_text_parse_f64(lm_t, &lm_v)) lm_opt_speed = (float)lm_v;",
        "lm_t->b[0] == '1');",
        "lumart_text_parse_i64(lm_t, &lm_v)) lm_opt_count = (int32_t)lm_v;",
        "lm_cp_lumart_text(&lm_opt_colour, lm_t);",
        "->action(luma_rt_self, \"jump\", (LumaInputAction*)(void*)&lm_opt_jump);",
        "if (event->kind != LUMA_MODS_EVENT_OPTION) return;",
        "    case 0u:\n        lm_opt_read_0();\n        (void)lm_on_option_0_",
        "->watch_options(mod, lm_options, 0) != 0) return 1;",
        "->at(mod, (LumaDrawPoint)4, lm_draw_4, 0) != 0) return 1;",
        "->watch_options(luma_rt_self, 0, 0);",
        "->at(luma_rt_self, (LumaDrawPoint)4, 0, 0);",
        "if (lm_prepare() != 0) return 0;",
        ".layers = lm_layers};",
    ] {
        assert!(modc.contains(want), "{want:?} in\n{modc}");
    }
    let start = modc.find("static int32_t lm_start(").unwrap();
    let (prepare, load, first) = (modc[start..].find("lm_prepare()").unwrap(), modc[start..].find("lm_on_load_0()").unwrap(), modc[start..].find("lm_on_start_0_").unwrap());
    assert!(prepare < load && load < first, "values, then on load, then on start:\n{modc}");
    // With `use optional svc.draw` the slot may be null (the service not given): the point is taken and given back only
    // when it is not.
    let optional = run(&format!("use optional svc.draw\n{text}")).unwrap_or_else(|e| panic!("the checker refuses: {e:?}"));
    let omodc = optional.iter().find(|(p, _)| p == "lum/mod.c").map(|(_, c)| c.clone()).unwrap();
    let slot = omodc.split("->at(mod, (LumaDrawPoint)4").next().and_then(|s| s.rsplit("&& ").next()).unwrap_or("").to_string();
    assert!(slot.starts_with("lm_svc_"), "{omodc}");
    assert!(omodc.contains(&format!("if ({slot} != 0 && {slot}->at(mod, (LumaDrawPoint)4, lm_draw_4, 0) != 0) return 1;")), "{omodc}");
    assert!(omodc.contains(&format!("if ({slot} != 0) (void){slot}->at(luma_rt_self, (LumaDrawPoint)4, 0, 0);")), "{omodc}");
    assert!(!modc.contains("!= 0 && lm_svc_"), "a needed draw is called as before:\n{modc}");
    // luma.h with the `layers` field (D's line) over the SDK's until the SDK has it.
    let mut files = files.clone();
    for f in ["luma_mods.h", "luma_input.h", "luma_draw.h"] {
        files.push((format!("lum/{f}"), headers[f].clone()));
    }
    if let Some(sdk) = std::env::var_os("LUMA_SDK") {
        let luma_h = std::fs::read_to_string(std::path::Path::new(&sdk).join("luma.h")).unwrap();
        if !luma_h.contains("(*layers)(") {
            let gone = "void (*dependent_gone)(const LumaMod* dependent);";
            assert!(luma_h.contains(gone));
            files.push(("lum/luma.h".into(), luma_h.replace(gone, &format!("{gone} int64_t (*layers)(const LumaCore* core, const LumaMod* mod, char* buf, uint64_t size);"))));
        }
    }
    compiles(&files);

    // A point draw does not have, a draw that is no point, and layers of another type.
    assert_eq!(run("on draw draw.Point.Overlay { }\n"), Err(vec![code::NO_MEMBER]));
    assert_eq!(run("on draw seen { }\n\nvar seen: f32 = 0.0\n"), Err(vec![code::TYPE_MISMATCH]));
    assert_eq!(run("on layers -> list<int> {\n    return []\n}\n"), Err(vec![code::TYPE_MISMATCH]));
}

/// `on stage "<name>"` (section 10): run on the stage's first PLAY frame after a load once its opening is over (the
/// scenario starter and opening camera, as the test harness takes "ready"), so a handler that plays a scene or takes
/// the camera does not hold the opening up.
#[test]
fn on_stage_runs_once_the_stage_is_ready() {
    let c = build_file("on stage \"EggStarGalaxy\" {\n    log \"here\"\n}\n", "lum/mod.c").unwrap_or_else(|e| panic!("{e:?}"));
    for want in [
        "now->state == LUMA_TIME_PLAY && lm_stage_loaded",
        "if (lm_starting == 0 && lm_opening == 0) {",
        "lm_stage_is(lm_stage, \"EggStarGalaxy\")",
    ] {
        assert!(c.contains(want), "no {want:?} in:\n{c}");
    }
    // The stage counts as seen only once it is ready: a frame of its opening leaves lm_stage_loaded set.
    let ready = c.find("if (lm_starting == 0 && lm_opening == 0) {").unwrap();
    assert!(c[ready..].contains("lm_stage_loaded = 0;"), "{c}");
    assert!(!c[..ready].contains("lm_stage_loaded = 0;"), "{c}");
}

/// docs/modding/language/objects.md: a member the facts mark virtual, called on an object, runs the object's own class's function (import
/// kind 8, LumaCore.method; args[0] the object as the row's record, through core->as_class when the static type does
/// not say it is at the object's address): on a view, on `self` in a mod's class (an override of the class's own,
/// S0051 before), by its bare name in a hook, through `?.`. A member that is not virtual is the row itself (kind 0),
/// as is `base.f()` (the kernel's proceed).
#[test]
fn virtual_members_called_on_objects_run_the_objects_own() {
    let src = "class Walker : Actor {\n    var steps = 0\n    override fn control() {\n        steps += 1\n        base.control()\n    }\n    override fn receive(n: i32) -> bool {\n        self.control()\n        return base.receive(n)\n    }\n}\n\nhook after Actor::kill() {\n    control()\n}\n\non start {\n    if let k = MR::actor() {\n        k.control()\n        let took = k.receive(3)\n        k.kill()\n        log \"{took}\"\n    }\n    MR::actor()?.control()\n}\n";
    let imports = build_file(src, "lum/imports.c").unwrap_or_else(|e| panic!("{e:?}"));
    for want in ["LUMA_METHOD(lm_vm_", "\"Actor::control\", \"void()\")", "\"Actor::receive\", \"bool(s32)\")"] {
        assert!(imports.contains(want), "no {want:?} in:\n{imports}");
    }
    for line in imports.lines().filter(|l| l.contains("\"Actor::control\"") || l.contains("\"Actor::receive\"")) {
        assert!(line.trim_start().starts_with("LUMA_METHOD("), "a virtual called directly: {line}\n{imports}");
    }
    // kill is no virtual: its row itself.
    assert!(imports.lines().any(|l| l.trim_start().starts_with("LUMA_FUNCTION(") && l.contains("\"Actor::kill\"")), "{imports}");
    let c = build(src).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(c.contains("lm_vm_") && c.contains("->invoke("), "{c}");
    // `?.`'s value has no static record here: the object as Actor by as_class; a view of Actor itself is passed as it is.
    assert!(c.contains("(void*)luma_rt_core->as_class(luma_rt_self, "), "{c}");
    let h = build_file(src, "lum/lm.h").unwrap();
    assert!(h.contains("extern const LumaFunction* lm_vm_0; /* virtual "), "{h}");
}


/// D12.3 on a struct's fields: net's HttpRequest holds `url` (text), `headers` (a list of Header, its count hidden) and
/// `body` (bytes, its size hidden), each left out null; written as pointers into the script's values for the call.
/// an unnamed enum's cases are constants (input.TEXT_NAME, input.BUTTON_A), not the event enum's. D7.4: a
/// service's enum and `svc.Answer` are shown in text by their cases' names. D4.14: `text "Key"(name: v)` reads the
/// line through the runtime (lumart_text_message), the key when there is none, and fills `{name}`. Headers of Nebula
/// d80af5cce.
#[test]
fn pointer_fields_constants_shown_answers_and_message_rows() {
    let dir = format!("{}/tests/data/svc/c7", env!("CARGO_MANIFEST_DIR"));
    let mut headers = std::collections::BTreeMap::new();
    for f in ["luma_services.h", "luma_net.h", "luma_input.h"] {
        headers.insert(f.to_string(), std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
    }
    let (services, errors) = lumascript::svc::Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    use lumascript::svc::{FieldPtr, STy};
    let net = &services.faces["net.internet"];
    let ptrs = &net.field_ptrs["HttpRequest"];
    assert_eq!(ptrs["url"], FieldPtr::Text);
    assert_eq!(ptrs["body"], FieldPtr::Bytes { len: "body_bytes".into() });
    assert_eq!(ptrs["headers"], FieldPtr::List { count: "header_count".into(), elem: "Header".into(), elem_c: "LumaNetHeader".into() });
    let fields: Vec<&str> = net.structs["HttpRequest"].iter().map(|(n, _)| n.as_str()).collect();
    assert!(!fields.contains(&"body_bytes") && !fields.contains(&"header_count") && !fields.contains(&"size"), "{fields:?}");
    assert!(net.structs["HttpRequest"].contains(&("body".to_string(), STy::Bytes)));
    let input = &services.faces["input"];
    assert_eq!(input.constants["TEXT_NAME"], 2);
    assert_eq!(input.constants["BUTTON_A"], 1);
    assert!(!input.constants.keys().any(|k| k.starts_with("EVENT_")), "{:?}", input.constants.keys().collect::<Vec<_>>());
    let text = r#"use svc.net.internet as internet
use svc.input

on start {
    let r = internet.http(internet.HttpRequest {
        url: "https://a.example/x"
        headers: [internet.Header { name: "Content-Type", value: "application/json" }]
        body: b"{}"
        timeout_ms: 1000
        flags: 0
    }, fn(e: internet.Event) {
        match e {
            internet.Event.Error(socket, s, value) => {
                if let a = s { log "failed: {a}" }
            }
            else => return
        }
    })
    log "{input.TEXT_NAME} {r != none}"
    log text "Mod_Row"(row: 7)
}
"#;
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    let mut game = game();
    game.services = services;
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        for want in [
            // The request's pointer fields: text as C text (empty: null), the list as an array of LumaNetHeader in
            // scratch memory with its count, the bytes in place with their size.
            "d->url = lumart_text_cstr(s->url,",
            "lumart_scratch_zeroed(n * sizeof(LumaNetHeader),",
            "d->header_count = (uint32_t)n;",
            "d->body = d->body_bytes != 0 ? (const void*)lumart_bytes_p(s->body) : 0;",
            // An answer by its case's name.
            "\"NotFound\"",
            // The message row and its fill.
            "lumart_text_message(&",
            "lumart_text_fill(&",
        ] {
            assert!(main.contains(want), "{want}\n{main}");
        }
        // The runtime reads the row by names bound at run time, so the mod imports no game function for it.
        assert!(
            !out.files.iter().any(|(p, c)| !p.contains("lumart") && c.contains("\"MR::getGameMessageDirect\"")),
            "the row is read through an import: {:?}",
            out.files.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
        let mut files = out.files.clone();
        for f in ["luma_net.h", "luma_input.h"] {
            files.push((format!("lum/{f}"), headers[f].clone()));
        }
        compiles(&files);
    })
    .unwrap();
}

/// The mod's record carries the runtime layout when the SDK's LumaModInfo has the field, and is written as before when
/// it has not; both compile (against luma.h with the field added, and as it is).
#[test]
fn the_record_carries_the_runtime_layout_when_the_sdk_has_it() {
    let (module, d) = lumascript::parse_module("on start { log \"a\" }\n");
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }, lumascript::check::std_source()];
    for with in [false, true] {
        let mut game = game();
        game.mod_info_layout = with;
        lumascript::on_compiler_stack(|| {
            let checked = check::check(&sources, &[], ModFacts::of_game(&game));
            let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
            let record = out.files.iter().find(|(p, _)| p == "lum/mod.c").map(|(_, c)| c.clone()).unwrap();
            let want = format!(".runtime_layout = {}", lumascript::lumi::RUNTIME_LAYOUT);
            assert_eq!(record.contains(&want), with, "{record}");
            let mut files = out.files.clone();
            if with {
                if let Some(sdk) = std::env::var_os("LUMA_SDK") {
                    let real = std::fs::read_to_string(std::path::Path::new(&sdk).join("luma.h")).unwrap();
                    assert!(real.contains("} LumaModInfo;"));
                    // The SDK's own LumaModInfo, given the field only when it has none yet.
                    let info = &real[real.find("typedef struct LumaModInfo").unwrap_or(0)..real.find("} LumaModInfo;").unwrap()];
                    if !info.contains(" runtime_layout;") {
                        files.push(("lum/luma.h".into(), real.replacen("} LumaModInfo;", "    uint32_t runtime_layout;\n} LumaModInfo;", 1)));
                    }
                }
            }
            compiles(&files);
        })
        .unwrap();
    }
}

/// lumart's text from the game, run (when LUMA_SDK names luma.h's folder): a game's UTF-16 message with its tags in
/// bracket words as the import's decode_text writes them ([wait n], a tag of no word [tag g i], [icon n], [player]),
/// "[" as "[[", a surrogate pair and a lone surrogate (U+FFFD); a fill replaces every `{name}` and leaves a brace
/// that is no placeholder. `text "Key"` made before the game's messages exist (as in `on start`) is a row: read then,
/// it is its key and the mod log says so once; read after they exist, it is the game's message; with no message it is
/// its key. Run again with a message function that cannot bind: every row is its key, the log says which name once,
/// and nothing is called through null.
#[test]
fn game_text_and_message_rows_run() {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("lumart is not run: LUMA_SDK is not set");
        return;
    };
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let rt = format!("{}/rt", env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("lumart-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.c"), LUMART_RUN).unwrap();
    let run = |name: &str, define: Option<&str>| -> Vec<String> {
        let exe = dir.join(format!("{name}.exe"));
        let mut c = std::process::Command::new(&clang);
        c.arg("--target=x86_64-pc-windows-msvc").arg("-I").arg(&rt).arg("-I").arg(&sdk);
        if let Some(d) = define {
            c.arg(format!("-D{d}"));
        }
        let out = c.arg(format!("{rt}/lumart.c")).arg(dir.join("t.c")).arg("-o").arg(&exe).output()
            .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let run = std::process::Command::new(&exe).output().unwrap();
        assert!(run.status.success(), "exit {:?}", run.status);
        String::from_utf8(run.stdout).unwrap().lines().map(str::to_string).collect()
    };
    let game = "Hi [[[wait 30][tag 5 0][icon 7]\u{1F600}\u{FFFD}[player]!";
    let back = "Hi [[[wait 30][tag 5 0][icon 7]![player][color 3]a[color 0][page][center][small][normal][large][sound SE_SV_TICO][tag 6 5 0 1 2][wait 16][race_time][best_time 3][font AB][middle][number 5 2][string 1 0][player_name 1]";
    assert_eq!(
        run("t", None),
        vec![game, "log text \"Mod_Hello\" (main.lum:2) was read before the game's messages exist and reads as its key there",
            "Mod_Hello", "Mod_Hello", "1. Ana (you) 1. Ana{x", "Hello [player]", "Mod_Hello", "Hello [player]", "Hello [player]", "row keep16",
            "keep16 tags", back, "keep16 plain", "held 0"]
    );
    assert_eq!(
        run("unbound", Some("LUMART_MESSAGE_FUNCTION=\"MR::noSuchMessage\"")),
        vec![game, "log text \"Key\" reads as its key: the game has no function MR::noSuchMessage", "Mod_Hello", "Mod_Hello",
            "1. Ana (you) 1. Ana{x", "Mod_Hi", "Mod_Hello", "Mod_Hi", "Mod_Hi", "keep16 tags", back, "keep16 plain", "held 0"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const LUMART_RUN: &str = r#"#include "lumart.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static int live; /* blocks the core gave and was not given back */
static void* a(const LumaMod* m, uint64_t s, uint64_t al) { (void)m; live++; return _aligned_malloc((size_t)s, (size_t)al); }
static void f(const LumaMod* m, void* p) { (void)m; if (p != 0) live--; _aligned_free(p); }
static void rep(const LumaMod* m, const char* file, uint32_t line, uint32_t code, const char* const* args, uint32_t n) {
    (void)m; (void)args; (void)n; printf("report %s:%u %u\n", file, line, code);
}
static void lg(const LumaMod* m, const char* t) { (void)m; printf("log %s\n", t); }
static int32_t gt(void) { return 1; }
static LumaCore core;
const LumaCore* luma_rt_core = &core;
const LumaMod* luma_rt_self = 0;
static void show(lumart_text t) { printf("%.*s\n", (int)lumart_text_n(t), lumart_text_p(t)); }
/* The game's messages as the runtime finds them: the holder's message holder and its game data, both null until the
   game has read its message files; one message, "Mod_Hi". */
static const uint16_t hello[] = { 'H', 'e', 'l', 'l', 'o', ' ', 0x1A, 0x0508, 0, 0, 0 };
static struct { void* messages; } holder;
static struct { void* data; } messages;
static int data;
static void holder_of(void** args, void* result) { (void)args; *(void**)result = &holder; }
static void message_of(void** args, void* result) {
    const char* k = *(const char* const*)args[1];
    *(const uint16_t**)result = strcmp(k, "Mod_Hi") == 0 ? hello : 0;
}
static const LumaFunction message_fn = { "MR::getGameMessageDirect", "const char16_t *(const char *)", message_of, 0 };
static const LumaFunction holder_fn = { "MR::getGameSystemObjHolder", "GameSystemObjHolder *()", holder_of, 0 };
static const LumaFunction* function(const char* name, const char* signature) {
    (void)signature;
    if (strcmp(name, message_fn.name) == 0) return &message_fn;
    if (strcmp(name, holder_fn.name) == 0) return &holder_fn;
    return 0;
}
static int32_t field(const char* c, const char* name, const char* type) {
    (void)type;
    if (strcmp(c, "GameSystemObjHolder") == 0 && strcmp(name, "mMessageHolder") == 0) return 0;
    if (strcmp(c, "MessageHolder") == 0 && strcmp(name, "mGameMessageData") == 0) return 0;
    return -1;
}
int main(void) {
    core.alloc = a; core.free = f; core.report = rep; core.log = lg; core.game_thread = gt;
    core.function = function; core.field = field;
    const uint16_t s[] = { 'H', 'i', ' ', '[', 0x1A, 0x0108, 0, 30, 0x1A, 0x0506, 0, 0x1A, 0x0306, 7, 0xD83D, 0xDE00, 0xD800,
                           0x1A, 0x0508, 0, 0, '!', 0 };
    const uint16_t m[] = { '{', 'r', 'o', 'w', '}', ' ', '(', 'y', 'o', 'u', ')', ' ', '{', 'r', 'o', 'w', '}', '{', 'x', 0 };
    lumart_text t = 0, k = 0, h = 0, n = 0, r = 0, g = 0;   /* an out text is released first: it starts empty */
    if (lumart_text_from_game16(&t, s, "t.c", 1) != 0) return 2;
    show(t);
    /* Before the game's messages exist (a mod's `on start`): rows. Read now, the key, said once. */
    if (lumart_text_message(&k, "Mod_Hello", 9, "main.lum", 2) != 0) return 3;
    if (lumart_text_message(&h, "Mod_Hi", 6, "main.lum", 3) != 0) return 3;
    /* Given to a library's export before the messages exist: still a row there, not the key copied. */
    lumart_text c = 0;
    if (lumart_text_cross(&c, h, "lib.c", 9) != 0) return 3;
    show(k);
    show(k);
    if (lumart_text_from_game16(&r, m, "t.c", 3) != 0) return 4;
    if (lumart_text_fill(&g, r, "row", 3, "1. Ana", 6, "t.c", 4) != 0) return 5;
    show(g);
    /* The game has read its messages: a row not read before is the message; one with no message stays its key. */
    holder.messages = &messages;
    messages.data = &data;
    show(h);
    show(k);
    if (lumart_text_message(&n, "Mod_Hi", 6, "main.lum", 4) != 0) return 3;
    show(n);
    show(c);
    lm_rel_lumart_text(&c);
#ifndef LUMART_MESSAGE_FUNCTION
    const uint16_t* hu = lumart_text_keep16(h, "t.c", 4, "f", "p");
    if (hu == 0) return 13;
    for (uint32_t i = 0; i < sizeof(hello) / sizeof(hello[0]); i++) {
        if (hu[i] != hello[i]) { printf("row keep16 differs at %u\n", i); return 14; }
    }
    printf("row keep16\n");
#endif
    lm_rel_lumart_text(&k);
    lm_rel_lumart_text(&h);
    lm_rel_lumart_text(&n);
    /* Back to the game (a `const char16_t *` result): every tag word is the game's tag again, "[[" one '['. */
    static const char words[] = "Hi [[[wait 30][tag 5 0][icon 7]![player][COLOR 3]a[color 0][page][center][small][normal]"
                                "[large][sound SE_SV_TICO][tag 6 5 0 0x1 2][ wait  0x10 ][race_time][best_time 3][font AB][middle][number 5 2][string 1 0][player_name 1]";
    static const uint16_t game[] = { 'H', 'i', ' ', '[', 0x1A, 0x0108, 0, 30, 0x1A, 0x0506, 0, 0x1A, 0x0306, 7, '!',
                                     0x1A, 0x0508, 0, 0, 0x1A, 0xFF08, 0, 0x300, 'a', 0x1A, 0xFF08, 0, 0, 0x1A, 0x0106, 1,
                                     0x1A, 0x0106, 3, 0x1A, 0x0406, 0, 0x1A, 0x0406, 1, 0x1A, 0x0406, 2, 0x1A, 0x021A, 0,
                                     'S', 'E', '_', 'S', 'V', '_', 'T', 'I', 'C', 'O', 0x1A, 0x060C, 5, 0, 1, 2,
                                     0x1A, 0x0108, 0, 16, 0x1A, 0x0906, 5, 0x1A, 0x0906, 3, 0x1A, 0x0A0A, 0, 'A', 'B', 0x1A, 0x0106, 2,
                                     0x1A, 0x060E, 5, 0, 0, 0, 2, 0x1A, 0x070E, 1, 0, 0, 0, 0, 0x1A, 0x0508, 0, 0x100, 0 };
    lumart_text w = 0;
    if (lumart_text_make(&w, words, sizeof(words) - 1, "t.c", 5) != 0) return 6;
    const uint16_t* u = lumart_text_keep16(w, "t.c", 5, "f", "p");
    if (u == 0) return 7;
    for (uint32_t i = 0; i < sizeof(game) / sizeof(game[0]); i++) {
        if (u[i] != game[i]) { printf("keep16 differs at %u: %04x, not %04x\n", i, u[i], game[i]); return 8; }
    }
    printf("keep16 tags\n");
    lumart_text back = 0;
    if (lumart_text_from_game16(&back, game, "t.c", 7) != 0) return 12;
    show(back);
    /* Brackets that name no tag stay as written. */
    static const char plain[] = "[a] [nope 1] [wait] [wait x] [color 256] [wait 65536] [open";
    lumart_text p = 0;
    if (lumart_text_make(&p, plain, sizeof(plain) - 1, "t.c", 6) != 0) return 9;
    const uint16_t* q = lumart_text_keep16(p, "t.c", 6, "f", "p");
    if (q == 0) return 10;
    for (uint32_t i = 0; i <= sizeof(plain) - 1; i++) {
        if (q[i] != (uint16_t)(i < sizeof(plain) - 1 ? plain[i] : 0)) { printf("plain differs at %u\n", i); return 11; }
    }
    printf("keep16 plain\n");
    /* The mod's end: the kept texts, their UTF-16 copies and every pool page back to the core. */
    lumart_end();
    printf("held %d\n", live);
    return 0;
}
"#;

/// An out buffer with a `uint32_t size` and a `uint32_t* length` after it (input.text_peek, the SDK's header): its
/// length asked with a size of 0, then the text read with room for it and its 0 byte; the caret after it comes back
/// as a value beside the text.
#[test]
fn an_out_buffer_with_a_32_bit_size_and_length_gives_text() {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("skipped: LUMA_SDK is not set (the SDK's luma_input.h)");
        return;
    };
    let mut headers = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(&sdk).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("luma_") && name.ends_with(".h") {
            headers.insert(name, std::fs::read_to_string(e.path()).unwrap());
        }
    }
    let (services, _) = lumascript::svc::Services::of_sdk(&headers);
    let face = &services.faces["input"];
    let f = face.function("text_peek").unwrap();
    assert!(matches!(f.shape, lumascript::svc::Shape::Callable { .. }), "{:?}", f.shape);
    let text = "use svc.input

var shown = \"\"
var at: u32 = 0

fn peek(r: input.Request) {
    let (t, caret) = input.text_peek(r)
    shown = t
    at = caret
}
";
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = game();
    game.services = services.clone();
    lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "the checker refuses: {errors:?}");
        let out = emit::emit("small", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}"));
        let main = out.files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap();
        assert!(main.contains("->text_peek(luma_rt_self, ") && main.contains(", 0, 0, &"), "{main}");
        assert!(main.contains(" + 1u;"), "room for the 0 byte: {main}");
        let mut files = out.files.clone();
        for (name, text) in &headers {
            files.push((format!("lum/{name}"), text.clone()));
        }
        compiles(&files);
    })
    .unwrap();
}
