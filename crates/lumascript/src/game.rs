//! The game as the checker sees it (checker design, sections 2 and 3, revision
//! 2): the function rows and the types file as the validator's one reader gives them, and the mapping of the game's C++
//! type text to what crosses into LumaScript. Nothing here guesses: a text it cannot map says why, and the checker
//! then says that function, field or global cannot be used from LumaScript.
//!
//! What crosses ([`GameTy`]):
//! - numbers by width (`s32` is `i32`), `bool`, the game's enums;
//! - the vector, quaternion, matrix and colour records of [`VALUE_RECORDS`], each checked against its layout in the
//!   types file, as LumaScript's `vec2` .. `color`;
//! - `const char *` as `text`;
//! - a record reached by pointer or reference, or as a field, as a view: a place in the game's object;
//! - a record by value that is no class, as a game struct the script owns;
//! - a number, vector or enum reached by reference (a parameter the game reads or writes), by a pointer result or a
//!   pointer field, as a place in the game's storage;
//! - fields: bit-fields and arrays as places of their own kind (docs/modding/language/types.md).
//!
//! A pointer may be null until the self-description marks it never-null (request), so every pointer maps to
//! `Opt`. A reference never is.

use crate::ctype::{self, CType, CTypeError, Signature};
use luma_validate::names::{Field, FunctionFacts, FunctionKind, LeftOutKind, Names, Record, Types};
use std::collections::HashMap;
use std::fmt;

/// A number type of LumaScript (docs/modding/language/language.md, section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}

/// LumaScript's value types the game has as records or arrays (docs/modding/language/language.md, section 7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Vec2,
    Vec3,
    Vec4,
    Quat,
    Mat34,
    /// `color`: four u8 channels in the game, scaled by 255 in and out (checker).
    Color,
}

/// The records LumaScript reads as its value types, by their row names in the types file (checker). Each is
/// taken only when the types file gives it the layout of [`Value`] (see [`Game::new`]); a record this table lacks is
/// a game struct, never a guess by its name.
pub const VALUE_RECORDS: [(&str, Value); 9] = [
    ("Vec2<f32>", Value::Vec2),
    ("Vec3<f32>", Value::Vec3),
    ("Vec", Value::Vec3),
    ("Vec4f", Value::Vec4),
    ("Quat4f", Value::Quat),
    ("Quaternion", Value::Quat),
    ("Mtx34f", Value::Mat34),
    ("Color8", Value::Color),
    ("NebulaColor8", Value::Color),
];

/// The game's typedefs of a pointer to a matrix's rows. A raw `f32 (*)[4]` does not say how many rows it points at
/// (`Frame::setPerspective(const f32 (*)[4])` takes a 4x4 projection), so only these names cross, each checked to be
/// a typedef of `f32 (*)[4]`.
pub const MATRIX_POINTERS: [(&str, Value); 1] = [("MtxPtr", Value::Mat34)];

/// The <stdint.h> names of a fixed width, which every platform gives the same type. (`size_t`, `uintptr_t` and the
/// like are as wide as a pointer, so they cross only through the types file's own alias rows.)
const FIXED_WIDTH: [(&str, Scalar); 8] = [
    ("int8_t", Scalar::I8),
    ("int16_t", Scalar::I16),
    ("int32_t", Scalar::I32),
    ("int64_t", Scalar::I64),
    ("uint8_t", Scalar::U8),
    ("uint16_t", Scalar::U16),
    ("uint32_t", Scalar::U32),
    ("uint64_t", Scalar::U64),
];

/// The names as wide as a pointer. The types file's alias rows give each platform's own spelling
/// (`unsigned long long` on Windows, `unsigned long` elsewhere), so they are taken by name, before the file: every
/// target is 64-bit.
const POINTER_WIDTH: [(&str, Scalar); 4] = [("size_t", Scalar::U64), ("uintptr_t", Scalar::U64), ("intptr_t", Scalar::I64), ("ptrdiff_t", Scalar::I64)];

/// What a game type is in LumaScript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameTy {
    /// `void` as a result.
    Unit,
    Scalar(Scalar),
    Value(Value),
    /// A game enum by its row's name, by value.
    Enum(String),
    /// `const char *`: the game's text, read.
    Text,
    /// A record that is plain data, by value: the script's own copy (`GameStruct`; [`Game::by_value`]).
    Struct(String),
    /// A record in the game's memory: its fields are read and written in the game's object.
    View { record: String, konst: bool },
    /// A number, value or enum in the game's storage: a parameter the game writes (`var`) unless const, a result or
    /// field read and written through it.
    Place { of: Box<GameTy>, konst: bool },
    /// A pointer: none when null.
    Opt(Box<GameTy>),
    /// A bit-field, read and written only through LUMA_BITS_GET/SET (docs/modding/language/compiled-form.md).
    Bits { of: Scalar, width: u32 },
    /// An array field, each extent indexed with a check; outermost extent first.
    Array { of: Box<GameTy>, extents: Vec<u64> },
    /// A marked array parameter (docs/modding/language/game-description.md, `LUMA_COUNT` or `LUMA_COUNT_FIXED`): the script's
    /// `list<T>`, its buffer passed; read only by the game when `konst` (way `in`), else written (`var xs`).
    /// `fixed`: exactly that many elements.
    List { of: Box<GameTy>, konst: bool, fixed: Option<u32> },
    /// A pointer parameter the script can only leave out: marked `LUMA_NULLABLE` where its type does not cross
    /// otherwise (an address the game keeps, `LUMA_KEPT`; a `void *`). The argument is `none`, passed as null.
    Null,
}

/// The record of a `kept` and `nullable` parameter's view: no record of the game's has this name, so no script
/// value is of its type and the parameter takes `none` alone, passed as null.
pub const KEPT_NONE: &str = "kept pointer (LUMA_KEPT)";

/// Where a type text stands: it decides what a record by value or a reference is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Param,
    /// A hooked function's parameter (design revision 3): as `Param`, but a pointer to a number, value or enum is
    /// received as the place it points at (`T?`, as a result's), never refused for that.
    HookParam,
    Result,
    Field,
    Global,
}

/// Why a type does not cross into LumaScript (English for the build report and the tests; the checker says it in its
/// own rows, 14 languages, at the use).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotCrossing {
    /// The text does not read as a C++ type.
    Unreadable(CTypeError),
    /// A name the self-description has no row for and does not list as left out.
    NotKnown(String),
    /// A name the generator left out, with its reason (the generator's phrase).
    LeftOut { name: String, reason: String },
    /// `long`, `wchar_t`, `long double`: a width that differs between the platforms.
    PlatformWidth(String),
    /// `char` by value: signed on some platforms, unsigned on others.
    CharSign,
    /// `char *`: a buffer the game writes.
    Buffer,
    VoidPointer,
    /// `void` where a value goes.
    VoidValue,
    /// A function or a pointer to one.
    Function,
    MemberPointer,
    PointerToPointer,
    Volatile,
    /// A parameter that points at a number, vector or enum: one value or many, kept by the game or not, the row does
    /// not say (its header has no mark).
    PointerToValue,
    /// A pointer parameter the game keeps (`LUMA_KEPT`, a registration): never from LumaScript.
    Kept,
    /// A parameter mark the type cannot carry (a write through a const pointer; a count that is no integer).
    MarkNotFit,
    /// `f32 (*)[4]` not written as a matrix typedef: how many rows, the text does not say.
    MatrixRows,
    /// A class (with a vtable) by value: copying one is not a thing the game does.
    ClassByValue(String),
    /// A record by value that frees what it holds when dropped (types `trivial_destroy` false): a
    /// script's copy would never be dropped, or two copies would free one block.
    OwnsMemory(String),
    /// A record by value whose destructor is deleted (clang calls it trivially copyable and not trivially
    /// destructible): a script's copy could never be dropped.
    CannotDrop(String),
    /// A record by value whose copy is its own code (`trivial_copy` false): a bit copy is not its copy.
    CopiesItself(String),
    /// A record no copy or move can be called on (types `no_copy`). clang calls it trivially
    /// copyable, yet the game never copies one, and a C call would pass it by invisible reference on Itanium.
    CannotCopy(String),
    /// A function taking `...`.
    Variadic,
    /// A bit-field of a type that is no integer, bool or enum.
    BitFieldType(String),
    /// Anything else this mapping does not take, by its text (a reference to an array, an array of unknown length).
    Other(String),
}

impl fmt::Display for NotCrossing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotCrossing::Unreadable(e) => write!(f, "the type text does not read ({e})"),
            NotCrossing::NotKnown(name) => write!(f, "\"{name}\" is not known in this game's self-description"),
            NotCrossing::LeftOut { name, reason } => write!(f, "\"{name}\" is left out of the self-description: {reason}"),
            NotCrossing::PlatformWidth(name) => write!(f, "\"{name}\" is not one width on every platform"),
            NotCrossing::CharSign => write!(f, "a char is signed on some platforms and unsigned on others"),
            NotCrossing::Buffer => write!(f, "a char * is a buffer the game writes"),
            NotCrossing::VoidPointer => write!(f, "a void * is memory of no known type"),
            NotCrossing::VoidValue => write!(f, "void is no value"),
            NotCrossing::Function => write!(f, "a function is not a value LumaScript passes"),
            NotCrossing::MemberPointer => write!(f, "a pointer to a member"),
            NotCrossing::PointerToPointer => write!(f, "a pointer to a pointer"),
            NotCrossing::Volatile => write!(f, "a volatile type"),
            NotCrossing::PointerToValue => write!(f, "a pointer parameter to a number or vector: one or many, kept or not, the row does not say"),
            NotCrossing::Kept => write!(f, "a pointer parameter the game keeps (LUMA_KEPT)"),
            NotCrossing::MarkNotFit => write!(f, "a LUMA_ mark its type cannot carry"),
            NotCrossing::MatrixRows => write!(f, "f32 (*)[4] does not say how many rows it points at"),
            NotCrossing::ClassByValue(name) => write!(f, "the class \"{name}\" by value"),
            NotCrossing::OwnsMemory(name) => write!(f, "\"{name}\" by value frees what it holds when dropped"),
            NotCrossing::CannotDrop(name) => write!(f, "\"{name}\" cannot be dropped: its destructor is deleted"),
            NotCrossing::CopiesItself(name) => write!(f, "\"{name}\" by value is copied by its own code"),
            NotCrossing::CannotCopy(name) => write!(f, "\"{name}\" cannot be copied"),
            NotCrossing::Variadic => write!(f, "a function taking ..."),
            NotCrossing::BitFieldType(text) => write!(f, "a bit-field of \"{text}\", no integer"),
            NotCrossing::Other(text) => write!(f, "\"{text}\" is no type LumaScript takes"),
        }
    }
}

/// A function row's signature in LumaScript types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnTy {
    /// The parameters the script writes: every C parameter but the counts below.
    pub params: Vec<GameTy>,
    /// The marked counts: (the count parameter's C index, its list parameter's C index). The compiler passes the list's
    /// length there; the script writes no argument for it.
    pub counts: Vec<(usize, usize)>,
    /// `Unit` for `void` and for a constructor's or destructor's row.
    pub result: GameTy,
    /// A `const` member function: `self` is a const view.
    pub konst: bool,
    /// The `const char *` parameters (C indices) marked `in`: the game reads the text during the call only, so a
    /// text made for it lives in the scratch until the call returns (docs/modding/language/language.md, section 6.4, docs/modding/language/compiled-form.md), not kept.
    pub transient: Vec<usize>,
    /// A hook's parameters that do not cross (C index, why): `Unit` in `params`; the hook binds each as `_`, never
    /// by a name (S0060). Empty outside hooks.
    pub opaque: Vec<(usize, NotCrossing)>,
    /// A `const char16_t *` result: `text?` to the script; a hook's `result` written as the text's UTF-16 kept copy, a
    /// call's read into a new text (lumart_text_from_game16).
    pub text16_result: bool,
}

impl FnTy {
    /// The C parameter (from 0) the script's argument `i` is: its place in the row's text, the counts the compiler
    /// fills counted.
    pub fn c_index(&self, i: usize) -> usize {
        let mut c = 0;
        let mut left = i;
        loop {
            if !self.counts.iter().any(|(n, _)| *n == c) {
                if left == 0 {
                    return c;
                }
                left -= 1;
            }
            c += 1;
        }
    }
}

/// Why a signature does not cross: the parameter (from 0; None for the result or the whole text) and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigNotCrossing {
    pub param: Option<usize>,
    pub why: NotCrossing,
}

/// A field or global in LumaScript types, with whether a script may write it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceTy {
    pub ty: GameTy,
    pub read_only: bool,
    /// A constant the game's module does not define (docs/modding/language/types.md, section 2): its value as the types file
    /// writes it. It binds nothing; the checker folds it, read only, and no import row names it.
    pub constant: Option<String>,
}

/// The game for one game version: its function rows and types (design section 2). Plain data; LumaLink reads it from
/// the kernel build's luma folder with the validator's reader ([`Game::from_names`]), the tests from fixtures.
#[derive(Clone, Debug, Default)]
pub struct Game {
    /// Each function's signatures with whether the row is a constructor or destructor (`tell`).
    pub functions: HashMap<String, Vec<(String, bool)>>,
    /// The types file; None when the folder has none, and then every record, enum and global is not known.
    pub types: Option<Types>,
    /// Each function row's facts (docs/modding/language/types.md); None when the folder has none, and then a record's
    /// function is read as the script calls it, said once per name (NOT_DESCRIBED).
    pub facts: Option<FunctionFacts>,
    /// The value records of [`VALUE_RECORDS`] and the matrix pointers of [`MATRIX_POINTERS`] the types file has with
    /// their layout, by row or typedef name.
    values: HashMap<String, Value>,
    /// Table names the types file has, but not with the table's layout: name, value, why. The checker reads them as
    /// game structs; the real-file test fails on any.
    pub value_mismatches: Vec<(String, Value, String)>,
    /// The services' faces read from the game's SDK headers (docs/modding/language/language.md); empty when LumaLink was given none,
    /// and then `use svc.<name>` is a note (not described) and its parts are not checked.
    pub services: crate::svc::Services,
    /// The object factory's rows (docs/modding/language/game-description.md, section 9, NameObjFactory's cCreateTable, read from the game's
    /// smg_game.luma-placement.json): each name and the class its creator makes, None where the facts do not say.
    /// None when LumaLink was given no placement facts, and then `made by` and `replaces` are the kernel's to refuse
    /// at define_class.
    pub factory: Option<Vec<(String, Option<String>)>>,
    /// Whether the SDK's luma.h gives LumaModInfo its `runtime_layout` field: the mod's record then carries
    /// lumi::RUNTIME_LAYOUT, so the loader refuses by name two mods built with different runtime layouts. A build
    /// against an SDK without it writes the record as before.
    pub mod_info_layout: bool,
    /// The tables of the game's data LumaCore.table and set_table take, by name (docs/modding/language/numbers.md;
    /// luma_numbers.json's `tables`). None when LumaLink was given none, and then `set_table` and `table` cannot be
    /// laid out (S0150).
    pub tables: Option<HashMap<String, GameTable>>,
}

/// A table of the game's data as luma_numbers.json lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameTable {
    /// Its element's type in the compiler's spelling: an array's innermost element, or the record itself.
    pub ty: String,
    /// Its elements (an array's innermost), 1 for a record.
    pub count: u64,
    /// All of it, in bytes.
    pub size: u64,
}

/// One step into a table's bytes as `set_table` and `table` name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableStep {
    /// A record's field, by name.
    Field(String),
    /// An array's element: `count` elements of `stride` bytes.
    Index { stride: u64, count: u64 },
}

/// What a game type is as a place `set_table` writes: the bytes, the script's type of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableLeaf {
    Scalar(Scalar),
    /// A game enum, stored as that integer.
    Enum(String, Scalar),
    Value(Value),
}

impl TableLeaf {
    /// Its size in the game's memory.
    pub fn size(&self) -> u64 {
        match self {
            TableLeaf::Scalar(s) | TableLeaf::Enum(_, s) => scalar_size(*s),
            TableLeaf::Value(v) => value_size(*v),
        }
    }
}

/// A number type's size in the game's memory (bool is one byte).
pub fn scalar_size(s: Scalar) -> u64 {
    match s {
        Scalar::Bool | Scalar::I8 | Scalar::U8 => 1,
        Scalar::I16 | Scalar::U16 => 2,
        Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
        Scalar::I64 | Scalar::U64 | Scalar::F64 => 8,
    }
}

/// A value type's size in the game's memory ([`Game::value_layout`]: a colour is four u8 channels).
pub fn value_size(v: Value) -> u64 {
    match v {
        Value::Vec2 => 8,
        Value::Vec3 => 12,
        Value::Vec4 | Value::Quat => 16,
        Value::Mat34 => 48,
        Value::Color => 4,
    }
}

/// Where a game type stands inside a table: a record, an array, or a place of bytes; Err why `set_table` cannot
/// reach into it or write it (a phrase id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TablePart {
    Record { name: String, size: u64 },
    Array { of: Box<TablePart>, extents: Vec<u64> },
    Leaf(TableLeaf),
}

impl TablePart {
    pub fn size(&self) -> u64 {
        match self {
            TablePart::Record { size, .. } => *size,
            TablePart::Array { of, extents } => of.size() * extents.iter().product::<u64>(),
            TablePart::Leaf(l) => l.size(),
        }
    }
}

/// How deep typedefs may name typedefs before a text is refused (a cycle in a broken file).
const MAX_TYPEDEF_DEPTH: u32 = 32;

/// What a name in a type text names.
enum Named {
    Void,
    Scalar(Scalar),
    Value(Value),
    MatrixPointer(Value),
    Record(String),
    Enum(String),
    Typedef(CType),
}

impl Game {
    /// The game the validator's [`Names`] read from a luma folder.
    pub fn from_names(names: &Names) -> Game {
        Game { facts: names.function_facts.clone(), ..Game::new(names.functions.clone(), names.types.clone()) }
    }

    /// The game of these rows and types; checks each table record's layout.
    pub fn new(functions: HashMap<String, Vec<(String, bool)>>, types: Option<Types>) -> Game {
        let mut game = Game { functions, types, ..Game::default() };
        let mut values = HashMap::new();
        let mut mismatches = Vec::new();
        if let Some(types) = &game.types {
            for (name, value) in VALUE_RECORDS {
                if let Some(record) = types.records.get(name) {
                    match game.value_layout(record, value) {
                        Ok(()) => {
                            values.insert(name.to_string(), value);
                        }
                        Err(why) => mismatches.push((name.to_string(), value, why)),
                    }
                }
            }
            for (name, value) in MATRIX_POINTERS {
                match types.typedefs.get(name).map(String::as_str) {
                    Some("f32 (*)[4]" | "float (*)[4]") => {
                        values.insert(name.to_string(), value);
                    }
                    Some(other) => mismatches.push((name.to_string(), value, format!("a typedef of \"{other}\", not of f32 (*)[4]"))),
                    None => {}
                }
            }
        }
        game.values = values;
        game.value_mismatches = mismatches;
        game
    }

    /// Whether `record` has the layout of `value`: its size, and each component a field of the element type at its
    /// place (own or reached in a base).
    fn value_layout(&self, record: &Record, value: Value) -> Result<(), String> {
        let (size, elem, names): (u64, Scalar, &[&str]) = match value {
            Value::Vec2 => (8, Scalar::F32, &["x", "y"]),
            Value::Vec3 => (12, Scalar::F32, &["x", "y", "z"]),
            Value::Vec4 | Value::Quat => (16, Scalar::F32, &["x", "y", "z", "w"]),
            Value::Mat34 => (48, Scalar::F32, &[]),
            Value::Color => (4, Scalar::U8, &["r", "g", "b", "a"]),
        };
        if record.size != size {
            return Err(format!("{} bytes, not {size}", record.size));
        }
        let scalar = |text: &str| matches!(ctype::parse_type(text).ok().map(|t| self.map(&t, Place::Field, 0)), Some(Ok(GameTy::Scalar(s))) if s == elem);
        if value == Value::Mat34 {
            // One field of twelve f32, as `f32 mMtx[3][4]` is written.
            return match record.own.as_slice() {
                [f] if f.offset == 0 && f.bits.is_none() && f.extents.iter().product::<u64>() == 12 && scalar(&f.ty) => Ok(()),
                _ => Err("not one field of 12 f32".to_string()),
            };
        }
        let width = if elem == Scalar::U8 { 1 } else { 4 };
        for (i, name) in names.iter().enumerate() {
            let at = i as u64 * width;
            let own = record.own.iter().find(|f| f.name == *name).filter(|f| f.extents.is_empty() && f.bits.is_none()).map(|f| (f.offset, f.ty.as_str()));
            let found = own.or_else(|| record.reached.iter().find(|f| f.name == *name).map(|f| (f.offset, f.ty.as_str())));
            match found {
                Some((offset, ty)) if offset == at && scalar(ty) => {}
                Some((offset, ty)) => return Err(format!("{name} is \"{ty}\" at {offset}")),
                None => return Err(format!("no field {name}")),
            }
        }
        Ok(())
    }

    fn types(&self) -> Option<&Types> {
        self.types.as_ref()
    }

    /// What a name in a type text names, through the types file's rows, the fixed-width names and the builtins.
    fn named(&self, name: &str) -> Result<Named, NotCrossing> {
        let builtin = match name {
            "void" => Some(Named::Void),
            "bool" => Some(Named::Scalar(Scalar::Bool)),
            "signed char" => Some(Named::Scalar(Scalar::I8)),
            "unsigned char" | "char8_t" => Some(Named::Scalar(Scalar::U8)),
            "short" => Some(Named::Scalar(Scalar::I16)),
            "unsigned short" | "char16_t" => Some(Named::Scalar(Scalar::U16)),
            "int" => Some(Named::Scalar(Scalar::I32)),
            "unsigned int" | "char32_t" => Some(Named::Scalar(Scalar::U32)),
            "long long" => Some(Named::Scalar(Scalar::I64)),
            "unsigned long long" => Some(Named::Scalar(Scalar::U64)),
            "float" => Some(Named::Scalar(Scalar::F32)),
            "double" => Some(Named::Scalar(Scalar::F64)),
            "char" => return Err(NotCrossing::CharSign),
            "long" | "unsigned long" | "wchar_t" | "long double" => return Err(NotCrossing::PlatformWidth(name.to_string())),
            _ => None,
        };
        if let Some(b) = builtin {
            return Ok(b);
        }
        if let Some((_, s)) = POINTER_WIDTH.iter().find(|(n, _)| *n == name) {
            return Ok(Named::Scalar(*s));
        }
        let Some(types) = self.types() else {
            return FIXED_WIDTH.iter().find(|(n, _)| *n == name).map(|(_, s)| Named::Scalar(*s)).ok_or_else(|| NotCrossing::NotKnown(name.to_string()));
        };
        if let Some(value) = self.values.get(name) {
            return Ok(match MATRIX_POINTERS.iter().any(|(n, _)| *n == name) {
                true => Named::MatrixPointer(*value),
                false => Named::Value(*value),
            });
        }
        if let Some((row, _)) = types.record(name) {
            return Ok(match self.values.get(row) {
                Some(value) => Named::Value(*value),
                None => Named::Record(row.to_string()),
            });
        }
        if types.enums.contains_key(name) {
            return Ok(Named::Enum(name.to_string()));
        }
        if let Some(of) = types.typedefs.get(name) {
            let of = ctype::parse_type(of).map_err(NotCrossing::Unreadable)?;
            // `typedef struct NebulaActor NebulaActor;` of a struct with no row names only itself: it is the
            // struct's left-out reason or not known, below.
            if !matches!(&of, CType::Named { name: n, .. } if n == name) {
                return Ok(Named::Typedef(of));
            }
        }
        if let Some((_, s)) = FIXED_WIDTH.iter().find(|(n, _)| *n == name) {
            return Ok(Named::Scalar(*s));
        }
        let template = name.split_once('<').map(|(t, _)| t);
        let left = [LeftOutKind::Record, LeftOutKind::Alias, LeftOutKind::Enum]
            .iter()
            .find_map(|k| types.left_out(*k, name))
            .or_else(|| template.and_then(|t| types.left_out(LeftOutKind::Template, t)));
        match left {
            Some(reason) => Err(NotCrossing::LeftOut { name: name.to_string(), reason: reason.to_string() }),
            None => Err(NotCrossing::NotKnown(name.to_string())),
        }
    }

    /// The kernel's gravity base (docs/modding/language/classes.md, "gravity"): a mod class may take PlanetGravity itself as its base
    /// (no NameObj at its start), with no `made by` or `replaces`; LumaCore.make_gravity alone makes its objects.
    pub fn gravity_base(&self, record: &str) -> bool {
        record == "PlanetGravity" && self.types.as_ref().and_then(|t| t.record(record)).is_some_and(|(_, r)| r.polymorphic)
    }

    /// A game class a mod's class may derive from (docs/modding/language/classes.md, `define_class`): polymorphic, with NameObj at
    /// its start (the class itself, or the base at offset 0, not virtual, down the chain).
    pub fn derivable(&self, record: &str) -> bool {
        let Some(types) = self.types.as_ref() else { return false };
        let Some((_, rec)) = types.record(record) else { return false };
        if !rec.polymorphic {
            return false;
        }
        let mut at = record.to_string();
        for _ in 0..64 {
            if at == "NameObj" {
                return true;
            }
            let Some((_, r)) = types.record(&at) else { return false };
            match r.bases.iter().find(|b| b.offset == 0 && !b.is_virtual) {
                Some(b) => at = b.name.clone(),
                None => return false,
            }
        }
        false
    }

    /// The factory row `name` (docs/modding/language/game-description.md, section 9): Some(the class it makes, when the facts say); None when the
    /// factory has no such row or there are no placement facts ([`Game::factory`]). Case is ignored, as the game's
    /// factory ignores it (NameObjFactory::getName2CreateFunc, MR::isEqualStringCase).
    pub fn factory_row(&self, name: &str) -> Option<Option<&str>> {
        self.factory.as_ref()?.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, class)| class.as_deref())
    }

    /// The virtual a mod's class overrides by `name` (docs/modding/language/language.md, section 5.5): of [`Game::virtuals_named`]'s signatures, the
    /// one, or the one with `arity` parameters when several are; else none.
    pub fn virtual_of(&self, base: &str, name: &str, arity: Option<usize>) -> Option<(String, String)> {
        let (full, sigs) = self.virtuals_named(base, name)?;
        match sigs.as_slice() {
            [one] => Some((full, one.clone())),
            many => {
                let fit: Vec<&String> = many.iter().filter(|sig| arity.is_some_and(|n| ctype::parse_signature(sig).is_ok_and(|s| s.params.len() == n))).collect();
                match fit.as_slice() {
                    [one] => Some((full, (*one).clone())),
                    _ => None,
                }
            }
        }
    }

    /// A member call on an object that runs the object's own class's function (docs/modding/language/objects.md): the row `name`
    /// (`signature`) is a member the facts mark virtual, and the types' `virtuals` say the kernel binds it (import kind
    /// 8, LumaCore.method). Some(the row's record: `this` is passed as it); None: a direct call (kind 0), also when
    /// the types carry no `virtuals` (kind 8 could not be bound).
    pub fn dispatched(&self, name: &str, signature: &str) -> Option<String> {
        let f = self.facts.as_ref()?.get(name, signature)?;
        if !f.is_virtual || !matches!(f.kind, FunctionKind::Method) {
            return None;
        }
        let methods = self.types.as_ref()?.methods.as_ref()?;
        if methods.get(&(name.to_string(), signature.to_string())) != Some(&true) {
            return None;
        }
        name.rsplit_once("::").map(|(record, _)| record.to_string())
    }

    /// Whether an object seen as record `from` is at the same address as its `to` part: `to` is `from`, or is reached
    /// from it through bases at offset 0 that are not virtual.
    pub fn same_address(&self, from: &str, to: &str) -> bool {
        let Some(types) = self.types.as_ref() else { return from == to };
        let mut at = from.to_string();
        for _ in 0..64 {
            if at == to {
                return true;
            }
            let Some((_, r)) = types.record(&at) else { return false };
            match r.bases.iter().find(|b| b.offset == 0 && !b.is_virtual) {
                Some(b) => at = b.name.clone(),
                None => return false,
            }
        }
        false
    }

    /// The virtual rows `<Record>::<name>` of the nearest record of `base`'s chain (itself, then its bases, breadth
    /// first) that has any: (row name, its virtual signatures, at least one). None when no record has, or the game
    /// has no function facts.
    pub fn virtuals_named(&self, base: &str, name: &str) -> Option<(String, Vec<String>)> {
        let facts = self.facts.as_ref()?;
        let types = self.types.as_ref();
        let mut queue = std::collections::VecDeque::from([base.to_string()]);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(r) = queue.pop_front() {
            if !seen.insert(r.clone()) {
                continue;
            }
            let full = format!("{r}::{name}");
            let mut rows: Vec<String> = self
                .functions
                .get(&full)
                .map(|rows| rows.iter().filter(|(sig, _)| facts.get(&full, sig).is_some_and(|f| f.is_virtual)).map(|(sig, _)| sig.clone()).collect())
                .unwrap_or_default();
            // A virtual the record declares with no function row (defined inline in its header, as
            // PlanetGravity::calcOwnGravityVector): its vtable slot is still the kernel's to wrap (docs/modding/language/objects.md, section 7a).
            if rows.is_empty() {
                if let Some(methods) = types.and_then(|t| t.methods.as_ref()) {
                    rows = methods.keys().filter(|(n, _)| *n == full).map(|(_, sig)| sig.clone()).collect();
                    rows.sort();
                }
            }
            if !rows.is_empty() {
                return Some((full, rows));
            }
            if let Some((_, rec)) = types.and_then(|t| t.record(&r)) {
                for b in &rec.bases {
                    queue.push_back(b.name.clone());
                }
            }
        }
        None
    }

    /// A game type a script writes by name (`LiveActor`, `Mario::MovementStates`, `MarioStatus`, `ref<Kuribo>`'s
    /// `Kuribo`): a class is a view (docs/modding/language/language.md, section 3, "game classes"), a record that is no class a game struct by value,
    /// an enum, or a number or value type through the game's typedefs (`s32`, `Vec3f`).
    pub fn type_by_name(&self, name: &str) -> Result<GameTy, NotCrossing> {
        match self.named(name)? {
            Named::Record(r) => {
                let class = self.types().and_then(|t| t.records.get(&r)).is_some_and(|rec| rec.polymorphic);
                if class {
                    return Ok(GameTy::View { record: r, konst: false });
                }
                // A record that cannot be the script's copy (its copy or drop is its own code, or it has none) is,
                // written as a type, its view, as a class is (decision (a): HitSensor); valid
                // where a view is, as hook parameters are.
                match self.by_value(&r) {
                    Ok(()) => Ok(GameTy::Struct(r)),
                    Err(NotCrossing::OwnsMemory(_) | NotCrossing::CannotDrop(_) | NotCrossing::CopiesItself(_) | NotCrossing::CannotCopy(_)) => {
                        Ok(GameTy::View { record: r, konst: false })
                    }
                    Err(why) => Err(why),
                }
            }
            Named::Void => Err(NotCrossing::VoidValue),
            Named::MatrixPointer(_) => Err(NotCrossing::PointerToValue),
            _ => self.map(&CType::Named { name: name.to_string(), konst: false, volatile: false, tag: None }, Place::Param, 0),
        }
    }

    /// Maps a type at `place` to LumaScript. `depth` counts typedefs followed.
    pub fn map(&self, t: &CType, place: Place, depth: u32) -> Result<GameTy, NotCrossing> {
        if depth > MAX_TYPEDEF_DEPTH {
            return Err(NotCrossing::Other(t.to_string()));
        }
        match t {
            CType::Named { name, konst, volatile, .. } => {
                if *volatile {
                    return Err(NotCrossing::Volatile);
                }
                match self.named(name)? {
                    Named::Void if place == Place::Result => Ok(GameTy::Unit),
                    Named::Void => Err(NotCrossing::VoidValue),
                    Named::Scalar(s) => Ok(GameTy::Scalar(s)),
                    // A field or global of a value type is itself the place ([`PlaceTy`]).
                    Named::Value(v) => Ok(GameTy::Value(v)),
                    Named::MatrixPointer(v) => self.pointer_to(GameTy::Value(v), false, place),
                    Named::Enum(e) => Ok(GameTy::Enum(e)),
                    Named::Record(r) => self.record(r, *konst, place),
                    Named::Typedef(of) => self.map(&with_const(of, *konst), place, depth + 1),
                }
            }
            CType::Ptr { to, .. } => self.pointer(to, place, depth),
            CType::Ref(to) | CType::RRef(to) => self.reference(to, depth),
            CType::Array { of, len } => self.array(of, *len, place, depth),
            CType::Func(_) => Err(NotCrossing::Function),
            CType::MemberPtr { .. } => Err(NotCrossing::MemberPointer),
        }
    }

    /// A record by value (or as a field or global, where it is a place).
    fn record(&self, r: String, konst: bool, place: Place) -> Result<GameTy, NotCrossing> {
        match place {
            Place::Field | Place::Global => Ok(GameTy::View { record: r, konst }),
            Place::Param | Place::HookParam | Place::Result => {
                self.by_value(&r)?;
                Ok(GameTy::Struct(r))
            }
        }
    }

    /// Whether a record may be held by value as the script's own copy (docs/modding/language/types.md):
    /// no vtable, trivially destructible (dropping a copy frees nothing), a copy can be called on it (`no_copy` false),
    /// and trivially copyable (a bit copy is its copy). A record the types file lacks is not known.
    pub fn by_value(&self, record: &str) -> Result<(), NotCrossing> {
        let rec = self.types().and_then(|t| t.records.get(record)).ok_or_else(|| NotCrossing::NotKnown(record.to_string()))?;
        if rec.polymorphic {
            Err(NotCrossing::ClassByValue(record.to_string()))
        } else if !rec.trivial_destroy && rec.trivial_copy {
            // Only a deleted destructor gives this pair: one the game wrote makes the copy not trivial too.
            Err(NotCrossing::CannotDrop(record.to_string()))
        } else if !rec.trivial_destroy {
            Err(NotCrossing::OwnsMemory(record.to_string()))
        } else if rec.no_copy {
            Err(NotCrossing::CannotCopy(record.to_string()))
        } else if !rec.trivial_copy {
            Err(NotCrossing::CopiesItself(record.to_string()))
        } else {
            Ok(())
        }
    }

    /// What `T *` is.
    fn pointer(&self, to: &CType, place: Place, depth: u32) -> Result<GameTy, NotCrossing> {
        if depth > MAX_TYPEDEF_DEPTH {
            return Err(NotCrossing::Other(to.to_string()));
        }
        match to {
            CType::Named { name, konst, volatile: false, .. } if name == "char" => {
                if *konst {
                    Ok(GameTy::Opt(Box::new(GameTy::Text)))
                } else {
                    Err(NotCrossing::Buffer)
                }
            }
            CType::Named { name, konst, volatile: false, .. } => match self.named(name)? {
                Named::Void => Err(NotCrossing::VoidPointer),
                Named::Record(r) => Ok(GameTy::Opt(Box::new(GameTy::View { record: r, konst: *konst }))),
                Named::Scalar(s) => self.pointer_to(GameTy::Scalar(s), *konst, place),
                Named::Value(v) => self.pointer_to(GameTy::Value(v), *konst, place),
                Named::Enum(e) => self.pointer_to(GameTy::Enum(e), *konst, place),
                Named::MatrixPointer(_) => Err(NotCrossing::PointerToPointer),
                // A pointer to a typedef: the typedef's type, its const kept.
                Named::Typedef(of) => self.pointer(&with_const(of, *konst), place, depth + 1),
            },
            CType::Named { volatile: true, .. } => Err(NotCrossing::Volatile),
            CType::Ptr { .. } => Err(NotCrossing::PointerToPointer),
            CType::Func(_) => Err(NotCrossing::Function),
            CType::MemberPtr { .. } => Err(NotCrossing::MemberPointer),
            CType::Array { of, len: Some(4) } if is_f32(self, of) => Err(NotCrossing::MatrixRows),
            // `const Mtx *`: one matrix or many.
            t @ CType::Array { .. } if is_matrix(self, t) => self.pointer_to(GameTy::Value(Value::Mat34), innermost_const(t), place),
            CType::Ref(_) | CType::RRef(_) | CType::Array { .. } => Err(NotCrossing::Other(CType::Ptr { to: Box::new(to.clone()), konst: false }.to_string())),
        }
    }

    /// A pointer to a number, value or enum: a result's or field's place, a parameter's [`NotCrossing::PointerToValue`].
    fn pointer_to(&self, of: GameTy, konst: bool, place: Place) -> Result<GameTy, NotCrossing> {
        match place {
            Place::Param => Err(NotCrossing::PointerToValue),
            Place::HookParam | Place::Result | Place::Field | Place::Global => Ok(GameTy::Opt(Box::new(GameTy::Place { of: Box::new(of), konst }))),
        }
    }

    /// What `T &` (and `T &&`, docs/modding/language/compiled-form.md) is: never null.
    fn reference(&self, to: &CType, depth: u32) -> Result<GameTy, NotCrossing> {
        if depth > MAX_TYPEDEF_DEPTH {
            return Err(NotCrossing::Other(to.to_string()));
        }
        match to {
            CType::Named { name, konst, volatile: false, .. } => match self.named(name)? {
                Named::Void => Err(NotCrossing::VoidValue),
                Named::Record(r) => Ok(GameTy::View { record: r, konst: *konst }),
                Named::Scalar(s) => Ok(GameTy::Place { of: Box::new(GameTy::Scalar(s)), konst: *konst }),
                Named::Value(v) => Ok(GameTy::Place { of: Box::new(GameTy::Value(v)), konst: *konst }),
                Named::Enum(e) => Ok(GameTy::Place { of: Box::new(GameTy::Enum(e)), konst: *konst }),
                // A reference to a pointer: the pointer is a place the game writes.
                Named::MatrixPointer(_) => Err(NotCrossing::PointerToPointer),
                Named::Typedef(of) => self.reference(&with_const(of, *konst), depth + 1),
            },
            CType::Named { volatile: true, .. } => Err(NotCrossing::Volatile),
            CType::Ptr { .. } => Err(NotCrossing::PointerToPointer),
            CType::Func(_) => Err(NotCrossing::Function),
            // `const Mtx &`.
            t @ CType::Array { .. } if is_matrix(self, t) => Ok(GameTy::Place { of: Box::new(GameTy::Value(Value::Mat34)), konst: innermost_const(t) }),
            other => Err(NotCrossing::Other(CType::Ref(Box::new(other.clone())).to_string())),
        }
    }

    /// An array type: `f32[3][4]` is `mat34` (the game's `Mtx`); a parameter's array is received as a pointer to its
    /// element (docs/modding/language/compiled-form.md), which crosses only as a matrix.
    fn array(&self, of: &CType, len: Option<u64>, place: Place, depth: u32) -> Result<GameTy, NotCrossing> {
        let whole = CType::Array { of: Box::new(of.clone()), len };
        if is_matrix(self, &whole) {
            return match place {
                // Received as a pointer to the rows: the game reads it, or writes it unless const.
                Place::Param | Place::HookParam => Ok(GameTy::Place { of: Box::new(GameTy::Value(Value::Mat34)), konst: innermost_const(of) }),
                Place::Field | Place::Global => Ok(GameTy::Value(Value::Mat34)),
                Place::Result => Err(NotCrossing::Other(whole.to_string())),
            };
        }
        match (place, len) {
            (Place::Field | Place::Global, Some(n)) => {
                let elem = self.map(of, place, depth)?;
                Ok(match elem {
                    GameTy::Array { of, mut extents } => {
                        extents.insert(0, n);
                        GameTy::Array { of, extents }
                    }
                    elem => GameTy::Array { of: Box::new(elem), extents: vec![n] },
                })
            }
            _ => Err(NotCrossing::Other(CType::Array { of: Box::new(of.clone()), len }.to_string())),
        }
    }

    /// A function row's signature (`void(const TVec3f &, s32)`).
    pub fn signature(&self, text: &str) -> Result<FnTy, SigNotCrossing> {
        self.signature_at(text, Place::Param)
    }

    /// A function row's signature as a script calls it, its parameters' marks applied (the function facts'
    /// `marks`; none without facts): a pointer marked `in` is a value the game reads, `out` and `inout` a place it
    /// writes (`var x`), with `nullable` also `none`; marked with a count or a fixed count, a `list<T>`, the count
    /// parameter filled by the compiler ([`FnTy::counts`]). `kept` is [`NotCrossing::Kept`]; `kept` or a `void *`
    /// with `nullable` is [`GameTy::Null`]; unmarked as [`Game::signature`].
    pub fn call_signature(&self, name: &str, text: &str) -> Result<FnTy, SigNotCrossing> {
        use luma_validate::names::Way;
        let marks = self.facts.as_ref().and_then(|f| f.get(name, text)).map(|f| f.marks.clone()).unwrap_or_default();
        if marks.iter().all(Option::is_none) {
            return self.signature(text);
        }
        let sig: Signature = ctype::parse_signature(text).map_err(|e| SigNotCrossing { param: None, why: NotCrossing::Unreadable(e) })?;
        if sig.variadic {
            return Err(SigNotCrossing { param: None, why: NotCrossing::Variadic });
        }
        let counts: Vec<(usize, usize)> = marks.iter().enumerate().filter_map(|(i, m)| m.and_then(|m| m.count).map(|c| (c, i))).collect();
        let mut params = Vec::with_capacity(sig.params.len());
        let mut transient = Vec::new();
        for (i, p) in sig.params.iter().enumerate() {
            let at = |why| SigNotCrossing { param: Some(i), why };
            if counts.iter().any(|(c, _)| *c == i) {
                // A count: an integer of one width everywhere, filled with the list's length.
                match self.map(p, Place::Param, 0).map_err(at)? {
                    GameTy::Scalar(Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::I64 | Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64) => continue,
                    _ => return Err(at(NotCrossing::MarkNotFit)),
                }
            }
            let ty = match marks.get(i).copied().flatten() {
                None => self.map(p, Place::Param, 0),
                // An address the game keeps, or memory of no known type, that the function accepts null for: the
                // script leaves it out (talk's balloon matrix; gravity's host, compared, never read).
                Some(m) if m.nullable && m.count.is_none() && m.fixed.is_none() && (m.way == Way::Kept || self.map(p, Place::Param, 0) == Err(NotCrossing::VoidPointer)) => {
                    Ok(GameTy::Null)
                }
                Some(m) if m.way == Way::Kept => Err(NotCrossing::Kept),
                // Text the game reads during the call only: still `text?` to the script (null for none, as unmarked).
                Some(m) if m.way == Way::In && m.count.is_none() && m.fixed.is_none() && self.is_text_param(p) => {
                    transient.push(i);
                    self.map(p, Place::Param, 0)
                }
                Some(m) => self.marked(p, m.way != Way::In, m.count.is_some() || m.fixed.is_some(), m.fixed, m.nullable),
            };
            params.push(ty.map_err(at)?);
        }
        let result = match &sig.result {
            Some(r) => self.map(r, Place::Result, 0).map_err(|why| SigNotCrossing { param: None, why })?,
            None => GameTy::Unit,
        };
        Ok(FnTy { params, result, konst: sig.konst, counts, transient, opaque: Vec::new(), text16_result: false })
    }

    /// A `const char *` parameter: `text?` to the script.
    fn is_text_param(&self, p: &CType) -> bool {
        matches!(self.map(p, Place::Param, 0), Ok(GameTy::Opt(ref t)) if **t == GameTy::Text)
    }

    /// A marked pointer parameter: the place or list it is to the script.
    fn marked(&self, p: &CType, writes: bool, list: bool, fixed: Option<u32>, nullable: bool) -> Result<GameTy, NotCrossing> {
        // An array parameter is a pointer to its first element (a matrix's rows are mapped as one).
        let decayed;
        let p = match p {
            CType::Array { of, .. } if !is_matrix(self, p) => {
                decayed = CType::Ptr { to: of.clone(), konst: false };
                &decayed
            }
            _ => p,
        };
        // As a hook receives it: the place it points at, its const.
        let (of, konst) = match self.map(p, Place::HookParam, 0)? {
            GameTy::Opt(inner) => match *inner {
                GameTy::Place { of, konst } => (of, konst),
                _ => return Err(NotCrossing::MarkNotFit),
            },
            GameTy::Place { of, konst } => (of, konst),
            _ => return Err(NotCrossing::MarkNotFit),
        };
        // A place of a number, value or enum only (a record pointer is a view, 's generator refuses a mark on it).
        if !matches!(*of, GameTy::Scalar(_) | GameTy::Value(_) | GameTy::Enum(_)) {
            return Err(NotCrossing::MarkNotFit);
        }
        if writes && konst {
            return Err(NotCrossing::MarkNotFit);
        }
        let t = if list { GameTy::List { of, konst: !writes, fixed } } else { GameTy::Place { of, konst: !writes } };
        Ok(if nullable { GameTy::Opt(Box::new(t)) } else { t })
    }

    /// A hooked function row's signature: its parameters as the hook receives them ([`Place::HookParam`]).
    pub fn hook_signature(&self, text: &str) -> Result<FnTy, SigNotCrossing> {
        self.signature_at(text, Place::HookParam)
    }

    fn signature_at(&self, text: &str, at: Place) -> Result<FnTy, SigNotCrossing> {
        let sig: Signature = ctype::parse_signature(text).map_err(|e| SigNotCrossing { param: None, why: NotCrossing::Unreadable(e) })?;
        if sig.variadic {
            return Err(SigNotCrossing { param: None, why: NotCrossing::Variadic });
        }
        let hook = at == Place::HookParam;
        let mut params = Vec::with_capacity(sig.params.len());
        let mut opaque = Vec::new();
        for (i, p) in sig.params.iter().enumerate() {
            match self.map(p, at, 0) {
                Ok(t) => params.push(t),
                // A hook passes a parameter it cannot read through untouched.
                Err(why) if hook => {
                    opaque.push((i, why));
                    params.push(GameTy::Unit);
                }
                Err(why) => return Err(SigNotCrossing { param: Some(i), why }),
            }
        }
        // A `const char16_t *` result is `text?`: a hook's written as the text's UTF-16 kept copy, a call's read as a
        // new text (its tags as bracket words, lumart_text_from_game16).
        let text16 = matches!(&sig.result, Some(CType::Ptr { to, .. }) if matches!(&**to, CType::Named { name, konst: true, .. } if name == "char16_t"));
        let result = match &sig.result {
            _ if text16 => GameTy::Opt(Box::new(GameTy::Text)),
            Some(r) => self.map(r, Place::Result, 0).map_err(|why| SigNotCrossing { param: None, why })?,
            None => GameTy::Unit,
        };
        Ok(FnTy { params, result, konst: sig.konst, counts: Vec::new(), transient: Vec::new(), opaque, text16_result: text16 })
    }

    /// A record's own field: a bit-field, an array (its `extents`), or a place of its type.
    pub fn field(&self, field: &Field) -> Result<PlaceTy, NotCrossing> {
        let (t, width) = ctype::parse_field_type(&field.ty).map_err(NotCrossing::Unreadable)?;
        let read_only = top_const(&t);
        if let Some(width) = width {
            if width != field.bits.map_or(0, |b| b.width) {
                return Err(NotCrossing::Other(field.ty.clone()));
            }
            let of = match self.map(&t, Place::Field, 0)? {
                GameTy::Scalar(s) if s != Scalar::F32 && s != Scalar::F64 => s,
                GameTy::Enum(e) => self.enum_scalar(&e).ok_or_else(|| NotCrossing::BitFieldType(field.ty.clone()))?,
                _ => return Err(NotCrossing::BitFieldType(field.ty.clone())),
            };
            return Ok(PlaceTy { ty: GameTy::Bits { of, width }, read_only, constant: None });
        }
        if field.bits.is_some() {
            return Err(NotCrossing::Other(field.ty.clone()));
        }
        let ty = self.map(&t, Place::Field, 0)?;
        if field.extents.is_empty() {
            return Ok(PlaceTy { read_only: read_only || matches!(ty, GameTy::Opt(ref t) if **t == GameTy::Text), ty, constant: None });
        }
        Ok(PlaceTy { ty: GameTy::Array { of: Box::new(ty), extents: field.extents.clone() }, read_only, constant: None })
    }

    /// A global by name: its place and whether a script may write it (docs/modding/language/compiled-form.md: a const global is read only).
    /// A constant the module does not define, with a value (`ChordInfo::OCTAVE`), is that value: a number or an
    /// enum, read only, folded ([`PlaceTy::constant`]).
    pub fn global(&self, name: &str) -> Result<PlaceTy, NotCrossing> {
        let types = self.types().ok_or_else(|| NotCrossing::NotKnown(name.to_string()))?;
        if let Some(global) = types.global_facts.get(name).filter(|g| !g.defined) {
            if let Some(value) = &global.value {
                let t = ctype::parse_type(&global.ty).map_err(NotCrossing::Unreadable)?;
                return match self.map(&t, Place::Global, 0)? {
                    ty @ (GameTy::Scalar(_) | GameTy::Enum(_)) => Ok(PlaceTy { ty, read_only: true, constant: Some(value.clone()) }),
                    _ => Err(NotCrossing::Other(global.ty.clone())),
                };
            }
        }
        let Some(global) = types.global_facts.get(name).filter(|g| g.defined) else {
            return Err(match types.left_out(LeftOutKind::Global, name) {
                Some(reason) => NotCrossing::LeftOut { name: name.to_string(), reason: reason.to_string() },
                None => NotCrossing::NotKnown(name.to_string()),
            });
        };
        let t = ctype::parse_type(&global.ty).map_err(NotCrossing::Unreadable)?;
        let ty = self.map(&t, Place::Global, 0)?;
        Ok(PlaceTy { read_only: global.konst || top_const(&t) || matches!(ty, GameTy::Opt(ref t) if **t == GameTy::Text), ty, constant: None })
    }

    /// A table's element as a part of the table's bytes: the record it names (any record, plain data or not: the
    /// bytes are written as they lie), or a place of the type text's kind.
    pub fn table_element(&self, ty: &str) -> Result<TablePart, &'static str> {
        if let Some((name, record)) = self.types().and_then(|t| t.record(ty)) {
            if let Some(v) = self.values.get(name) {
                return Ok(TablePart::Leaf(TableLeaf::Value(*v)));
            }
            return Ok(TablePart::Record { name: name.to_string(), size: record.size });
        }
        let t = ctype::parse_type(ty).map_err(|_| "Script.Table.Type")?;
        self.table_part(&t)
    }

    /// A record's field (own, or reached in a base) as a part of the table's bytes, with its offset in the record;
    /// None: no such field.
    pub fn table_field(&self, record: &str, field: &str) -> Option<(u64, Result<TablePart, &'static str>)> {
        let (_, r) = self.types()?.record(record)?;
        if let Some(f) = r.own.iter().find(|f| f.name == field) {
            if f.bits.is_some() {
                return Some((f.offset, Err("Script.Table.BitField")));
            }
            let part = ctype::parse_field_type(&f.ty).map_err(|_| "Script.Table.Type").and_then(|(t, width)| match width {
                Some(_) => Err("Script.Table.BitField"),
                None => self.table_part(&t),
            });
            let part = match part {
                Ok(p) if !f.extents.is_empty() => Ok(TablePart::Array { of: Box::new(p), extents: f.extents.clone() }),
                other => other,
            };
            return Some((f.offset, part));
        }
        let reached = r.reached.iter().find(|f| f.name == field)?;
        let part = ctype::parse_field_type(&reached.ty).map_err(|_| "Script.Table.Type").and_then(|(t, width)| match width {
            Some(_) => Err("Script.Table.BitField"),
            None => self.table_part(&t),
        });
        Some((reached.offset, part))
    }

    /// A type text's part of a table: a record, an array of a part, or a number, enum or value; a pointer and anything
    /// else is no place `set_table` writes.
    fn table_part(&self, t: &CType) -> Result<TablePart, &'static str> {
        match t {
            CType::Array { of, len: Some(n) } if !is_matrix(self, t) => {
                let inner = self.table_part(of)?;
                Ok(match inner {
                    TablePart::Array { of, mut extents } => {
                        extents.insert(0, *n);
                        TablePart::Array { of, extents }
                    }
                    other => TablePart::Array { of: Box::new(other), extents: vec![*n] },
                })
            }
            CType::Ptr { .. } | CType::MemberPtr { .. } | CType::Ref(_) | CType::RRef(_) | CType::Func(_) => Err("Script.Table.Pointer"),
            CType::Named { name, .. } => {
                if let Some((row, record)) = self.types().and_then(|ty| ty.record(name)) {
                    if let Some(v) = self.values.get(row) {
                        return Ok(TablePart::Leaf(TableLeaf::Value(*v)));
                    }
                    return Ok(TablePart::Record { name: row.to_string(), size: record.size });
                }
                match self.map(t, Place::Field, 0) {
                    Ok(GameTy::Scalar(s)) => Ok(TablePart::Leaf(TableLeaf::Scalar(s))),
                    Ok(GameTy::Enum(e)) => match self.enum_scalar(&e) {
                        Some(s) => Ok(TablePart::Leaf(TableLeaf::Enum(e, s))),
                        None => Err("Script.Table.Type"),
                    },
                    Ok(GameTy::Value(v)) => Ok(TablePart::Leaf(TableLeaf::Value(v))),
                    Ok(GameTy::Text | GameTy::Opt(_)) => Err("Script.Table.Pointer"),
                    _ => Err("Script.Table.Type"),
                }
            }
            _ => match self.map(t, Place::Field, 0) {
                Ok(GameTy::Value(v)) => Ok(TablePart::Leaf(TableLeaf::Value(v))),
                _ => Err("Script.Table.Type"),
            },
        }
    }

    /// The integer an enum is stored as.
    fn enum_scalar(&self, name: &str) -> Option<Scalar> {
        let e = self.types()?.enums.get(name)?;
        match self.map(&ctype::parse_type(&e.underlying).ok()?, Place::Field, 0) {
            Ok(GameTy::Scalar(s)) if s != Scalar::F32 && s != Scalar::F64 && s != Scalar::Bool => Some(s),
            _ => None,
        }
    }
}

/// `t` with `const` added where a typedef's use wrote it (`const Mtx`: the elements; `const QuaternionPtr`: the
/// pointer, which changes nothing a script sees).
fn with_const(t: CType, konst: bool) -> CType {
    if !konst {
        return t;
    }
    match t {
        CType::Named { name, volatile, tag, .. } => CType::Named { name, konst: true, volatile, tag },
        CType::Array { of, len } => CType::Array { of: Box::new(with_const(*of, true)), len },
        CType::Ptr { to, .. } => CType::Ptr { to, konst: true },
        CType::MemberPtr { class, to, .. } => CType::MemberPtr { class, to, konst: true },
        other => other,
    }
}

/// Whether a field's or global's own storage is const: a const name, a const pointer, an array of const.
fn top_const(t: &CType) -> bool {
    match t {
        CType::Named { konst, .. } | CType::Ptr { konst, .. } | CType::MemberPtr { konst, .. } => *konst,
        CType::Array { of, .. } => top_const(of),
        // Assigning through a reference writes what it names: the referent's const says ([`GameTy::Place`]).
        CType::Ref(_) | CType::RRef(_) => false,
        CType::Func(_) => true,
    }
}

fn innermost_const(t: &CType) -> bool {
    match t {
        CType::Array { of, .. } => innermost_const(of),
        CType::Named { konst, .. } => *konst,
        _ => false,
    }
}

/// `f32[3][4]`, the game's `Mtx`: LumaScript's `mat34`.
fn is_matrix(game: &Game, t: &CType) -> bool {
    matches!(t, CType::Array { of, len: Some(3) } if matches!(&**of, CType::Array { of: row, len: Some(4) } if is_f32(game, row)))
}

fn is_f32(game: &Game, t: &CType) -> bool {
    matches!(t, CType::Named { volatile: false, .. }) && matches!(game.map(t, Place::Field, 0), Ok(GameTy::Scalar(Scalar::F32)))
}
