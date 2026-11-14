//! Expressions: literals that take their place's type (docs/modding/language/language.md, section 3), strings and their escapes,
//! names in the lookup order, members of every kind of value, operators (3), `as`, `if` and `match` as
//! values (4), closures (5.4), `wait` (11.1) and `ref` (6.2).

use super::call::STD_NAMES;
use super::pat::Pat;
use super::{LocalId, Members, Resolved, Walk, Want};
use crate::ast::{self, BinOp, Block, Expr, ExprKind, Pos, StrPart, StrPrefix};
use crate::check::{ItemRef, ModRef, ModTarget, Name, Ty};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::{GameTy, NotCrossing, PlaceTy, Scalar, Value};

/// What a bare name names, found without saying anything ('s order).
#[derive(Clone, Debug)]
pub(super) enum NameKind {
    Local(LocalId, bool),
    Item(ItemRef),
    Module(ModTarget),
    /// A member of the class or game record the body is in, reached without `self.`.
    Member(Members),
    Std(&'static str),
    /// One of the language's type names (`u32`, for `u32.max`).
    Type(&'static str),
    Game,
    Unknown,
}

/// A number as written.
enum Num {
    Int(i128),
    Float(f64),
    /// Past i128: no type takes it.
    Huge,
}

/// The element of a game array with these extents (outermost first): another array, or the element's type.
pub(super) fn array_element(of: &GameTy, extents: &[u64]) -> Ty {
    if extents.len() > 1 {
        Ty::Game(GameTy::Array { of: Box::new(of.clone()), extents: extents[1..].to_vec() })
    } else {
        from_game_place(of)
    }
}

/// A game type where a value is read: numbers and vectors as LumaScript's, a place's referent, a pointer's `?`.
/// `-N` with N a number literal that is not 0 (through `( )`).
fn negative_literal(e: &Expr) -> bool {
    let ExprKind::Neg(x) = &e.kind else { return false };
    let mut x: &Expr = x;
    while let ExprKind::Paren(inner) = &x.kind {
        x = inner;
    }
    let ExprKind::Number(text) = &x.kind else { return false };
    let lower = text.to_ascii_lowercase();
    match lower.strip_prefix("0x").or_else(|| lower.strip_prefix("0b")).or_else(|| lower.strip_prefix("0o")) {
        Some(digits) => digits.chars().take_while(|c| c.is_ascii_hexdigit() || *c == '_').any(|c| c.is_ascii_hexdigit() && c != '0'),
        None => lower.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_').any(|c| ('1'..='9').contains(&c)),
    }
}

/// A number literal's text, through `( )`.
fn number_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Paren(x) => number_text(x),
        ExprKind::Number(t) => t.clone(),
        _ => String::new(),
    }
}

pub(super) fn from_game_place(g: &GameTy) -> Ty {
    match g {
        GameTy::Unit => Ty::Unit,
        GameTy::Scalar(_) | GameTy::Value(_) => Ty::from_game(g.clone()),
        GameTy::Text => Ty::Text,
        GameTy::Opt(of) => Ty::Opt(Box::new(from_game_place(of))),
        GameTy::Place { of, .. } => from_game_place(of),
        GameTy::Bits { of: Scalar::Bool, .. } => Ty::Bool,
        GameTy::Bits { of, .. } => Ty::Int(*of),
        other => Ty::Game(other.clone()),
    }
}

/// The type a literal on one side takes from the other side's type: the same number type; `f32` against a
/// vector or colour that is scaled.
fn literal_want(op: BinOp, other: &Ty) -> Option<Ty> {
    match other {
        Ty::Int(_) | Ty::Float(_) => Some(other.clone()),
        Ty::Value(_) if matches!(op, BinOp::Mul | BinOp::Div) => Some(Ty::Float(Scalar::F32)),
        _ => None,
    }
}

/// What the right side of `x op= value` is checked against, when the value is a literal.
pub(super) fn rhs_want(op: BinOp, lhs: &Ty) -> Option<Ty> {
    match (op, lhs) {
        (BinOp::Add | BinOp::Sub, Ty::Value(_)) => Some(lhs.clone()),
        (_, other) => literal_want(op, other),
    }
}

/// A literal-shaped part, which takes its place's type: its kind before the place decides (overloads, `min`, lists).
pub(super) fn shape(e: &Expr) -> Option<Ty> {
    match &e.kind {
        ExprKind::Number(t) => number_shape(t),
        ExprKind::Neg(inner) => match &inner.kind {
            ExprKind::Number(t) => number_shape(t),
            _ => None,
        },
        ExprKind::Paren(inner) => shape(inner),
        ExprKind::Binary { op, left, right } => literal_binary(*op, left, right),
        ExprKind::None => Some(Ty::Open),
        ExprKind::List(items) if items.is_empty() => Some(Ty::Open),
        ExprKind::Braces(entries) if entries.is_empty() => Some(Ty::Open),
        _ => None,
    }
}

/// `l op r` over literals only (`1 << 4`, `1 + 2 * 3`): an integer literal, or a float one when a side is a float;
/// None when an operand is no literal or the operator takes no such literal (`&` of floats, a comparison).
fn literal_binary(op: BinOp, l: &Expr, r: &Expr) -> Option<Ty> {
    let (a, b) = (shape(l)?, shape(r)?);
    let float = a == Ty::FloatLit || b == Ty::FloatLit;
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem if matches!(a, Ty::IntLit | Ty::FloatLit) && matches!(b, Ty::IntLit | Ty::FloatLit) => {
            Some(if float { Ty::FloatLit } else { Ty::IntLit })
        }
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr if a == Ty::IntLit && b == Ty::IntLit => Some(Ty::IntLit),
        _ => None,
    }
}

/// The value of an integer expression of literals, when it has one here (no suffix, no division by zero, a shift
/// within i128): what a place's fit is checked against.
pub(super) fn fold_int(e: &Expr) -> Option<i128> {
    match &e.kind {
        ExprKind::Number(t) => match parse_number(t) {
            (Num::Int(v), None) => Some(v),
            _ => None,
        },
        ExprKind::Neg(inner) => fold_int(inner)?.checked_neg(),
        ExprKind::Paren(inner) => fold_int(inner),
        ExprKind::Binary { op, left, right } => {
            let (a, b) = (fold_int(left)?, fold_int(right)?);
            match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                BinOp::Mul => a.checked_mul(b),
                BinOp::Div => a.checked_div(b),
                BinOp::Rem => a.checked_rem(b),
                BinOp::BitAnd => Some(a & b),
                BinOp::BitOr => Some(a | b),
                BinOp::BitXor => Some(a ^ b),
                BinOp::Shl => u32::try_from(b).ok().filter(|n| *n < 127).and_then(|n| a.checked_mul(1i128 << n)),
                BinOp::Shr => u32::try_from(b).ok().filter(|n| *n < 127).map(|n| a >> n),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The number type a place takes (`T?`'s `T`), if it is one.
fn number_place(want: Want<'_>) -> Option<Ty> {
    match want {
        Want::Ty(Ty::Opt(inner)) => number_place(Want::Ty(inner)),
        Want::Ty(t @ (Ty::Int(_) | Ty::Float(_))) => Some(t.clone()),
        _ => None,
    }
}

fn number_shape(text: &str) -> Option<Ty> {
    match parse_number(text) {
        (_, Some(_)) => None,
        (Num::Float(_), None) => Some(Ty::FloatLit),
        _ => Some(Ty::IntLit),
    }
}

const SUFFIXES: [(&str, Scalar); 10] = [
    ("f32", Scalar::F32),
    ("f64", Scalar::F64),
    ("i8", Scalar::I8),
    ("i16", Scalar::I16),
    ("i32", Scalar::I32),
    ("i64", Scalar::I64),
    ("u8", Scalar::U8),
    ("u16", Scalar::U16),
    ("u32", Scalar::U32),
    ("u64", Scalar::U64),
];

/// The value of a whole-number literal as written (`3`, `-1`, `(2)`); None for anything else.
pub(super) fn literal_whole(e: &Expr) -> Option<i128> {
    match &e.kind {
        ExprKind::Number(t) => match parse_number(t).0 {
            Num::Int(n) => Some(n),
            _ => None,
        },
        ExprKind::Neg(inner) => literal_whole(inner).map(|n| -n),
        ExprKind::Paren(inner) => literal_whole(inner),
        _ => None,
    }
}

/// A number's value and its suffix's type (G.1: `0x` and `0b` numbers take no suffix).
fn parse_number(text: &str) -> (Num, Option<Scalar>) {
    let digits: String = text.chars().filter(|c| *c != '_').collect();
    for (lead, radix) in [("0x", 16), ("0b", 2)] {
        if let Some(rest) = digits.strip_prefix(lead) {
            return (i128::from_str_radix(rest, radix).map_or(Num::Huge, Num::Int), None);
        }
    }
    let (body, suffix) = match SUFFIXES.iter().find(|(s, _)| digits.ends_with(s)) {
        Some((s, scalar)) => (&digits[..digits.len() - s.len()], Some(*scalar)),
        None => (digits.as_str(), None),
    };
    if body.contains(['.', 'e', 'E']) {
        return (body.parse::<f64>().map_or(Num::Huge, Num::Float), suffix);
    }
    (body.parse::<i128>().map_or(Num::Huge, Num::Int), suffix)
}

/// The range of an integer type.
fn int_range(s: Scalar) -> (i128, i128) {
    match s {
        Scalar::I8 => (i8::MIN.into(), i8::MAX.into()),
        Scalar::I16 => (i16::MIN.into(), i16::MAX.into()),
        Scalar::I32 => (i32::MIN.into(), i32::MAX.into()),
        Scalar::I64 => (i64::MIN.into(), i64::MAX.into()),
        Scalar::U8 => (0, u8::MAX.into()),
        Scalar::U16 => (0, u16::MAX.into()),
        Scalar::U32 => (0, u32::MAX.into()),
        Scalar::U64 => (0, u64::MAX.into()),
        Scalar::Bool | Scalar::F32 | Scalar::F64 => (0, 0),
    }
}

fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Coalesce => "??",
        BinOp::Or => "or",
        BinOp::And => "and",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Range => "..",
        BinOp::RangeIncl => "..=",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::BitAnd => "&",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
    }
}

/// Whether an escape `\x` (the text right after the backslash) is one of these: `\n \t \r \0 \\ \" \' \{ \}
/// \xHH \u{...}`. Gives the escape as written when it is not.
fn bad_escape(text: &str, bytes: bool) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' {
            i += 1;
            continue;
        }
        let Some(&next) = chars.get(i + 1) else { return Some("\\".into()) };
        match next {
            'n' | 't' | 'r' | '0' | '\\' | '"' | '\'' | '{' | '}' => i += 2,
            'x' => {
                let hex: String = chars.iter().skip(i + 2).take(2).collect();
                let ok = hex.len() == 2 && u8::from_str_radix(&hex, 16).is_ok_and(|b| bytes || b < 0x80);
                if !ok {
                    return Some(format!("\\x{hex}"));
                }
                i += 4;
            }
            'u' => {
                let rest: String = chars.iter().skip(i + 2).collect();
                let close = rest.find('}');
                let ok = rest.starts_with('{')
                    && close.is_some_and(|c| (2..=7).contains(&c) && u32::from_str_radix(&rest[1..c], 16).ok().and_then(char::from_u32).is_some());
                if !ok {
                    let shown: String = rest.chars().take_while(|c| *c != '}').take(9).collect();
                    return Some(format!("\\u{shown}{}", if close.is_some() { "}" } else { "" }));
                }
                i += 2 + rest[..=close.unwrap_or(0)].chars().count();
            }
            other => return Some(format!("\\{other}")),
        }
    }
    None
}

impl<'w, 'a> Walk<'w, 'a> {
    /// An expression with the type its place wants; its own type is recorded and given (`Ty::Error` once said
    /// wrong).
    pub(super) fn expr(&mut self, e: &'a Expr, want: Want<'_>) -> Ty {
        let found = self.expr_kind(e, want);
        let found = match (&found, want) {
            (Ty::Unit, Want::Any) => {
                self.say(Diagnostic::new(code::NO_VALUE, e.at));
                Ty::Error
            }
            (Ty::Unit, Want::Ty(t)) if !matches!(t, Ty::Unit | Ty::Error) => {
                self.say(Diagnostic::new(code::NO_VALUE, e.at));
                Ty::Error
            }
            _ => found,
        };
        let found = match want {
            Want::Ty(t) => self.check_fit(e.at, found, t),
            _ => found,
        };
        self.tys.insert(e.id, found.clone());
        found
    }

    /// A value of `found` where `want` goes: MAY_BE_NONE for a `T?` where a `T` goes, COPY_NOT_VIEW for
    /// the script's own copy where the game's object goes (revision 3), else TYPE_MISMATCH.
    pub(super) fn check_fit(&mut self, at: Pos, found: Ty, want: &Ty) -> Ty {
        if self.fits(&found, want) {
            return found;
        }
        match (&found, want) {
            (Ty::Opt(inner), _) if self.fits(inner, want) => {
                let t = self.ty_text(inner);
                self.say(Diagnostic::new(code::MAY_BE_NONE, at).with("found", Arg::Text(t)));
            }
            (Ty::Game(GameTy::Struct(r)), Ty::Game(GameTy::View { record, .. })) if self.derives(r, record) => {
                self.say(Diagnostic::new(code::COPY_NOT_VIEW, at).with("found", Arg::Text(r.clone())));
            }
            _ => {
                let (f, w) = (self.ty_text(&found), self.ty_text(want));
                self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(f)).with("expected", Arg::Text(w)));
            }
        }
        Ty::Error
    }

    /// Whether a value of `found` may stand where `want` goes.
    pub(super) fn fits(&self, found: &Ty, want: &Ty) -> bool {
        if found == want {
            return true;
        }
        match (found, want) {
            (Ty::Error | Ty::Never, _) | (_, Ty::Error) => true,
            (Ty::IntLit, Ty::Int(_) | Ty::Float(_)) | (Ty::FloatLit, Ty::Float(_)) => true,
            (Ty::Open, Ty::Opt(_) | Ty::List(_) | Ty::Map(..) | Ty::Set(_) | Ty::Json) => true,
            (Ty::IntLit | Ty::FloatLit, Ty::Json) => true,
            (Ty::Opt(f), Ty::Opt(w)) => self.fits(f, w),
            (f, Ty::Opt(w)) => self.fits(f, w),
            (Ty::Game(GameTy::View { record: a, konst: ka }), Ty::Game(GameTy::View { record: b, konst: kb })) => (!*ka || *kb) && self.derives(a, b),
            // A view of a plain struct where the script's own copy goes: a copy is taken.
            (Ty::Game(GameTy::View { record: a, .. }), Ty::Game(GameTy::Struct(b))) => a == b,
            (Ty::Class(i), Ty::Game(GameTy::View { record, .. })) => self.class_base(*i).is_some_and(|b| self.derives(&b, record)),
            // `nerve "Class.Step"` is the game's `const Nerve *` (language.md).
            (Ty::Content(w), Ty::Game(GameTy::View { record, .. })) => w == "nerve" && record == "Nerve",
            (Ty::Ref(a), Ty::Ref(b)) => self.fits(a, b),
            (Ty::List(a), Ty::List(b)) | (Ty::Set(a), Ty::Set(b)) => matches!(**a, Ty::Error | Ty::Never) || a == b,
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => (matches!(**ka, Ty::Error) || ka == kb) && (matches!(**va, Ty::Error) || va == vb),
            (Ty::Tuple(a), Ty::Tuple(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| self.fits(x, y)),
            (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => pa == pb && (ra == rb || matches!(**ra, Ty::Error | Ty::Never)),
            _ => false,
        }
    }

    /// Whether a game record is `base` or derives from it, through the types file's bases.
    pub(super) fn derives(&self, record: &str, base: &str) -> bool {
        if record == base {
            return true;
        }
        let Some(types) = self.game().types.as_ref() else { return false };
        let mut seen = std::collections::BTreeSet::new();
        let mut todo = vec![record.to_string()];
        while let Some(r) = todo.pop() {
            if !seen.insert(r.clone()) {
                continue;
            }
            let Some((_, rec)) = types.record(&r) else { continue };
            for b in &rec.bases {
                let row = types.record(&b.name).map_or(b.name.as_str(), |(row, _)| row);
                if row == base {
                    return true;
                }
                todo.push(row.to_string());
            }
        }
        false
    }

    /// The game record a mod class derives from, by its row name.
    pub(super) fn class_base(&self, class: ItemRef) -> Option<String> {
        let ast::ItemKind::Class(cd) = &self.item_of(class).kind else { return None };
        let types = self.game().types.as_ref()?;
        types.record(&cd.base.text).map(|(row, _)| row.to_string())
    }

    fn expr_kind(&mut self, e: &'a Expr, want: Want<'_>) -> Ty {
        match &e.kind {
            ExprKind::Number(text) => self.number(e.at, text, false, want),
            ExprKind::Neg(inner) if matches!(inner.kind, ExprKind::Number(_)) => {
                let ExprKind::Number(text) = &inner.kind else { unreachable!("matched above") };
                let t = self.number(inner.at, text, true, want);
                self.tys.insert(inner.id, t.clone());
                t
            }
            ExprKind::Color(_) => match want {
                Want::Ty(Ty::Json) => Ty::Json,
                _ => Ty::Value(Value::Color),
            },
            ExprKind::Str(s) => self.string(s, want),
            ExprKind::True | ExprKind::False => match want {
                Want::Ty(Ty::Json) => Ty::Json,
                _ => Ty::Bool,
            },
            ExprKind::None => match want {
                Want::Ty(t @ Ty::Opt(_)) => t.clone(),
                Want::Ty(Ty::Json) => Ty::Json,
                Want::Ty(Ty::Error) => Ty::Error,
                Want::Ty(t) => {
                    let expected = self.ty_text(t);
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, e.at).with("found", Arg::Text("none".into())).with("expected", Arg::Text(expected)));
                    Ty::Error
                }
                Want::Any | Want::Drop => {
                    self.say(Diagnostic::new(code::TYPE_UNKNOWN_HERE, e.at).with("found", Arg::Text("none".into())));
                    Ty::Error
                }
            },
            ExprKind::SelfValue => match self.frames.iter().rev().find_map(|f| f.self_ty.clone()) {
                // A constructor's `before` or a destructor's `after`: the object is not there (S0128).
                Some(_) if self.frames.iter().rev().find(|f| f.self_ty.is_some()).and_then(|f| f.self_gone).is_some() => {
                    let why = self.frames.iter().rev().find(|f| f.self_ty.is_some()).and_then(|f| f.self_gone).unwrap_or("Script.Self.NotYet");
                    self.say(Diagnostic::new(code::SELF_NOT_BUILT, e.at).with("why", Arg::Phrase(why)));
                    Ty::Error
                }
                Some(t) => {
                    self.self_read(e.at);
                    t
                }
                None => {
                    self.say(Diagnostic::new(code::SELF_OUTSIDE, e.at));
                    Ty::Error
                }
            },
            ExprKind::Result => self.result_read(e.at),
            ExprKind::Proceed => self.proceed_here(e.at),
            ExprKind::Ref(args) => self.ref_expr(e, args),
            ExprKind::If { arms, otherwise } => self.if_expr(e.at, arms, otherwise.as_ref(), want),
            ExprKind::Match { subject, arms } => self.match_expr(e.at, subject, arms, want),
            ExprKind::Closure { params, ret, body } => {
                let m = self.m;
                let tys: Vec<Ty> = params.iter().map(|p| p.ty.as_ref().map_or(Ty::Error, |t| self.c.resolve(m, t))).collect();
                let r = ret.as_ref().map(|t| self.c.resolve(m, t));
                self.closure_body(params, &tys, r.clone(), body);
                Ty::Fn(tys, Box::new(r.unwrap_or(Ty::Unit)))
            }
            ExprKind::Paren(inner) => self.expr(inner, want),
            ExprKind::Tuple(items) => match want {
                Want::Ty(Ty::Tuple(ts)) if ts.len() == items.len() => {
                    let ts = ts.clone();
                    Ty::Tuple(items.iter().zip(ts.iter()).map(|(i, t)| self.expr(i, Want::Ty(t))).collect())
                }
                _ => Ty::Tuple(items.iter().map(|i| self.expr(i, Want::Any)).collect()),
            },
            ExprKind::List(items) => self.list_lit(e.at, items, want),
            ExprKind::Braces(entries) => self.braces_lit(e.at, entries, want),
            ExprKind::Content { word, path, args } => self.content(e.at, word, path, args.as_deref()),
            ExprKind::Game(name) => self.game_value(e, name),
            ExprKind::Name { name, .. } => self.name_value(e, name),
            ExprKind::StructLit { path, fields } => self.struct_lit(e.at, path, fields),
            ExprKind::Call { callee, args } => self.call(e, callee, args, want),
            ExprKind::Index { base, index } => self.index(e, base, index),
            ExprKind::Member { base, name, optional, .. } => self.member_value(e, base, name, *optional),
            ExprKind::Unwrap(inner) => {
                let t = self.expr(inner, Want::Any);
                match t {
                    Ty::Opt(x) => {
                        if self.game_places.contains(&inner.id) {
                            self.game_places.insert(e.id);
                        }
                        *x
                    }
                    Ty::Error | Ty::Never => Ty::Error,
                    other => {
                        let found = self.ty_text(&other);
                        self.say(Diagnostic::new(code::UNWRAP_NOT_OPTIONAL, e.at).with("found", Arg::Text(found)));
                        other
                    }
                }
            }
            ExprKind::Neg(inner) => {
                let t = self.expr(inner, Want::Any);
                match &t {
                    Ty::Int(s) if !matches!(s, Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64) => t,
                    Ty::Float(_) | Ty::Value(Value::Vec2 | Value::Vec3 | Value::Vec4) | Ty::Error | Ty::Never => t,
                    other => {
                        let found = self.ty_text(other);
                        self.say(Diagnostic::new(code::OPERATOR_ONE, e.at).with("op", Arg::Text("-".into())).with("found", Arg::Text(found)));
                        Ty::Error
                    }
                }
            }
            ExprKind::Not(inner) => {
                self.expr(inner, Want::Ty(&Ty::Bool));
                Ty::Bool
            }
            ExprKind::Binary { op, left, right } => self.binary(e, *op, left, right, want),
            // The amount is read once, before the wait; `until`'s condition on each frame after it starts.
            ExprKind::Wait { amount, unit } => {
                // a literal below 0 (S0110); a computed one waits 0 (S0902 at run time).
                if unit.is_some() && negative_literal(amount) {
                    let written = format!("-{}", match &amount.kind {
                        ExprKind::Neg(x) => number_text(x),
                        _ => String::new(),
                    });
                    self.say(Diagnostic::new(code::WAIT_NEGATIVE, amount.at).with("found", Arg::Text(written)));
                }
                let t = match unit {
                    Some(ast::WaitUnit::Frames) => {
                        self.expr(amount, Want::Ty(&Ty::Int(Scalar::I32)));
                        Ty::Unit
                    }
                    Some(ast::WaitUnit::Seconds) => {
                        self.expr(amount, Want::Ty(&Ty::Float(Scalar::F32)));
                        Ty::Unit
                    }
                    None => match self.expr(amount, Want::Any) {
                        Ty::Task => Ty::Unit,
                        Ty::Pending(of) => *of,
                        Ty::Error | Ty::Never => Ty::Error,
                        other => {
                            let found = self.ty_text(&other);
                            self.say(Diagnostic::new(code::WAIT_WHAT, amount.at).with("found", Arg::Text(found)));
                            Ty::Error
                        }
                    },
                };
                self.wait_here(e);
                t
            }
            ExprKind::WaitUntil(cond) => {
                self.wait_here(e);
                self.expr(cond, Want::Ty(&Ty::Bool));
                Ty::Unit
            }
            ExprKind::Start(inner) => self.start_task(inner),
            ExprKind::As { value, ty, narrow } => {
                let t = self.expr(value, Want::Any);
                let m = self.m;
                let target = self.c.resolve(m, ty);
                if *narrow {
                    // `x as? T`: a view (or an optional view) of a game object to a class of the same line of
                    // classes (T derives from x's record, or x's record from T): `T?`, none when the object is not
                    // a T (the kernel's as_class at run time) or x is none.
                    let from = match &t {
                        Ty::Opt(x) => (**x).clone(),
                        x => x.clone(),
                    };
                    let fits = match (&from, &target) {
                        (Ty::Error | Ty::Never, _) | (_, Ty::Error) => true,
                        (Ty::Game(GameTy::View { record: a, .. }), Ty::Game(GameTy::View { record: b, .. })) => {
                            self.derives(b, a) || self.derives(a, b)
                        }
                        _ => false,
                    };
                    if !fits {
                        let (f, w) = (self.ty_text(&t), self.ty_text(&target));
                        self.say(Diagnostic::new(code::AS_NUMBERS, e.at).with("found", Arg::Text(f)).with("expected", Arg::Text(w)));
                        return Ty::Error;
                    }
                    return match target {
                        Ty::Error => Ty::Error,
                        target => Ty::Opt(Box::new(target)),
                    };
                }
                if !self.converts(&t, &target) {
                    let (f, w) = (self.ty_text(&t), self.ty_text(&target));
                    self.say(Diagnostic::new(code::AS_NUMBERS, e.at).with("found", Arg::Text(f)).with("expected", Arg::Text(w)));
                    return Ty::Error;
                }
                target
            }
        }
    }

    /// A service's enum, whose values are its C header's `int32_t` numbers.
    pub(super) fn service_enum(&self, service: &str, name: &str) -> bool {
        self.game().services.faces.get(service).is_some_and(|f| f.enums.contains_key(name))
    }

    /// `as`: number to number, a game enum to and from an integer, the mod's enum without data to an integer.
    fn converts(&self, from: &Ty, to: &Ty) -> bool {
        let number = |t: &Ty| matches!(t, Ty::Int(_) | Ty::Float(_));
        match (from, to) {
            (Ty::Error | Ty::Never, _) | (_, Ty::Error) => true,
            (a, b) if a == b => true,
            (a, b) if number(a) && number(b) => true,
            (Ty::Game(GameTy::Enum(_)), Ty::Int(_)) | (Ty::Int(_), Ty::Game(GameTy::Enum(_))) => true,
            // A service's enum is a game enum the same way (J, FB 2026-11-12: `time.now().state as u32`).
            (Ty::Service { service, name }, Ty::Int(_)) | (Ty::Int(_), Ty::Service { service, name }) => self.service_enum(service, name),
            (Ty::Enum(e), Ty::Int(_)) => self.c.enum_without_data(*e),
            _ => false,
        }
    }

    /// A number literal: its suffix's type, else its place's (an integer adapts to a float place when it is exact
    /// there: within 2^24 for f32, 2^53 for f64), else `i32` or `f32`.
    fn number(&mut self, at: Pos, text: &str, negate: bool, want: Want<'_>) -> Ty {
        let (num, suffix) = parse_number(text);
        let written = if negate { format!("-{text}") } else { text.to_string() };
        let place = match want {
            Want::Ty(Ty::Opt(inner)) => Some((**inner).clone()),
            Want::Ty(t) => Some(t.clone()),
            _ => None,
        };
        let target = match (suffix, &place) {
            (Some(s), _) => Ty::from_game(GameTy::Scalar(s)),
            (None, Some(t @ (Ty::Int(_) | Ty::Float(_)))) => t.clone(),
            (None, Some(Ty::Json)) => return Ty::Json,
            (None, Some(Ty::Error)) => return Ty::Error,
            (None, _) => match num {
                Num::Float(_) => Ty::Float(Scalar::F32),
                _ => Ty::Int(Scalar::I32),
            },
        };
        let fit = |w: &mut Self| {
            let expected = w.ty_text(&target);
            w.say(Diagnostic::new(code::LITERAL_FIT, at).with("found", Arg::Text(written.clone())).with("expected", Arg::Text(expected)));
            Ty::Error
        };
        match (&num, &target) {
            (Num::Huge, _) => fit(self),
            (Num::Int(v), Ty::Int(s)) => {
                let v = if negate { -v } else { *v };
                let (lo, hi) = int_range(*s);
                if v < lo || v > hi {
                    return fit(self);
                }
                target
            }
            (Num::Int(v), Ty::Float(s)) => {
                let limit: i128 = if *s == Scalar::F32 { 1 << 24 } else { 1 << 53 };
                if v.abs() > limit {
                    return fit(self);
                }
                target
            }
            (Num::Float(f), Ty::Float(s)) => {
                if *s == Scalar::F32 && f.abs() > f64::from(f32::MAX) || !f.is_finite() {
                    return fit(self);
                }
                target
            }
            (Num::Float(_), Ty::Int(_)) if suffix.is_some() => fit(self),
            // A float where an integer goes: its own type, said by the place.
            (Num::Float(_), _) => Ty::Float(Scalar::F32),
            _ => target,
        }
    }

    /// A string: its escapes and interpolated values (each shown, none `T?`:).
    fn string(&mut self, s: &'a ast::StrLit, want: Want<'_>) -> Ty {
        self.str_parts(s);
        match (s.prefix, want) {
            (StrPrefix::Bytes, _) => Ty::Bytes,
            (_, Want::Ty(Ty::Json)) => Ty::Json,
            _ => Ty::Text,
        }
    }

    fn str_parts(&mut self, s: &'a ast::StrLit) {
        for part in &s.parts {
            match part {
                StrPart::Text(t) if s.prefix != StrPrefix::Raw => {
                    if let Some(bad) = bad_escape(t, s.prefix == StrPrefix::Bytes) {
                        self.say(Diagnostic::new(code::BAD_ESCAPE, s.at).with("found", Arg::Text(bad)));
                    }
                }
                StrPart::Text(_) => {}
                StrPart::Expr(x) => {
                    let t = self.expr(x, Want::Any);
                    self.shown(&t, x.at);
                }
            }
        }
    }

    /// A value put in text (interpolation, `log`): a number, `bool`, text, a vector, an enum, a container or tuple of
    /// those; a `T?` is MAY_BE_NONE.
    pub(super) fn shown(&mut self, t: &Ty, at: Pos) {
        if let Ty::Opt(inner) = t {
            let found = self.ty_text(inner);
            self.say(Diagnostic::new(code::MAY_BE_NONE, at).with("found", Arg::Text(found)));
            return;
        }
        if !self.is_shown(t) {
            let found = self.ty_text(t);
            self.say(Diagnostic::new(code::NOT_SHOWN, at).with("found", Arg::Text(found)));
        }
    }

    fn is_shown(&self, t: &Ty) -> bool {
        match t {
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Value(_) | Ty::Text | Ty::Json | Ty::Enum(_) | Ty::Error | Ty::Never => true,
            Ty::Game(GameTy::Enum(_)) => true,
            // A service's enum and `svc.Answer`, by its case's name as a game enum is.
            Ty::Service { service, name } => {
                (service == "svc" && name == "Answer") || self.game().services.faces.get(service).is_some_and(|f| f.enums.contains_key(name))
            }
            Ty::List(e) | Ty::Set(e) => self.is_shown(e),
            Ty::Map(k, v) => self.is_shown(k) && self.is_shown(v),
            Ty::Tuple(items) => items.iter().all(|i| self.is_shown(i)),
            _ => false,
        }
    }

    /// `[a, b]`: of its place's element type, else of its first value that is no literal (literals adapt to it).
    fn list_lit(&mut self, at: Pos, items: &'a [Expr], want: Want<'_>) -> Ty {
        let elem = match want {
            Want::Ty(Ty::List(t)) => Some((**t).clone()),
            Want::Ty(Ty::Opt(o)) => match &**o {
                Ty::List(t) => Some((**t).clone()),
                _ => None,
            },
            Want::Ty(Ty::Json) => {
                for i in items {
                    self.json_element(i);
                }
                return Ty::Json;
            }
            _ => None,
        };
        match elem {
            Some(t) => {
                for i in items {
                    self.expr(i, Want::Ty(&t));
                }
                Ty::List(Box::new(t))
            }
            None if items.is_empty() => {
                if matches!(want, Want::Ty(Ty::Error)) {
                    return Ty::Error;
                }
                self.say(Diagnostic::new(code::TYPE_UNKNOWN_HERE, at).with("found", Arg::Text("[]".into())));
                Ty::Error
            }
            None => Ty::List(Box::new(self.unify(items.iter().collect(), at))),
        }
    }

    /// The type values written together share: the first that is no literal's, the literals adapting to it; of
    /// literals alone `f32` when any has a fraction, else `i32`.
    pub(super) fn unify(&mut self, items: Vec<&'a Expr>, at: Pos) -> Ty {
        let first = items.iter().position(|i| shape(i).is_none());
        let t = match first {
            Some(k) => {
                let t = self.expr(items[k], Want::Any);
                self.stored(t, items[k].at)
            }
            None if items.iter().any(|i| shape(i) == Some(Ty::Open)) => {
                self.say(Diagnostic::new(code::TYPE_UNKNOWN_HERE, at).with("found", Arg::Text("none".into())));
                Ty::Error
            }
            None if items.iter().any(|i| shape(i) == Some(Ty::FloatLit)) => Ty::Float(Scalar::F32),
            None => Ty::Int(Scalar::I32),
        };
        for (k, i) in items.iter().enumerate() {
            if Some(k) != first {
                self.expr(i, Want::Ty(&t));
            }
        }
        t
    }

    /// A value inside a json literal (each by its kind, recursively): a literal becomes a json.Value; any other
    /// value is json data (a number, text, a struct of those...: what `json.encode` takes), else NOT_JSON_DATA.
    fn json_element(&mut self, e: &'a Expr) {
        let literal = shape(e).is_some()
            || matches!(e.kind, ExprKind::List(_) | ExprKind::Braces(_) | ExprKind::Str(_) | ExprKind::Number(_) | ExprKind::True | ExprKind::False | ExprKind::Color(_));
        if literal {
            self.expr(e, Want::Ty(&Ty::Json));
            return;
        }
        let t = self.expr(e, Want::Any);
        if !self.json_data(&t) && !matches!(t, Ty::Never) {
            let found = self.ty_text(&t);
            self.say(Diagnostic::new(code::NOT_JSON_DATA, e.at).with("found", Arg::Text(found)));
        }
    }

    /// `{}`, `{k: v, ...}` (a map), `{a, b}` (a set), by their place where it says.
    fn braces_lit(&mut self, at: Pos, entries: &'a [(Expr, Option<Expr>)], want: Want<'_>) -> Ty {
        let with = entries.iter().filter(|(_, v)| v.is_some()).count();
        if with != 0 && with != entries.len() {
            self.say(Diagnostic::new(code::MIXED_BRACES, at));
            for (k, v) in entries {
                self.expr(k, Want::Ty(&Ty::Error));
                if let Some(v) = v {
                    self.expr(v, Want::Ty(&Ty::Error));
                }
            }
            return Ty::Error;
        }
        let is_map = with != 0;
        let target = match want {
            Want::Ty(Ty::Opt(inner)) => Some((**inner).clone()),
            Want::Ty(t) => Some(t.clone()),
            _ => None,
        };
        match target {
            Some(Ty::Map(k, v)) if is_map || entries.is_empty() => {
                for (key, value) in entries {
                    self.expr(key, Want::Ty(&k));
                    if let Some(value) = value {
                        self.expr(value, Want::Ty(&v));
                    }
                }
                Ty::Map(k, v)
            }
            Some(Ty::Set(t)) if !is_map => {
                for (key, _) in entries {
                    self.expr(key, Want::Ty(&t));
                }
                Ty::Set(t)
            }
            Some(Ty::Json) if is_map || entries.is_empty() => {
                for (key, value) in entries {
                    self.expr(key, Want::Ty(&Ty::Text));
                    if let Some(value) = value {
                        self.json_element(value);
                    }
                }
                Ty::Json
            }
            Some(Ty::Error) => {
                for (key, value) in entries {
                    self.expr(key, Want::Any);
                    if let Some(value) = value {
                        self.expr(value, Want::Any);
                    }
                }
                Ty::Error
            }
            _ if entries.is_empty() => {
                self.say(Diagnostic::new(code::TYPE_UNKNOWN_HERE, at).with("found", Arg::Text("{}".into())));
                Ty::Error
            }
            _ => {
                let keys = self.unify(entries.iter().map(|(k, _)| k).collect(), at);
                let key_ok = match &keys {
                    Ty::Int(_) | Ty::Text | Ty::Bool | Ty::Error | Ty::Game(GameTy::Enum(_)) => true,
                    Ty::Enum(e) => self.c.enum_without_data(*e),
                    _ => false,
                };
                if !key_ok {
                    let found = self.ty_text(&keys);
                    self.say(Diagnostic::new(code::MAP_KEY, entries[0].0.at).with("found", Arg::Text(found)));
                }
                if is_map {
                    let values = self.unify(entries.iter().filter_map(|(_, v)| v.as_ref()).collect(), at);
                    Ty::Map(Box::new(keys), Box::new(values))
                } else {
                    Ty::Set(Box::new(keys))
                }
            }
        }
    }

    /// `model "x"`, `text "Key"(args)`, `data "f.json"`, `nerve "Class.Step"`...: the name is part (f)'s.
    fn content(&mut self, at: Pos, word: &str, path: &'a ast::StrLit, args: Option<&'a [ast::Arg]>) -> Ty {
        self.str_parts(path);
        let plain = path.parts.iter().all(|p| matches!(p, StrPart::Text(_)));
        let name: String = path.parts.iter().map(|p| if let StrPart::Text(t) = p { t.as_str() } else { "" }).collect();
        // Part (f): the name against the mod's content, when the build gives it (an interpolated name is S0015's).
        let mut placeholders = None;
        if let (Some(content), true) = (self.c.facts.content, plain) {
            let known = if word == "text" {
                content.texts.as_ref().map(|texts| {
                    placeholders = texts.get(&name);
                    placeholders.is_some()
                })
            } else {
                content.names.get(word).map(|set| set.contains(&name))
            };
            let maybe_game = content.game_unlisted.contains(word) && !name.contains(':') && !name.contains('/');
            if known == Some(false) && !maybe_game {
                self.say(Diagnostic::new(code::CONTENT_MISSING, path.at).with("kind", Arg::Text(word.to_string())).with("found", Arg::Text(name.clone())));
            }
        }
        if word == "text" {
            let given: Vec<&str> = args.unwrap_or(&[]).iter().filter_map(|a| a.name.as_ref().map(|n| n.name.as_str())).collect();
            for a in args.unwrap_or(&[]) {
                match (&a.name, placeholders) {
                    (None, Some(_)) => {
                        self.say(Diagnostic::new(code::TEXT_ARG_UNNAMED, a.value.at).with("found", Arg::Text(name.clone())));
                    }
                    (Some(n), Some(set)) if !set.contains(&n.name) => {
                        self.say(Diagnostic::new(code::TEXT_ARG_UNKNOWN, n.at).with("found", Arg::Text(name.clone())).with("name", Arg::Text(n.name.clone())));
                    }
                    _ => {}
                }
            }
            if let Some(set) = placeholders {
                for p in set.iter().filter(|p| !given.contains(&p.as_str())) {
                    self.say(Diagnostic::new(code::TEXT_ARG_MISSING, at).with("found", Arg::Text(name.clone())).with("name", Arg::Text(p.clone())));
                }
            }
        }
        if let Some(args) = args {
            if word == "text" {
                for a in args {
                    let t = self.expr(&a.value, Want::Any);
                    self.shown(&t, a.value.at);
                }
            } else {
                let d = Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(word.to_string())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text("0".into()));
                self.say(d);
                for a in args {
                    self.expr(&a.value, Want::Ty(&Ty::Error));
                }
            }
        }
        match word {
            "text" => Ty::Text,
            "data" => Ty::Bytes,
            other => Ty::Content(other.to_string()),
        }
    }

    /// `ref(x)` (6.2): a handle to a game object the game destroys through NameObj, or to the mod's class.
    fn ref_expr(&mut self, e: &'a Expr, args: &'a [ast::Arg]) -> Ty {
        if args.len() != 1 {
            let d = Diagnostic::new(code::ARG_COUNT, e.at).with("name", Arg::Text("ref".into())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text("1".into()));
            self.say(d);
            for a in args {
                self.expr(&a.value, Want::Ty(&Ty::Error));
            }
            return Ty::Error;
        }
        let t = self.expr(&args[0].value, Want::Any);
        match t {
            Ty::Game(GameTy::View { record, .. }) => {
                if !self.derives(&record, "NameObj") {
                    self.say(Diagnostic::new(code::REF_NOT_DESTROYED, args[0].value.at).with("found", Arg::Text(record)));
                    return Ty::Error;
                }
                Ty::Ref(Box::new(Ty::Game(GameTy::View { record, konst: false })))
            }
            Ty::Class(c) => {
                if !self.class_base(c).is_some_and(|b| self.derives(&b, "NameObj")) {
                    let found = self.c.item_name(c);
                    self.say(Diagnostic::new(code::REF_NOT_DESTROYED, args[0].value.at).with("found", Arg::Text(found)));
                    return Ty::Error;
                }
                Ty::Ref(Box::new(Ty::Class(c)))
            }
            Ty::Error | Ty::Never => Ty::Error,
            other => {
                let found = self.ty_text(&other);
                self.say(Diagnostic::new(code::REF_OF, args[0].value.at).with("found", Arg::Text(found)));
                Ty::Error
            }
        }
    }

    // ------------------------------------------------------------ if, match

    fn if_expr(&mut self, at: Pos, arms: &'a [(Vec<ast::Cond>, Block)], otherwise: Option<&'a Block>, want: Want<'_>) -> Ty {
        let value = !matches!(want, Want::Drop);
        let mut joined: Option<Ty> = match want {
            Want::Ty(t) => Some(t.clone()),
            _ => None,
        };
        if value && otherwise.is_none() {
            self.say(Diagnostic::new(code::IF_NEEDS_ELSE, at));
        }
        let arm_want = if value && otherwise.is_some() { want } else { Want::Drop };
        let mut all_leave = true;
        // Each arm runs after the conditions before it, not after their arms (A42D-3).
        let mut base = self.path_waits();
        let mut reach: Option<super::Path> = None;
        let mut first_leaves = false;
        // What the arms before shows given by not holding (`if x == none { } else { x... }`): an arm with one plain
        // condition runs only when it held, so the later arms and the `else` run where it did not.
        let mut otherwise_given: Vec<String> = Vec::new();
        for (conds, block) in arms {
            self.arm_waits(&base);
            self.scope_push();
            self.given.extend(otherwise_given.iter().cloned());
            self.conds(conds);
            if let [ast::Cond { pattern: None, value }] = conds.as_slice() {
                self.given_by(value, false, &mut otherwise_given);
            }
            base = self.path_waits();
            let leaves = self.arm_block(block, arm_want, &mut joined);
            first_leaves |= leaves && arms.len() == 1;
            all_leave &= leaves;
            if !leaves {
                super::Path::join(&mut reach, &self.path_waits());
            }
            self.scope_pop();
        }
        // `if x == none { ...leaves }` with no other arm: after it, x is given.
        if let ([(conds, _)], None, true) = (arms, otherwise, first_leaves) {
            if let [ast::Cond { pattern: None, value }] = conds.as_slice() {
                let mut shown = Vec::new();
                self.given_by(value, false, &mut shown);
                self.given.extend(shown);
            }
        }
        self.arm_waits(&base);
        match otherwise {
            Some(b) => {
                self.scope_push();
                self.given.extend(otherwise_given);
                let leaves = self.arm_block(b, arm_want, &mut joined);
                self.scope_pop();
                all_leave &= leaves;
                if !leaves {
                    super::Path::join(&mut reach, &self.path_waits());
                }
            }
            None => {
                all_leave = false;
                super::Path::join(&mut reach, &base);
            }
        }
        self.arms_joined(reach, base);
        if !value {
            return if all_leave { Ty::Never } else { Ty::Unit };
        }
        if otherwise.is_none() {
            return Ty::Error;
        }
        joined.unwrap_or(Ty::Never)
    }

    /// One arm's block; with a value wanted, its value joins the others' (the first arm's type, or the place's).
    /// True when the arm always leaves.
    fn arm_block(&mut self, b: &'a Block, want: Want<'_>, joined: &mut Option<Ty>) -> bool {
        let current = joined.clone();
        let w = match (want, &current) {
            (Want::Drop, _) => Want::Drop,
            (_, Some(t)) => Want::Ty(t),
            (w, None) => w,
        };
        let t = self.block(b, w);
        if matches!(w, Want::Drop) {
            return t == Ty::Never;
        }
        match t {
            Ty::Never => true,
            Ty::Unit => {
                self.say(Diagnostic::new(code::NO_VALUE, b.end));
                false
            }
            t => {
                if joined.is_none() {
                    *joined = Some(self.stored(t, b.at));
                }
                false
            }
        }
    }

    fn match_expr(&mut self, at: Pos, subject: &'a Expr, arms: &'a [ast::Arm], want: Want<'_>) -> Ty {
        let st = self.expr(subject, Want::Any);
        let value = !matches!(want, Want::Drop);
        let mut joined: Option<Ty> = match want {
            Want::Ty(t) => Some(t.clone()),
            _ => None,
        };
        let mut pats: Vec<Pat> = Vec::new();
        let mut all_leave = !arms.is_empty();
        // Each arm runs after the subject, not after another arm (A42D-3).
        let base = self.path_waits();
        let mut reach: Option<super::Path> = None;
        for arm in arms {
            self.arm_waits(&base);
            self.scope_push();
            pats.push(match &arm.pattern {
                Some(p) => self.pattern(p, &st, false),
                None => Pat::Any,
            });
            let leaves = match &arm.body {
                ast::ArmBody::Block(b) => self.arm_block(b, want, &mut joined),
                ast::ArmBody::Stmt(s) => match (&s.kind, value) {
                    (ast::StmtKind::Expr(x), true) => {
                        let current = joined.clone();
                        let t = match &current {
                            Some(t) => self.expr(x, Want::Ty(t)),
                            None => self.expr(x, want),
                        };
                        if joined.is_none() && t != Ty::Never {
                            joined = Some(self.stored(t.clone(), x.at));
                        }
                        t == Ty::Never
                    }
                    (_, true) => {
                        let leaves = self.stmt(s);
                        if !leaves {
                            self.say(Diagnostic::new(code::NO_VALUE, s.at));
                        }
                        leaves
                    }
                    (_, false) => self.stmt(s),
                },
            };
            all_leave &= leaves;
            if !leaves {
                super::Path::join(&mut reach, &self.path_waits());
            }
            self.scope_pop();
        }
        self.arms_joined(reach, base);
        self.coverage(at, &st, pats);
        if !value {
            return if all_leave { Ty::Never } else { Ty::Unit };
        }
        joined.unwrap_or(Ty::Never)
    }

    // ------------------------------------------------------------ operators

    fn binary(&mut self, e: &'a Expr, op: BinOp, l: &'a Expr, r: &'a Expr, want: Want<'_>) -> Ty {
        let at = e.at;
        if let Some(kind) = literal_binary(op, l, r) {
            return self.literal_expr(e, kind, op, l, r, want);
        }
        match op {
            BinOp::And | BinOp::Or => {
                self.expr(l, Want::Ty(&Ty::Bool));
                self.expr(r, Want::Ty(&Ty::Bool));
                Ty::Bool
            }
            BinOp::Coalesce => {
                let lt = self.expr(l, Want::Any);
                match lt {
                    Ty::Opt(x) => {
                        let rt = self.expr(r, Want::Ty(&Ty::Opt(x.clone())));
                        if matches!(rt, Ty::Opt(_)) {
                            Ty::Opt(x)
                        } else {
                            *x
                        }
                    }
                    Ty::Error | Ty::Never => {
                        self.expr(r, Want::Any);
                        Ty::Error
                    }
                    other => {
                        let found = self.ty_text(&other);
                        self.say(Diagnostic::new(code::OPERATOR_ONE, at).with("op", Arg::Text("??".into())).with("found", Arg::Text(found)));
                        self.expr(r, Want::Ty(&other));
                        other
                    }
                }
            }
            BinOp::Eq | BinOp::Ne => {
                // `x == none` tests any `T?`, and whether an optional service (`use optional svc.<name>`) was given.
                let none_side = [(l, r), (r, l)].into_iter().find(|(n, _)| matches!(n.kind, ExprKind::None));
                if let Some((n, other)) = none_side {
                    if let ExprKind::Name { name, .. } = &other.kind {
                        if let Some(service) = self.optional_service(&name.name) {
                            self.res.insert(other.id, Resolved::ServiceGiven(service));
                            self.tys.insert(other.id, Ty::Bool);
                            self.tys.insert(n.id, Ty::Bool);
                            return Ty::Bool;
                        }
                    }
                    let t = self.expr(other, Want::Any);
                    match &t {
                        Ty::Opt(_) => {
                            self.expr(n, Want::Ty(&t));
                        }
                        Ty::Error | Ty::Never => {
                            self.tys.insert(n.id, Ty::Error);
                        }
                        _ => {
                            let found = self.ty_text(&t);
                            self.say(Diagnostic::new(code::UNWRAP_NOT_OPTIONAL, other.at).with("found", Arg::Text(found)));
                            self.tys.insert(n.id, Ty::Error);
                        }
                    }
                    return Ty::Bool;
                }
                let t = self.operands(op, l, r, at);
                if !self.equatable(&t) {
                    let found = self.ty_text(&t);
                    self.say(Diagnostic::new(code::NOT_EQUATABLE, at).with("found", Arg::Text(found)));
                }
                Ty::Bool
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let t = self.operands(op, l, r, at);
                if !matches!(t, Ty::Int(_) | Ty::Float(_) | Ty::Text | Ty::Error | Ty::Never) {
                    let found = self.ty_text(&t);
                    self.say(Diagnostic::new(code::NOT_ORDERED, at).with("found", Arg::Text(found)));
                }
                Ty::Bool
            }
            BinOp::Range | BinOp::RangeIncl => {
                let t = self.operands(op, l, r, at);
                match t {
                    Ty::Int(_) => Ty::Range(Box::new(t)),
                    Ty::Error | Ty::Never => Ty::Error,
                    other => {
                        let found = self.ty_text(&other);
                        let d = Diagnostic::new(code::OPERATOR, at).with("op", Arg::Text(op_text(op).into())).with("left", Arg::Text(found.clone())).with("right", Arg::Text(found));
                        self.say(d);
                        Ty::Error
                    }
                }
            }
            _ => {
                let (lt, rt) = self.arith_operands(op, l, r, want);
                self.binary_types(op, &lt, &rt, at)
            }
        }
    }

    /// An expression of literals only (A42C-2): its place's number type when it has one (an integer kind also a
    /// float place), else `i32` or `f32`; an integer's folded value checked to fit once, here, for the whole.
    fn literal_expr(&mut self, e: &'a Expr, kind: Ty, op: BinOp, l: &'a Expr, r: &'a Expr, want: Want<'_>) -> Ty {
        let t = match (number_place(want), &kind) {
            (Some(t @ Ty::Int(_)), Ty::IntLit) | (Some(t @ Ty::Float(_)), _) => t,
            (_, Ty::FloatLit) => Ty::Float(Scalar::F32),
            _ => Ty::Int(Scalar::I32),
        };
        if let (Ty::Int(s), false) = (&t, self.folding) {
            if let Some(v) = fold_int(e) {
                let (lo, hi) = int_range(*s);
                if v < lo || v > hi {
                    let expected = self.ty_text(&t);
                    self.say(Diagnostic::new(code::LITERAL_FIT, e.at).with("found", Arg::Text(v.to_string())).with("expected", Arg::Text(expected)));
                    // Each literal is checked quietly: the whole was said.
                    self.expr(l, Want::Ty(&Ty::Error));
                    self.expr(r, Want::Ty(&Ty::Error));
                    return Ty::Error;
                }
            }
        }
        let outer = std::mem::replace(&mut self.folding, true);
        let lt = self.expr(l, Want::Ty(&t));
        let rt = self.expr(r, Want::Ty(&t));
        self.folding = outer;
        self.binary_types(op, &lt, &rt, e.at)
    }

    /// Two sides compared or ranged: one type, a literal side taking the other's; a mismatch is said once.
    fn operands(&mut self, op: BinOp, l: &'a Expr, r: &'a Expr, at: Pos) -> Ty {
        let (lt, rt) = self.arith_operands(op, l, r, Want::Any);
        if self.fits(&rt, &lt) {
            return if matches!(lt, Ty::Never) { rt } else { lt };
        }
        if self.fits(&lt, &rt) {
            return rt;
        }
        let (a, b) = (self.ty_text(&lt), self.ty_text(&rt));
        self.say(Diagnostic::new(code::OPERATOR, at).with("op", Arg::Text(op_text(op).into())).with("left", Arg::Text(a)).with("right", Arg::Text(b)));
        Ty::Error
    }

    /// Both sides' types, a literal side checked against what the other side gives it; a shift's literal left side
    /// takes the place's number type, else `i32`, never the count's (A42C-1).
    fn arith_operands(&mut self, op: BinOp, l: &'a Expr, r: &'a Expr, want: Want<'_>) -> (Ty, Ty) {
        match (shape(l), shape(r)) {
            (Some(Ty::IntLit), None) if matches!(op, BinOp::Shl | BinOp::Shr) => {
                let rt = self.expr(r, Want::Any);
                let place = match number_place(want) {
                    Some(t @ Ty::Int(_)) => t,
                    _ => Ty::Int(Scalar::I32),
                };
                (self.expr(l, Want::Ty(&place)), rt)
            }
            (Some(_), None) => {
                let rt = self.expr(r, Want::Any);
                let lt = match literal_want(op, &rt) {
                    Some(w) => self.expr(l, Want::Ty(&w)),
                    None => self.expr(l, Want::Any),
                };
                (lt, rt)
            }
            (None, Some(_)) => {
                let lt = self.expr(l, Want::Any);
                let want = if matches!(op, BinOp::Shl | BinOp::Shr) { Some(lt.clone()).filter(|t| matches!(t, Ty::Int(_))) } else { literal_want(op, &lt) };
                let rt = match want {
                    Some(w) => self.expr(r, Want::Ty(&w)),
                    None => self.expr(r, Want::Any),
                };
                (lt, rt)
            }
            (Some(a), Some(b)) if a == Ty::FloatLit || b == Ty::FloatLit => {
                let f = Ty::Float(Scalar::F32);
                (self.expr(l, Want::Ty(&f)), self.expr(r, Want::Ty(&f)))
            }
            _ => (self.expr(l, Want::Any), self.expr(r, Want::Any)),
        }
    }

    /// The type of `l op r` for arithmetic and bits (3): numbers of one type; vectors added and scaled;
    /// colours added and scaled; integers' bits.
    pub(super) fn binary_types(&mut self, op: BinOp, l: &Ty, r: &Ty, at: Pos) -> Ty {
        use Value::{Color, Vec2, Vec3, Vec4};
        if matches!(l, Ty::Error | Ty::Never) || matches!(r, Ty::Error | Ty::Never) {
            return Ty::Error;
        }
        let f32_ = Ty::Float(Scalar::F32);
        let out = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => match (l, r) {
                (Ty::Int(a), Ty::Int(b)) if a == b => Some(l.clone()),
                (Ty::Float(a), Ty::Float(b)) if a == b => Some(l.clone()),
                (Ty::Value(a @ (Vec2 | Vec3 | Vec4)), Ty::Value(b)) if a == b && matches!(op, BinOp::Add | BinOp::Sub) => Some(l.clone()),
                (Ty::Value(Vec2 | Vec3 | Vec4), f) if *f == f32_ && matches!(op, BinOp::Mul | BinOp::Div) => Some(l.clone()),
                (f, Ty::Value(Vec2 | Vec3 | Vec4)) if *f == f32_ && op == BinOp::Mul => Some(r.clone()),
                (Ty::Value(Color), Ty::Value(Color)) if op == BinOp::Add => Some(l.clone()),
                (Ty::Value(Color), f) if *f == f32_ && op == BinOp::Mul => Some(l.clone()),
                _ => None,
            },
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => match (l, r) {
                (Ty::Int(a), Ty::Int(b)) if a == b => Some(l.clone()),
                _ => None,
            },
            BinOp::Shl | BinOp::Shr => match (l, r) {
                (Ty::Int(_), Ty::Int(_)) => Some(l.clone()),
                _ => None,
            },
            _ => None,
        };
        match out {
            Some(t) => t,
            None => {
                let (a, b) = (self.ty_text(l), self.ty_text(r));
                self.say(Diagnostic::new(code::OPERATOR, at).with("op", Arg::Text(op_text(op).into())).with("left", Arg::Text(a)).with("right", Arg::Text(b)));
                Ty::Error
            }
        }
    }

    /// `==` takes every type but functions, views, handles of services, tasks and pending results (3; `same` for
    /// views and handles), and a container, tuple, struct or enum only when each part it holds is taken.
    pub(super) fn equatable(&self, t: &Ty) -> bool {
        self.equatable_in(t, &mut Vec::new())
    }

    /// `seen`: the structs and enums being looked into (a `list<Self>` field leads back to one, which holds then).
    fn equatable_in(&self, t: &Ty, seen: &mut Vec<ItemRef>) -> bool {
        match t {
            // A service's enum and `svc.Answer`; its struct when each field is. Handles and requests: `same`.
            Ty::Service { service, name } => match self.service_fields(service, name) {
                _ if service == "svc" && name == "Answer" => true,
                _ if self.game().services.faces.get(service).is_some_and(|f| f.enums.contains_key(name)) => true,
                Some(fields) if !name.ends_with("Event") => fields.iter().all(|(_, t)| self.equatable_in(t, seen)),
                _ => false,
            },
            // A content value is its name (C2): equal names, equal content; a nerve is the game's.
            Ty::Content(w) => w != "nerve",
            Ty::Fn(..) | Ty::Task | Ty::Pending(_) | Ty::Class(_) | Ty::Group { .. } => false,
            Ty::Game(GameTy::View { .. } | GameTy::Array { .. }) => false,
            Ty::List(e) | Ty::Set(e) | Ty::Opt(e) => self.equatable_in(e, seen),
            Ty::Map(k, v) => self.equatable_in(k, seen) && self.equatable_in(v, seen),
            Ty::Tuple(items) => items.iter().all(|i| self.equatable_in(i, seen)),
            Ty::Struct(item) | Ty::Enum(item) => {
                if seen.contains(item) {
                    return true;
                }
                seen.push(*item);
                let keys: Vec<String> = match &self.item_of(*item).kind {
                    ast::ItemKind::Struct(s) => s.fields.iter().map(|f| format!(".{}", f.name.name)).collect(),
                    ast::ItemKind::Enum(e) => e.cases.iter().flat_map(|c| c.fields.iter().map(move |f| format!("{}.{}", c.name.name, f.name.name))).collect(),
                    _ => Vec::new(),
                };
                let ok = keys.iter().all(|k| self.equatable_in(&self.declared(*item, k), seen));
                seen.pop();
                ok
            }
            _ => true,
        }
    }

    // ------------------------------------------------------------ names

    /// What a bare name names, in the lookup order, saying nothing.
    pub(super) fn name_kind(&self, name: &str) -> NameKind {
        if let Some((id, captured)) = self.find_local(name) {
            return NameKind::Local(id, captured);
        }
        match self.c.scope(self.m).names.get(name) {
            Some(Name::Item { item, .. }) => return NameKind::Item(self.m.item(*item)),
            Some(Name::Taken { item, .. }) => return NameKind::Item(*item),
            Some(Name::Module { target, .. }) => return NameKind::Module(target.clone()),
            None => {}
        }
        if let Some(members) = self.frames.iter().rev().find_map(|f| f.members.clone()) {
            if self.has_member(&members, name) {
                return NameKind::Member(members);
            }
        }
        if let Some(s) = STD_NAMES.iter().find(|s| **s == name) {
            return NameKind::Std(s);
        }
        if let Some(item) = self.c.std_item(name) {
            return NameKind::Item(item);
        }
        if let Some((t, _)) = super::super::BUILTIN_TYPES.iter().chain(super::super::GENERIC_TYPES.iter()).find(|(t, _)| *t == name) {
            return NameKind::Type(t);
        }
        let game = self.game();
        if self.c.game_values.contains_key(name) || game.functions.contains_key(name) || game.global(name).is_ok() {
            return NameKind::Game;
        }
        NameKind::Unknown
    }

    /// Whether the class or record a body is in has a member of this name (the mod's class's own first, then its game
    /// base's).
    fn has_member(&self, members: &Members, name: &str) -> bool {
        match members {
            Members::Mod(class) => {
                if let ast::ItemKind::Class(cd) = &self.item_of(*class).kind {
                    let own = cd.members.iter().any(|m| match m {
                        ast::ClassMember::Binding(b) => {
                            let mut idents = Vec::new();
                            crate::check::pattern_names(&b.pattern, &mut idents);
                            idents.iter().any(|i| i.name == name)
                        }
                        ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) => f.name.name == name,
                        ast::ClassMember::Hook(_) => false,
                    });
                    if own {
                        return true;
                    }
                }
                self.class_base(*class).is_some_and(|b| self.game_has_member(&b, name))
            }
            Members::Game(record) => self.game_has_member(record, name),
        }
    }

    fn game_has_member(&self, record: &str, name: &str) -> bool {
        self.game_field(record, name).is_some() || self.game_method(record, name).is_some()
    }

    fn name_value(&mut self, e: &'a Expr, ident: &'a ast::Ident) -> Ty {
        let name = ident.name.as_str();
        match self.name_kind(name) {
            NameKind::Local(id, captured) => {
                if captured {
                    self.captured.insert(e.id);
                }
                self.res.insert(e.id, Resolved::Local(id));
                self.local_read(id, captured, e.at);
                self.local(id).ty.clone()
            }
            NameKind::Item(item) => self.item_value(e, item, name),
            NameKind::Module(_) => self.not_a_value(e.at, name, "Script.Kind.Module"),
            NameKind::Member(members) => {
                self.self_read(e.at);
                self.member_of_self(e, &members, name)
            }
            NameKind::Std("json") => self.not_a_value(e.at, name, "Script.Kind.Module"),
            NameKind::Std(_) => self.not_a_value(e.at, name, "Script.Kind.Function"),
            NameKind::Type(_) => self.not_a_value(e.at, name, "Script.Kind.Type"),
            NameKind::Game => self.game_named_value(e, name),
            NameKind::Unknown => {
                self.unknown_name(e.at, name);
                Ty::Error
            }
        }
    }

    pub(super) fn not_a_value(&mut self, at: Pos, found: &str, what: &'static str) -> Ty {
        self.say(Diagnostic::new(code::NOT_A_VALUE, at).with("found", Arg::Text(found.to_string())).with("what", Arg::Phrase(what)));
        Ty::Error
    }

    /// An item used as a value: a `let`, `var` or option's value, a function as a value.
    pub(super) fn item_value(&mut self, e: &'a Expr, item: ItemRef, name: &str) -> Ty {
        let kind = &self.item_of(item).kind;
        let t = match kind {
            ast::ItemKind::Binding(_) => self.module_name_ty(item, name, e.at),
            ast::ItemKind::Option { .. } => self.declared(item, ""),
            ast::ItemKind::Fn(f) => self.fn_type(item, "", &f.params, f.ret.is_some()),
            ast::ItemKind::Extern { params, ret, .. } | ast::ItemKind::Export { params, ret, .. } => self.fn_type(item, "", params, ret.is_some()),
            ast::ItemKind::Task { .. } => return self.not_a_value(e.at, name, "Script.Kind.Task"),
            _ => return self.not_a_value(e.at, name, "Script.Kind.Type"),
        };
        self.res.insert(e.id, Resolved::Item(item));
        t
    }

    /// A function's type from its declared parameter and result types.
    pub(super) fn fn_type(&self, owner: ItemRef, key: &str, params: &[ast::Param], has_ret: bool) -> Ty {
        let ps = params.iter().enumerate().filter(|(_, p)| !p.is_self).map(|(i, p)| self.declared(owner, &super::param_key(key, p, i))).collect();
        let r = if has_ret { self.declared(owner, &format!("{key}->")) } else { Ty::Unit };
        Ty::Fn(ps, Box::new(r))
    }

    /// A member of the class or game record the body is in, written without `self.`.
    fn member_of_self(&mut self, e: &'a Expr, members: &Members, name: &str) -> Ty {
        match members {
            Members::Mod(class) => {
                let class = *class;
                if let ast::ItemKind::Class(cd) = &self.item_of(class).kind {
                    let is_fn = cd.members.iter().any(|m| matches!(m, ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) if f.name.name == name));
                    if is_fn {
                        return self.not_a_value(e.at, name, "Script.Kind.Function");
                    }
                    if self.has_own_field(class, name) {
                        let t = self.class_field_ty(class, name, e.at);
                        self.res.insert(e.id, Resolved::Member { owner: class, name: name.to_string() });
                        return t;
                    }
                }
                match self.class_base(class) {
                    Some(base) => self.game_member(e, &base, false, name, false),
                    None => Ty::Error,
                }
            }
            Members::Game(record) => {
                let record = record.clone();
                self.game_member(e, &record, false, name, false)
            }
        }
    }

    fn has_own_field(&self, class: ItemRef, name: &str) -> bool {
        let ast::ItemKind::Class(cd) = &self.item_of(class).kind else { return false };
        cd.members.iter().any(|m| match m {
            ast::ClassMember::Binding(b) => {
                let mut idents = Vec::new();
                crate::check::pattern_names(&b.pattern, &mut idents);
                idents.iter().any(|i| i.name == name)
            }
            _ => false,
        })
    }

    /// A game name written bare: an enum's value, a global, or a function (no value).
    fn game_named_value(&mut self, e: &'a Expr, name: &str) -> Ty {
        if let Some(v) = self.c.game_values.get(name).cloned() {
            return self.game_enum_value(e, v);
        }
        match self.game().global(name) {
            Ok(place) => return self.game_global(e, name, place),
            Err(NotCrossing::NotKnown(_)) => {}
            Err(why) => return self.not_crossing(e.at, name, &why),
        }
        self.not_a_value(e.at, name, "Script.Kind.Function")
    }

    fn game_enum_value(&mut self, e: &Expr, v: super::GameValue) -> Ty {
        let t = if v.enum_name.is_empty() { Ty::Int(Scalar::I32) } else { Ty::Game(GameTy::Enum(v.enum_name.clone())) };
        self.res.insert(e.id, Resolved::GameEnumValue { enum_name: v.enum_name, value: v.value, number: v.number });
        t
    }

    fn game_global(&mut self, e: &Expr, name: &str, place: PlaceTy) -> Ty {
        match &place.constant {
            Some(value) => {
                self.res.insert(e.id, Resolved::GameConstant { name: name.to_string(), value: value.clone() });
            }
            None => {
                self.res.insert(e.id, Resolved::GameGlobal(name.to_string()));
            }
        }
        if place.read_only {
            self.read_only.insert(e.id, "Script.Place.ReadOnly");
        }
        self.place_ty(e, &place.ty, false)
    }

    /// A game field's or global's type as a script reads it; a pointer to a number or vector is a place its `!`
    /// writes through, unless const.
    fn place_ty(&mut self, e: &Expr, g: &GameTy, konst_view: bool) -> Ty {
        match g {
            GameTy::View { record, konst } => Ty::Game(GameTy::View { record: record.clone(), konst: *konst || konst_view }),
            GameTy::Opt(inner) => {
                if let GameTy::Place { konst, .. } = &**inner {
                    if !konst && !konst_view {
                        self.game_places.insert(e.id);
                    }
                }
                from_game_place(g)
            }
            other => from_game_place(other),
        }
    }

    fn not_crossing(&mut self, at: Pos, found: &str, why: &NotCrossing) -> Ty {
        let d = Diagnostic::new(code::GAME_TYPE_NOT_CROSSING, at).with("found", Arg::Text(found.to_string())).with("why", Arg::Phrase(crate::check::why_phrase(why)));
        self.say(d);
        Ty::Error
    }

    /// `Game::name` not called: an enum's value, a global or constant, else what it is.
    fn game_value(&mut self, e: &'a Expr, gn: &'a ast::GameName) -> Ty {
        let text = gn.text.as_str();
        if gn.quoted && text.contains('(') {
            return self.not_a_value(e.at, text, "Script.Kind.Function");
        }
        if let Some(v) = self.c.game_values.get(text).cloned() {
            return self.game_enum_value(e, v);
        }
        match self.game().global(text) {
            Ok(place) => return self.game_global(e, text, place),
            Err(NotCrossing::NotKnown(_)) => {}
            Err(why) => return self.not_crossing(e.at, text, &why),
        }
        if self.game().functions.contains_key(text) {
            return self.not_a_value(e.at, text, "Script.Kind.Function");
        }
        match self.game().type_by_name(text) {
            Ok(_) => self.not_a_value(e.at, text, "Script.Kind.Type"),
            Err(NotCrossing::NotKnown(_)) => {
                self.unknown_name(e.at, text);
                Ty::Error
            }
            Err(why) => self.not_crossing(e.at, text, &why),
        }
    }

    // ------------------------------------------------------------ members

    /// A game record's field by name, its own or reached in a base (types `reachable`): the record declaring it and
    /// its place, or why it does not cross.
    pub(super) fn game_field(&self, record: &str, name: &str) -> Option<Result<PlaceTy, NotCrossing>> {
        let types = self.game().types.as_ref()?;
        let (_, rec) = types.record(record)?;
        if let Some(f) = rec.own.iter().find(|f| f.name == name) {
            return Some(self.game().field(f));
        }
        let reached = rec.reached.iter().find(|f| f.name == name)?;
        let (_, from) = types.record(&reached.from)?;
        let f = from.own.iter().find(|f| f.name == name)?;
        Some(self.game().field(f))
    }

    /// Whether a game record has fields of an anonymous member `name` (`mFlags.isGettable`: `mFlags`).
    fn game_group(&self, record: &str, name: &str) -> bool {
        let Some((_, rec)) = self.game().types.as_ref().and_then(|t| t.record(record)) else { return false };
        let prefix = format!("{name}.");
        rec.own.iter().any(|f| f.name.starts_with(&prefix)) || rec.reached.iter().any(|f| f.name.starts_with(&prefix))
    }

    /// The function rows' name for a game record's member function: the record's own, else the nearest base's that
    /// has one (C++'s name hiding).
    pub(super) fn game_method(&self, record: &str, name: &str) -> Option<String> {
        let game = self.game();
        let types = game.types.as_ref();
        let mut queue = std::collections::VecDeque::from([record.to_string()]);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(r) = queue.pop_front() {
            if !seen.insert(r.clone()) {
                continue;
            }
            let full = format!("{r}::{name}");
            if game.functions.get(&full).is_some_and(|rows| rows.iter().any(|(_, tell)| !tell)) {
                return Some(full);
            }
            if let Some((_, rec)) = types.and_then(|t| t.record(&r)) {
                for b in &rec.bases {
                    let row = types.and_then(|t| t.record(&b.name)).map_or(b.name.clone(), |(row, _)| row.to_string());
                    queue.push_back(row);
                }
            }
        }
        None
    }

    /// A member of a game record read as a value: a field (its place), an anonymous member, or a function
    /// (no value). `copy`: read from the script's own copy, where fields are the copy's.
    pub(super) fn game_member(&mut self, e: &Expr, record: &str, konst: bool, name: &str, copy: bool) -> Ty {
        match self.game_field(record, name) {
            Some(Ok(place)) => {
                self.res.insert(e.id, Resolved::GameField { record: record.to_string(), field: name.to_string() });
                if !copy {
                    if place.read_only {
                        self.read_only.insert(e.id, "Script.Place.ReadOnly");
                    } else if konst {
                        self.read_only.insert(e.id, "Script.Place.ConstView");
                    }
                }
                let t = self.place_ty(e, &place.ty, konst);
                // A record field of a copy is part of the copy.
                match (copy, t) {
                    (true, Ty::Game(GameTy::View { record, .. })) if self.plain_struct(&record) => Ty::Game(GameTy::Struct(record)),
                    (_, t) => t,
                }
            }
            Some(Err(why)) => self.not_crossing(e.at, &format!("{record}.{name}"), &why),
            None if self.game_group(record, name) => Ty::Group { record: record.to_string(), prefix: name.to_string(), konst },
            None if self.game_method(record, name).is_some() => self.not_a_value(e.at, name, "Script.Kind.Function"),
            None => {
                self.say(Diagnostic::new(code::NO_MEMBER, e.at).with("name", Arg::Text(name.to_string())).with("found", Arg::Text(record.to_string())));
                Ty::Error
            }
        }
    }

    /// `base.name` as a value: a module's item, an enum's case, an integer type's limit, or a member of a value.
    fn member_value(&mut self, e: &'a Expr, base: &'a Expr, name: &'a ast::Ident, optional: bool) -> Ty {
        // `<svc>.<Enum>.<Case>`.
        if let ExprKind::Member { base: inner, name: ty_name, optional: false, .. } = &base.kind {
            if let ExprKind::Name { name: b, .. } = &inner.kind {
                if let NameKind::Module(ModTarget::Service(service)) = self.name_kind(&b.name) {
                    if !self.game().services.faces.is_empty() {
                        return self.service_case(e, &service, ty_name, name);
                    }
                }
            }
        }
        // `module.Type.name`: as `Type.name` in that module.
        match self.qualified_type(base) {
            Some(Some(item)) => return self.type_member(e, item, name),
            Some(None) => return Ty::Error,
            None => {}
        }
        if let ExprKind::Name { name: b, .. } = &base.kind {
            match self.name_kind(&b.name) {
                NameKind::Module(ModTarget::Service(service)) => return self.service_member(e, &service, name),
                NameKind::Module(target) => return self.module_member(e, base, &target, name),
                NameKind::Item(item) if matches!(self.item_of(item).kind, ast::ItemKind::Enum(_) | ast::ItemKind::Struct(_) | ast::ItemKind::Class(_)) => {
                    self.res.insert(base.id, Resolved::Item(item));
                    return self.type_member(e, item, name);
                }
                NameKind::Type(t) => {
                    let scalar = match self.c.builtin(self.m, t, Vec::new(), &[]) {
                        Ty::Int(s) => Some(s),
                        _ => None,
                    };
                    return match (scalar, name.name.as_str()) {
                        (Some(s), "max" | "min") => {
                            self.res.insert(e.id, Resolved::Limit { ty: s, max: name.name == "max" });
                            Ty::Int(s)
                        }
                        _ => {
                            self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(t.to_string())));
                            Ty::Error
                        }
                    };
                }
                NameKind::Std("json") => {
                    return match name.name.as_str() {
                        "Value" => self.not_a_value(e.at, "json.Value", "Script.Kind.Type"),
                        "encode" | "decode" => self.not_a_value(e.at, &format!("json.{}", name.name), "Script.Kind.Function"),
                        _ => {
                            self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text("json".into())));
                            Ty::Error
                        }
                    };
                }
                _ => {}
            }
        }
        let bt = self.expr(base, Want::Any);
        let inner = match (bt, optional) {
            (Ty::Opt(x), true) => *x,
            (Ty::Opt(x), false) => {
                let found = self.ty_text(&x);
                self.say(Diagnostic::new(code::MAY_BE_NONE, base.at).with("found", Arg::Text(found)));
                return Ty::Error;
            }
            (t @ (Ty::Error | Ty::Never), _) => {
                let _ = t;
                return Ty::Error;
            }
            (other, true) => {
                let found = self.ty_text(&other);
                self.say(Diagnostic::new(code::UNWRAP_NOT_OPTIONAL, base.at).with("found", Arg::Text(found)));
                other
            }
            (other, false) => other,
        };
        let copy = !self.is_view_expr(base);
        let t = self.member_of(e, &inner, name, copy);
        match (optional, t) {
            (true, Ty::Opt(x)) => Ty::Opt(x),
            (true, Ty::Error) => Ty::Error,
            (true, t) => Ty::Opt(Box::new(t)),
            (false, t) => t,
        }
    }

    /// Whether an expression's value is the game's object rather than the script's copy (so its fields are places).
    fn is_view_expr(&self, e: &Expr) -> bool {
        !matches!(self.tys.get(&e.id), Some(Ty::Game(GameTy::Struct(_))))
    }

    /// A member of a value of type `t`.
    pub(super) fn member_of(&mut self, e: &'a Expr, t: &Ty, name: &'a ast::Ident, copy: bool) -> Ty {
        let n = name.name.as_str();
        let no_member = |w: &mut Self, found: String| {
            w.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(n.to_string())).with("found", Arg::Text(found)));
            Ty::Error
        };
        match t {
            Ty::Struct(item) => {
                let item = *item;
                let ast::ItemKind::Struct(s) = &self.item_of(item).kind else { return Ty::Error };
                if s.fields.iter().any(|f| f.name.name == n) {
                    self.res.insert(e.id, Resolved::Member { owner: item, name: n.to_string() });
                    return self.declared(item, &format!(".{n}"));
                }
                if s.fns.iter().any(|f| f.name.name == n) {
                    return self.not_a_value(name.at, n, "Script.Kind.Function");
                }
                let found = self.c.item_name(item);
                no_member(self, found)
            }
            Ty::Class(class) => {
                let class = *class;
                if self.has_own_field(class, n) {
                    self.res.insert(e.id, Resolved::Member { owner: class, name: n.to_string() });
                    return self.class_field_ty(class, n, name.at);
                }
                let ast::ItemKind::Class(cd) = &self.item_of(class).kind else { return Ty::Error };
                if cd.members.iter().any(|m| matches!(m, ast::ClassMember::Fn(f) | ast::ClassMember::Override(f) if f.name.name == n)) {
                    return self.not_a_value(name.at, n, "Script.Kind.Function");
                }
                match self.class_base(class) {
                    Some(base) => self.game_member(e, &base, false, n, false),
                    None => Ty::Error,
                }
            }
            Ty::Tuple(items) => match n.parse::<usize>() {
                Ok(k) if k < items.len() => {
                    self.res.insert(e.id, Resolved::TupleIndex(k));
                    items[k].clone()
                }
                _ => {
                    let found = self.ty_text(t);
                    no_member(self, found)
                }
            },
            Ty::Value(v) => {
                let names: &[&str] = match v {
                    Value::Vec2 => &["x", "y"],
                    Value::Vec3 => &["x", "y", "z"],
                    Value::Vec4 | Value::Quat => &["x", "y", "z", "w"],
                    Value::Color => &["r", "g", "b", "a"],
                    Value::Mat34 => &[],
                };
                match names.iter().position(|c| *c == n) {
                    Some(k) => {
                        self.res.insert(e.id, Resolved::Component(k));
                        Ty::Float(Scalar::F32)
                    }
                    None => {
                        let found = self.ty_text(t);
                        no_member(self, found)
                    }
                }
            }
            Ty::Game(GameTy::View { record, konst }) => {
                let (record, konst) = (record.clone(), *konst);
                self.game_member(e, &record, konst, n, false)
            }
            Ty::Game(GameTy::Struct(record)) => {
                let record = record.clone();
                self.game_member(e, &record, false, n, copy)
            }
            Ty::Group { record, prefix, konst } => {
                let (record, field, konst) = (record.clone(), format!("{prefix}.{n}"), *konst);
                self.game_member(e, &record, konst, &field, false)
            }
            // Not described (no SDK headers): said once at its `use`.
            Ty::Service { .. } if self.game().services.faces.is_empty() => Ty::Error,
            Ty::Service { service, name: record } => {
                let (service, record) = (service.clone(), record.clone());
                self.service_field(e, &service, &record, name)
            }
            Ty::Error | Ty::Never => Ty::Error,
            other => {
                if super::call::method_known(other, n) {
                    return self.not_a_value(name.at, n, "Script.Kind.Function");
                }
                let found = self.ty_text(other);
                no_member(self, found)
            }
        }
    }

    /// `module.name` (an item of a used module, `pub`).
    fn module_member(&mut self, e: &'a Expr, base: &Expr, target: &ModTarget, name: &'a ast::Ident) -> Ty {
        let _ = base;
        let ModTarget::Module { owner, module } = target else { return Ty::Error };
        let m = ModRef { owner: *owner, module: *module };
        match self.c.scope(m).names.get(&name.name) {
            Some(Name::Item { item, public: true, .. }) => {
                let item = m.item(*item);
                self.item_value(e, item, &name.name)
            }
            Some(Name::Item { .. }) => {
                let module = self.module_path(m);
                self.say(Diagnostic::new(code::NAME_NOT_PUB, name.at).with("name", Arg::Text(name.name.clone())).with("module", Arg::Text(module)));
                Ty::Error
            }
            _ => {
                let module = self.module_path(m);
                self.say(Diagnostic::new(code::NO_NAME_IN_MODULE, name.at).with("name", Arg::Text(name.name.clone())).with("module", Arg::Text(module)));
                Ty::Error
            }
        }
    }

    /// A module's path as a `use` writes it, for messages.
    pub(super) fn module_path(&self, m: ModRef) -> String {
        match m.owner {
            crate::check::Owner::Mod => format!("./{}", self.c.modules[m.module].path),
            crate::check::Owner::Library(l) => {
                let lib = &self.c.libraries[l];
                let id = if lib.id == crate::check::LUMAKIT { "kit" } else { lib.id.as_str() };
                format!("{id}.{}", lib.modules[m.module].path.replace('/', "."))
            }
        }
    }

    /// `Type.name` as a value: an enum's case without data.
    fn type_member(&mut self, e: &'a Expr, item: ItemRef, name: &'a ast::Ident) -> Ty {
        match &self.item_of(item).kind {
            ast::ItemKind::Enum(decl) => match decl.cases.iter().find(|c| c.name.name == name.name) {
                Some(case) if case.fields.is_empty() => {
                    self.res.insert(e.id, Resolved::Case { item, case: case.name.name.clone() });
                    Ty::Enum(item)
                }
                Some(_) => self.not_a_value(e.at, &format!("{}.{}", decl.name.name, name.name), "Script.Kind.Function"),
                None => {
                    self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(decl.name.name.clone())));
                    Ty::Error
                }
            },
            ast::ItemKind::Struct(s) if s.fns.iter().any(|f| f.name.name == name.name) => self.not_a_value(e.at, &name.name, "Script.Kind.Function"),
            _ => {
                let found = self.c.item_name(item);
                self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(found)));
                Ty::Error
            }
        }
    }

    // ------------------------------------------------------------ struct literals, indexes

    /// `Name { field: value, shorthand }`: a struct of the mod, or an enum's case with braces.
    fn struct_lit(&mut self, at: Pos, path: &'a [ast::Ident], fields: &'a [(ast::Ident, Option<Expr>)]) -> Ty {
        let written = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
        match self.name_kind(&path[0].name) {
            NameKind::Module(ModTarget::Service(_)) if self.game().services.faces.is_empty() || path.len() != 2 => return self.struct_quiet(fields),
            NameKind::Module(ModTarget::Service(service)) => return self.service_struct_lit(at, &service, &path[1], fields),
            NameKind::Unknown => {
                self.unknown_name(path[0].at, &path[0].name);
                return self.struct_quiet(fields);
            }
            _ => {}
        }
        let target = self.struct_path(path);
        let (owner, prefix, decl_fields, result, shown) = match target {
            Some((item, None)) => match &self.item_of(item).kind {
                ast::ItemKind::Struct(s) => (item, ".".to_string(), &s.fields, Ty::Struct(item), s.name.name.clone()),
                _ => return self.not_struct(at, &written, fields),
            },
            Some((item, Some(case))) => {
                let ast::ItemKind::Enum(decl) = &self.item_of(item).kind else { return self.not_struct(at, &written, fields) };
                let c = &decl.cases[case];
                self.named.insert(path[path.len() - 1].at, Resolved::Case { item, case: c.name.name.clone() });
                (item, format!("{}.", c.name.name), &c.fields, Ty::Enum(item), format!("{}.{}", decl.name.name, c.name.name))
            }
            None => return self.not_struct(at, &written, fields),
        };
        let mut given: Vec<&str> = Vec::new();
        for (ident, value) in fields {
            let Some(f) = decl_fields.iter().find(|f| f.name.name == ident.name) else {
                self.say(Diagnostic::new(code::NO_MEMBER, ident.at).with("name", Arg::Text(ident.name.clone())).with("found", Arg::Text(shown.clone())));
                if let Some(v) = value {
                    self.expr(v, Want::Ty(&Ty::Error));
                }
                continue;
            };
            if given.contains(&ident.name.as_str()) {
                self.say(Diagnostic::new(code::ARG_TWICE, ident.at).with("name", Arg::Text(ident.name.clone())));
            }
            given.push(&ident.name);
            let ty = self.declared(owner, &format!("{prefix}{}", f.name.name));
            match value {
                Some(v) => {
                    self.named.insert(ident.at, Resolved::Member { owner, name: ident.name.clone() });
                    self.expr(v, Want::Ty(&ty));
                }
                None => {
                    // `{ x }`: the local (or other name) `x` as the field `x`.
                    let found = self.shorthand(ident);
                    self.check_fit(ident.at, found, &ty);
                }
            }
        }
        for f in decl_fields.iter() {
            if f.default.is_none() && !given.contains(&f.name.name.as_str()) {
                self.say(Diagnostic::new(code::FIELD_MISSING, at).with("name", Arg::Text(f.name.name.clone())).with("found", Arg::Text(shown.clone())));
            }
        }
        result
    }

    /// A shorthand field's value: the name as an expression would read it (recorded at the name's place).
    pub(super) fn shorthand(&mut self, ident: &'a ast::Ident) -> Ty {
        match self.name_kind(&ident.name) {
            NameKind::Local(id, captured) => {
                self.named.insert(ident.at, Resolved::Local(id));
                self.local_read(id, captured, ident.at);
                self.local(id).ty.clone()
            }
            NameKind::Item(item) if matches!(self.item_of(item).kind, ast::ItemKind::Binding(_) | ast::ItemKind::Option { .. }) => {
                self.named.insert(ident.at, Resolved::Item(item));
                match &self.item_of(item).kind {
                    ast::ItemKind::Binding(_) => self.module_name_ty(item, &ident.name, ident.at),
                    _ => self.declared(item, ""),
                }
            }
            NameKind::Member(Members::Mod(class)) if self.has_own_field(class, &ident.name) => {
                self.self_read(ident.at);
                self.named.insert(ident.at, Resolved::Member { owner: class, name: ident.name.clone() });
                self.class_field_ty(class, &ident.name, ident.at)
            }
            NameKind::Unknown => {
                self.unknown_name(ident.at, &ident.name);
                Ty::Error
            }
            _ => self.not_a_value(ident.at, &ident.name, "Script.Kind.Function"),
        }
    }

    fn not_struct(&mut self, at: Pos, written: &str, fields: &'a [(ast::Ident, Option<Expr>)]) -> Ty {
        self.say(Diagnostic::new(code::NOT_A_STRUCT, at).with("found", Arg::Text(written.to_string())));
        self.struct_quiet(fields)
    }

    /// A struct literal that did not check: its values checked where anything goes, nothing said of them as fields.
    pub(super) fn struct_quiet(&mut self, fields: &'a [(ast::Ident, Option<Expr>)]) -> Ty {
        for (ident, v) in fields {
            match v {
                Some(v) => {
                    self.expr(v, Want::Ty(&Ty::Error));
                }
                None => {
                    self.shorthand(ident);
                }
            }
        }
        Ty::Error
    }

    /// A struct literal's path: the struct, or the enum and its case's index.
    fn struct_path(&self, path: &[ast::Ident]) -> Option<(ItemRef, Option<usize>)> {
        let (item, rest) = match self.name_kind(&path[0].name) {
            NameKind::Item(item) => (item, &path[1..]),
            NameKind::Module(ModTarget::Module { owner, module }) if path.len() >= 2 => {
                let m = ModRef { owner, module };
                match self.c.scope(m).names.get(&path[1].name) {
                    Some(Name::Item { item, public: true, .. }) => (m.item(*item), &path[2..]),
                    _ => return None,
                }
            }
            _ => return None,
        };
        match (&self.item_of(item).kind, rest) {
            (ast::ItemKind::Struct(_), []) => Some((item, None)),
            (ast::ItemKind::Enum(decl), [case]) => decl.cases.iter().position(|c| c.name.name == case.name && c.braces).map(|k| (item, Some(k))),
            _ => None,
        }
    }

    fn index(&mut self, e: &'a Expr, base: &'a Expr, index: &'a Expr) -> Ty {
        let bt = self.expr(base, Want::Any);
        let int_index = |w: &mut Self| {
            if shape(index).is_some() {
                w.expr(index, Want::Ty(&Ty::Int(Scalar::I32)));
            } else {
                let t = w.expr(index, Want::Any);
                if !matches!(t, Ty::Int(_) | Ty::Error | Ty::Never) {
                    let found = w.ty_text(&t);
                    w.say(Diagnostic::new(code::TYPE_MISMATCH, index.at).with("found", Arg::Text(found)).with("expected", Arg::Text("int".into())));
                }
            }
        };
        match bt {
            Ty::List(t) => {
                int_index(self);
                *t
            }
            Ty::Bytes => {
                int_index(self);
                Ty::Int(Scalar::U8)
            }
            Ty::Map(k, v) => {
                self.expr(index, Want::Ty(&k));
                *v
            }
            Ty::Game(GameTy::Array { of, extents }) => {
                int_index(self);
                if let Some(why) = self.read_only.get(&base.id).copied() {
                    self.read_only.insert(e.id, why);
                }
                array_element(&of, &extents)
            }
            Ty::Opt(x) => {
                let found = self.ty_text(&x);
                self.say(Diagnostic::new(code::MAY_BE_NONE, base.at).with("found", Arg::Text(found)));
                self.expr(index, Want::Ty(&Ty::Error));
                Ty::Error
            }
            Ty::Error | Ty::Never => {
                self.expr(index, Want::Ty(&Ty::Error));
                Ty::Error
            }
            other => {
                let found = self.ty_text(&other);
                self.say(Diagnostic::new(code::NOT_INDEXABLE, e.at).with("found", Arg::Text(found)));
                self.expr(index, Want::Ty(&Ty::Error));
                Ty::Error
            }
        }
    }
}
