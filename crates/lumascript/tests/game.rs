//! The game's type text to LumaScript (checker 4.2 part a): each rule of `lumascript::game` on a fixture in the types
//! file's form, and an ignored test over a kernel build's real rows and types file.

use luma_validate::names::{Names, Types};
use lumascript::game::{FnTy, Game, GameTy, NotCrossing, PlaceTy, Scalar, SigNotCrossing, Value};
use std::collections::{BTreeMap, HashMap};

/// A types file in its form with one case of each rule.
const FIXTURE: &str = r#"{
  "types": [
    {"kind": "struct", "name": "Vec", "size": 12, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [],
     "fields": [{"name": "x", "type": "f32", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "y", "type": "f32", "offset": 4, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "z", "type": "f32", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Vec3<f32>", "size": 12, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false,
     "bases": [{"name": "Vec", "offset": 0, "virtual": false}], "fields": [],
     "reachable": [{"name": "x", "from": "Vec", "offset": 0, "type": "f32"}, {"name": "y", "from": "Vec", "offset": 4, "type": "f32"},
                   {"name": "z", "from": "Vec", "offset": 8, "type": "f32"}]},
    {"kind": "class", "name": "Vec4f", "size": 12, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "struct", "name": "Mtx34f", "size": 48, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [],
     "fields": [{"name": "mMtx", "type": "f32", "offset": 0, "count": 12, "extents": [3, 4], "access": "public", "bits": null}]},
    {"kind": "class", "name": "Color8", "size": 4, "align": 1, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [],
     "fields": [{"name": "r", "type": "u8", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "g", "type": "u8", "offset": 1, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "b", "type": "uint8_t", "offset": 2, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "a", "type": "unsigned char", "offset": 3, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "Mario::MovementStates", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [],
     "fields": [{"name": "onGround", "type": "unsigned int : 1", "offset": 0, "count": 1, "extents": null, "access": "public", "bits": {"width": 1, "offset": 0}}]},
    {"kind": "class", "name": "LiveActor", "size": 200, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [],
     "fields": [{"name": "mPos", "type": "Vec3f", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mFlags", "type": "u32 : 3", "offset": 20, "count": 1, "extents": null, "access": "protected", "bits": {"width": 3, "offset": 0}},
                {"name": "mMode", "type": "Mode : 2", "offset": 20, "count": 1, "extents": null, "access": "public", "bits": {"width": 2, "offset": 3}},
                {"name": "mGrid", "type": "f32", "offset": 24, "count": 12, "extents": [3, 4], "access": "public", "bits": null},
                {"name": "mRow", "type": "s16", "offset": 72, "count": 4, "extents": null, "access": "public", "bits": null},
                {"name": "mName", "type": "const char *", "offset": 80, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mStates", "type": "Mario::MovementStates", "offset": 88, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mTarget", "type": "LiveActor *", "offset": 96, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mScale", "type": "const f32 *", "offset": 104, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mBaseMtx", "type": "Mtx", "offset": 112, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mCount", "type": "const s32", "offset": 160, "count": 1, "extents": null, "access": "public", "bits": null},
                {"name": "mWide", "type": "f32 : 2", "offset": 164, "count": 1, "extents": null, "access": "public", "bits": {"width": 2, "offset": 0}},
                {"name": "mBuf", "type": "char *", "offset": 168, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "struct", "name": "MR::GrowBuffer", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": true, "bases": [], "reachable": [], "fields": []},
    {"kind": "struct", "name": "SoundID", "size": 4, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": false, "trivial_destroy": true, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "struct", "name": "NebulaListLink<Mario>", "size": 8, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": true, "no_copy": true, "bases": [], "reachable": [], "fields": []},
    {"kind": "struct", "name": "Sealed", "size": 4, "align": 4, "polymorphic": false, "abstract": false, "trivial_copy": true, "trivial_destroy": false, "no_copy": true, "bases": [], "reachable": [], "fields": []},
    {"kind": "alias", "name": "size_t", "of": "unsigned long", "record": null},
    {"kind": "alias", "name": "Vec3f", "of": "Vec3<f32>", "record": "Vec3<f32>", "file": null, "line": null, "row": false},
    {"kind": "spelling", "name": "Vec3<float>", "of": "Vec3<f32>", "record": "Vec3<f32>", "file": null, "line": null, "row": true},
    {"kind": "alias", "name": "f32", "of": "float", "record": null},
    {"kind": "alias", "name": "u8", "of": "unsigned char", "record": null},
    {"kind": "alias", "name": "s16", "of": "short", "record": null},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "alias", "name": "u32", "of": "unsigned int", "record": null},
    {"kind": "alias", "name": "Mtx", "of": "f32[3][4]", "record": null},
    {"kind": "alias", "name": "MtxPtr", "of": "f32 (*)[4]", "record": null},
    {"kind": "alias", "name": "Mtx44Ptr", "of": "f32 (*)[4]", "record": null},
    {"kind": "alias", "name": "Loop", "of": "Loop2", "record": null},
    {"kind": "alias", "name": "Loop2", "of": "Loop", "record": null},
    {"kind": "enum", "name": "Mode", "underlying": "u8", "scoped": false, "values": [{"name": "Mode_Off", "value": 0}, {"name": "Mode_On", "value": 1}]}
  ],
  "globals": [
    {"name": "gCount", "type": "s32", "const": false, "defined": true, "value": null},
    {"name": "gLimit", "type": "const f32", "const": true, "defined": true, "value": "1.5"},
    {"name": "gActor", "type": "LiveActor *", "const": false, "defined": true, "value": null},
    {"name": "gDeclared", "type": "s32", "const": false, "defined": false, "value": null},
    {"name": "ChordInfo::OCTAVE", "type": "const s32", "const": true, "defined": false, "value": "12"},
    {"name": "Table::cOrigin", "type": "const Vec3f", "const": true, "defined": false, "value": "1"}
  ],
  "left_out": [
    {"kind": "record", "name": "Gone", "file": "a.hpp", "line": 1, "reason": "no size (incomplete or dependent)"},
    {"kind": "template", "name": "TBox", "file": "a.hpp", "line": 2, "reason": "a template not used with these arguments"},
    {"kind": "global", "name": "gDeclared", "file": "a.hpp", "line": 3, "reason": "declared, defined nowhere in the game"}
  ]
}"#;

fn game() -> Game {
    let fixture = FIXTURE.replace("\"value\": \"1.5\"", "\"value\": 1.5").replace("\"value\": \"12\"", "\"value\": 12").replace("\"value\": \"1\"", "\"value\": 1");
    let types = Types::from_json(fixture.as_bytes()).unwrap();
    Game::new(HashMap::new(), Some(types))
}

fn place(of: GameTy, konst: bool) -> GameTy {
    GameTy::Place { of: Box::new(of), konst }
}

fn opt(of: GameTy) -> GameTy {
    GameTy::Opt(Box::new(of))
}

fn view(record: &str, konst: bool) -> GameTy {
    GameTy::View { record: record.to_string(), konst }
}

#[test]
fn the_value_records_are_taken_by_their_layout_only() {
    let game = game();
    let sig = |text: &str| game.signature(text).unwrap().params;
    assert_eq!(sig("void(Vec3f)"), vec![GameTy::Value(Value::Vec3)]);
    assert_eq!(sig("void(Vec3<float>)"), vec![GameTy::Value(Value::Vec3)]);
    assert_eq!(sig("void(Vec)"), vec![GameTy::Value(Value::Vec3)]);
    assert_eq!(sig("void(Color8)"), vec![GameTy::Value(Value::Color)]);
    assert_eq!(sig("void(Mtx34f)"), vec![GameTy::Value(Value::Mat34)]);
    // Vec4f is in the table but 12 bytes here: a game struct, and said.
    assert_eq!(sig("void(Vec4f)"), vec![GameTy::Struct("Vec4f".into())]);
    assert_eq!(game.value_mismatches, vec![("Vec4f".to_string(), Value::Vec4, "12 bytes, not 16".to_string())]);
}

#[test]
fn parameters_and_results_cross_as_design_section_3_says() {
    let game = game();
    let ok = |text: &str| game.signature(text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
    let refused = |text: &str| game.signature(text).unwrap_err();
    let fn_ty = |params: Vec<GameTy>, result: GameTy| FnTy { params, result, konst: false, counts: Vec::new(), transient: Vec::new(), opaque: Vec::new(), text16_result: false };
    // Numbers by width, through the file's typedefs and the fixed-width names.
    assert_eq!(ok("void(s32, u8, uint32_t, bool, double, long long)"), fn_ty(
        vec![GameTy::Scalar(Scalar::I32), GameTy::Scalar(Scalar::U8), GameTy::Scalar(Scalar::U32), GameTy::Scalar(Scalar::Bool), GameTy::Scalar(Scalar::F64), GameTy::Scalar(Scalar::I64)],
        GameTy::Unit,
    ));
    // A value by const reference is read; by reference the game writes it (`var`).
    assert_eq!(ok("void(const Vec3f &, Vec3f &)").params, vec![place(GameTy::Value(Value::Vec3), true), place(GameTy::Value(Value::Vec3), false)]);
    assert_eq!(ok("void(s32 &)").params, vec![place(GameTy::Scalar(Scalar::I32), false)]);
    assert_eq!(ok("void(Mode, const Mode &)").params, vec![GameTy::Enum("Mode".into()), place(GameTy::Enum("Mode".into()), true)]);
    // `const Mtx` is received as a pointer to its rows; `Mtx &` and `const Mtx &` too.
    assert_eq!(ok("void(const Mtx)").params, vec![place(GameTy::Value(Value::Mat34), true)]);
    assert_eq!(ok("void(Mtx)").params, vec![place(GameTy::Value(Value::Mat34), false)]);
    assert_eq!(ok("void(const Mtx &)").params, vec![place(GameTy::Value(Value::Mat34), true)]);
    // Records by pointer or reference are views, a pointer `T?`; `T &&` as `T &`.
    assert_eq!(ok("LiveActor *(const LiveActor &, LiveActor &&, const Mario::MovementStates *)"), fn_ty(
        vec![view("LiveActor", true), view("LiveActor", false), opt(view("Mario::MovementStates", true))],
        opt(view("LiveActor", false)),
    ));
    // A plain record by value is the script's copy; a class by value is refused.
    assert_eq!(ok("Mario::MovementStates(Mario::MovementStates)"), fn_ty(vec![GameTy::Struct("Mario::MovementStates".into())], GameTy::Struct("Mario::MovementStates".into())));
    assert_eq!(refused("void(LiveActor)"), SigNotCrossing { param: Some(0), why: NotCrossing::ClassByValue("LiveActor".into()) });
    assert_eq!(refused("LiveActor()"), SigNotCrossing { param: None, why: NotCrossing::ClassByValue("LiveActor".into()) });
    // By value only plain data: one that frees what it holds, or copies itself by its own code, is refused.
    assert_eq!(refused("void(MR::GrowBuffer)").why, NotCrossing::OwnsMemory("MR::GrowBuffer".into()));
    assert_eq!(refused("SoundID()").why, NotCrossing::CopiesItself("SoundID".into()));
    // Written as a type, a record that cannot be the script's copy is its view (decision (a)).
    assert_eq!(game.type_by_name("SoundID"), Ok(GameTy::View { record: "SoundID".into(), konst: false }));
    // One no copy can be called on, though clang calls it trivially copyable; said after OwnsMemory.
    assert_eq!(refused("void(NebulaListLink<Mario>)").why, NotCrossing::CannotCopy("NebulaListLink<Mario>".into()));
    assert_eq!(game.type_by_name("NebulaListLink<Mario>"), Ok(GameTy::View { record: "NebulaListLink<Mario>".into(), konst: false }));
    // A deleted destructor (trivially copyable, not trivially destructible) is its own reason.
    assert_eq!(refused("void(Sealed)").why, NotCrossing::CannotDrop("Sealed".into()));
    assert_eq!(game.type_by_name("Sealed"), Ok(GameTy::View { record: "Sealed".into(), konst: false }));
    assert_eq!(game.type_by_name("Mario::MovementStates"), Ok(GameTy::Struct("Mario::MovementStates".into())));
    // The pointer-width names by name, whatever the platform's file says.
    assert_eq!(ok("size_t(uintptr_t, intptr_t, ptrdiff_t)"), fn_ty(
        vec![GameTy::Scalar(Scalar::U64), GameTy::Scalar(Scalar::I64), GameTy::Scalar(Scalar::I64)], GameTy::Scalar(Scalar::U64)));
    // Text.
    assert_eq!(ok("const char *(const char *)"), fn_ty(vec![opt(GameTy::Text)], opt(GameTy::Text)));
    assert_eq!(refused("void(s32, char *)"), SigNotCrossing { param: Some(1), why: NotCrossing::Buffer });
    // A pointer to a number or vector: a result is a place in the game's storage, a parameter is not known enough.
    assert_eq!(ok("const Vec3f *()").result, opt(place(GameTy::Value(Value::Vec3), true)));
    assert_eq!(ok("f32 *()").result, opt(place(GameTy::Scalar(Scalar::F32), false)));
    assert_eq!(ok("MtxPtr()").result, opt(place(GameTy::Value(Value::Mat34), false)));
    assert_eq!(ok("const Mtx *()").result, opt(place(GameTy::Value(Value::Mat34), true)));
    for text in ["void(f32 *)", "void(const Vec3f *)", "void(MtxPtr)", "void(const Mtx *)", "void(Mode *)"] {
        assert_eq!(refused(text), SigNotCrossing { param: Some(0), why: NotCrossing::PointerToValue }, "{text}");
    }
    // Matrix rows only by the game's typedef: a raw f32 (*)[4] or another typedef of it may be 4x4.
    assert_eq!(refused("void(const f32 (*)[4])").why, NotCrossing::MatrixRows);
    assert_eq!(refused("void(Mtx44Ptr)").why, NotCrossing::MatrixRows);
    // Each refusal names its reason.
    let why = |text: &str| refused(text).why;
    assert_eq!(why("void(void *)"), NotCrossing::VoidPointer);
    assert_eq!(why("void(long)"), NotCrossing::PlatformWidth("long".into()));
    assert_eq!(why("void(wchar_t)"), NotCrossing::PlatformWidth("wchar_t".into()));
    assert_eq!(why("void(char)"), NotCrossing::CharSign);
    assert_eq!(why("void(int, ...)"), NotCrossing::Variadic);
    assert_eq!(why("void(void (*)(int))"), NotCrossing::Function);
    assert_eq!(why("void(int LiveActor::*)"), NotCrossing::MemberPointer);
    assert_eq!(why("void(LiveActor **)"), NotCrossing::PointerToPointer);
    assert_eq!(why("void(volatile s32 &)"), NotCrossing::Volatile);
    assert_eq!(why("void(Unknown *)"), NotCrossing::NotKnown("Unknown".into()));
    assert_eq!(why("void(Gone *)"), NotCrossing::LeftOut { name: "Gone".into(), reason: "no size (incomplete or dependent)".into() });
    assert_eq!(why("void(TBox<s8> &)"), NotCrossing::LeftOut { name: "TBox<s8>".into(), reason: "a template not used with these arguments".into() });
    assert_eq!(why("void(Loop)"), NotCrossing::Other("Loop2".into()));
    assert!(matches!(why("void(s32"), NotCrossing::Unreadable(_)));
    // A constructor's row has no result; a const member function's `self` is const.
    assert_eq!(ok("(const char *)"), fn_ty(vec![opt(GameTy::Text)], GameTy::Unit));
    assert!(ok("s32() const").konst);
}

#[test]
fn fields_are_places_of_their_kind() {
    let game = game();
    let types = game.types.as_ref().unwrap();
    let actor = &types.records["LiveActor"];
    let field = |name: &str| game.field(actor.own.iter().find(|f| f.name == name).unwrap());
    let place_ty = |ty: GameTy, read_only: bool| Ok(PlaceTy { ty, read_only, constant: None });
    assert_eq!(field("mPos"), place_ty(GameTy::Value(Value::Vec3), false));
    assert_eq!(field("mFlags"), place_ty(GameTy::Bits { of: Scalar::U32, width: 3 }, false));
    // An enum bit-field is stored as its underlying integer.
    assert_eq!(field("mMode"), place_ty(GameTy::Bits { of: Scalar::U8, width: 2 }, false));
    assert_eq!(field("mGrid"), place_ty(GameTy::Array { of: Box::new(GameTy::Scalar(Scalar::F32)), extents: vec![3, 4] }, false));
    assert_eq!(field("mRow"), place_ty(GameTy::Array { of: Box::new(GameTy::Scalar(Scalar::I16)), extents: vec![4] }, false));
    // The game's text is read; writing a script's text into the game's object is not offered.
    assert_eq!(field("mName"), place_ty(opt(GameTy::Text), true));
    // A plain struct inside a view is a view: `actor.mStates.onGround = true` writes the game's object.
    assert_eq!(field("mStates"), place_ty(view("Mario::MovementStates", false), false));
    assert_eq!(field("mTarget"), place_ty(opt(view("LiveActor", false)), false));
    assert_eq!(field("mScale"), place_ty(opt(place(GameTy::Scalar(Scalar::F32), true)), false));
    assert_eq!(field("mBaseMtx"), place_ty(GameTy::Value(Value::Mat34), false));
    assert_eq!(field("mCount"), place_ty(GameTy::Scalar(Scalar::I32), true));
    assert_eq!(field("mWide"), Err(NotCrossing::BitFieldType("f32 : 2".into())));
    assert_eq!(field("mBuf"), Err(NotCrossing::Buffer));
    let states = &types.records["Mario::MovementStates"];
    assert_eq!(game.field(&states.own[0]), place_ty(GameTy::Bits { of: Scalar::U32, width: 1 }, false));
}

#[test]
fn globals_are_places_and_a_const_one_is_read_only() {
    let game = game();
    assert_eq!(game.global("gCount"), Ok(PlaceTy { ty: GameTy::Scalar(Scalar::I32), read_only: false, constant: None }));
    assert_eq!(game.global("gLimit"), Ok(PlaceTy { ty: GameTy::Scalar(Scalar::F32), read_only: true, constant: None }));
    assert_eq!(game.global("gActor"), Ok(PlaceTy { ty: opt(view("LiveActor", false)), read_only: false, constant: None }));
    // A constant the module does not define is its value, folded; only a number's or an enum's.
    assert_eq!(game.global("ChordInfo::OCTAVE"), Ok(PlaceTy { ty: GameTy::Scalar(Scalar::I32), read_only: true, constant: Some("12".into()) }));
    assert_eq!(game.global("Table::cOrigin"), Err(NotCrossing::Other("const Vec3f".into())));
    assert_eq!(
        game.global("gDeclared"),
        Err(NotCrossing::LeftOut { name: "gDeclared".into(), reason: "declared, defined nowhere in the game".into() })
    );
    assert_eq!(game.global("gNone"), Err(NotCrossing::NotKnown("gNone".into())));
}

#[test]
fn with_no_types_file_only_builtins_and_fixed_widths_cross() {
    let game = Game::new(HashMap::new(), None);
    assert_eq!(game.signature("int(int, uint16_t)").map(|f| f.params), Ok(vec![GameTy::Scalar(Scalar::I32), GameTy::Scalar(Scalar::U16)]));
    assert_eq!(game.signature("u8()"), Err(SigNotCrossing { param: None, why: NotCrossing::NotKnown("u8".into()) }));
    assert_eq!(game.global("gCount"), Err(NotCrossing::NotKnown("gCount".into())));
}

/// The kind of a refusal, for counting.
fn kind(why: &NotCrossing) -> String {
    let text = format!("{why:?}");
    text.split(['(', ' ', '{']).next().unwrap_or("").to_string()
}

/// A kernel build's rows and types file (`LUMA_ROWS=<build>/luma LUMA_TYPES=<build>/luma_types/luma_types.json cargo
/// test -p lumascript --release --test game -- --ignored --nocapture`): every signature, field and global maps or is
/// said with its reason, never a panic; every table record the file has, has the table's layout. Prints the counts
/// (the build report's) and, for each reason, the names most often behind it.
#[test]
#[ignore]
fn a_kernel_builds_types_map_or_say_why() {
    let rows = std::env::var("LUMA_ROWS").expect("LUMA_ROWS names a kernel build's luma folder");
    let mut names = Names::from_rows_folder(std::path::Path::new(&rows), 1).unwrap_or_else(|e| panic!("{e}"));
    if names.types.is_none() {
        let path = std::env::var("LUMA_TYPES").expect("LUMA_TYPES names its luma_types.json");
        names.types = Some(Types::from_json(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{path}: {e}")));
    }
    let game = Game::from_names(&names);
    let types = game.types.as_ref().unwrap();
    assert!(game.value_mismatches.is_empty(), "{:?}", game.value_mismatches);
    for (name, _) in lumascript::game::VALUE_RECORDS {
        eprintln!("value table {name}: {}", if types.records.contains_key(name) { "in the file, layout checked" } else { "not in the file" });
    }
    if let Ok(why) = std::env::var("LUMA_SHOW") {
        // The signatures refused for one reason, to read them (`LUMA_SHOW=Other`).
        let mut shown: Vec<String> = game.functions.iter().flat_map(|(name, list)| list.iter().map(move |(sig, _)| (name, sig)))
            .filter_map(|(name, sig)| game.signature(sig).err().filter(|e| kind(&e.why) == why).map(|e| format!("{name} {sig}: {}", e.why)))
            .collect();
        shown.sort();
        eprintln!("{}", shown.join("\n"));
    }
    let mut report: BTreeMap<&str, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
    let mut not_known: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |what: &'static str, result: Result<(), NotCrossing>| {
        let entry = report.entry(what).or_default();
        match result {
            Ok(()) => entry.0 += 1,
            Err(why) => {
                if let NotCrossing::NotKnown(name) | NotCrossing::LeftOut { name, .. } = &why {
                    *not_known.entry(name.clone()).or_default() += 1;
                }
                *entry.1.entry(kind(&why)).or_default() += 1;
            }
        }
    };
    for list in game.functions.values() {
        for (sig, _) in list {
            count("signatures", game.signature(sig).map(|_| ()).map_err(|e| e.why));
        }
    }
    for record in types.records.values() {
        for field in &record.own {
            count("fields", game.field(field).map(|_| ()));
        }
    }
    for (name, global) in &types.global_facts {
        if global.defined {
            count("globals", game.global(name).map(|_| ()));
        } else if global.value.is_some() {
            count("constants", game.global(name).map(|_| ()));
        }
    }
    for (what, (crossing, refused)) in &report {
        let total = crossing + refused.values().sum::<usize>();
        eprintln!("{what}: {crossing} of {total} cross");
        for (why, n) in refused {
            eprintln!("    {n:6} {why}");
        }
    }
    let mut common: Vec<_> = not_known.into_iter().collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    eprintln!("names not known or left out, most used first: {:?}", &common[..common.len().min(25)]);
}
