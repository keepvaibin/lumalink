//! Calls: the mod's functions and its structs' and classes' (5.1, 5.2), enum cases, values of function
//! type (5.4), the standard names (16a), the builtin methods (language.md 6.2, 11.1), the game's
//! functions with their overloads (7.2, revision 3) and `start` (11.1).

use super::expr::{from_game_place, shape, NameKind};
use super::{Members, Resolved, Walk, Want};
use crate::ast::{self, Expr, ExprKind, Pos};
use crate::check::{ItemRef, ModRef, ModTarget, Name, Ty};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::{FnTy, GameTy, NotCrossing, Scalar, SigNotCrossing, TableLeaf, TablePart, Value};
use luma_validate::names::FunctionKind as FnKind;

/// The names every module has without `use` (16a), besides the type names.
pub(crate) const STD_NAMES: &[&str] = &[
    "min", "max", "abs", "clamp", "lerp", "sqrt", "sin", "cos", "atan2", "dot", "cross", "length", "normal", "hash32", "same", "low_bits", "vec2", "vec3", "vec4",
    "quat", "color", "json", "is_state", "keep", "kept", "caller", "model_named", "effect_named", "sound_named", "layout_named", "font_named",
    "movie_named", "make", "mat34", "alias_class", "placement_of", "placed_at", "make_gravity", "set_number", "set_table", "table",
    "as_class",
];

const INT: Ty = Ty::Int(Scalar::I32);
const F32: Ty = Ty::Float(Scalar::F32);

/// A parameter a call's arguments are matched to.
pub(super) struct P {
    pub(super) name: Option<String>,
    pub(super) ty: Ty,
    pub(super) var: bool,
    pub(super) default: bool,
}

impl P {
    pub(super) fn new(name: &str, ty: Ty) -> P {
        P { name: Some(name.to_string()), ty, var: false, default: false }
    }
}

/// What a builtin method's closure argument is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Closure {
    /// `fn(T) -> bool`.
    Test,
    /// `fn(T) -> U`, giving `list<U>`.
    Map,
    /// `fn(T) -> N`, N a number, giving N.
    Sum,
    /// `fn(T, T) -> bool`.
    Order,
}

/// A builtin method's signature.
struct Builtin {
    name: &'static str,
    params: Vec<P>,
    ret: Ty,
    /// It changes its value: the value must be a place that may be written.
    mutates: bool,
    closure: Option<(Closure, Ty)>,
    /// What the element type must allow (`contains` needs `==`, `sort` needs `<`).
    needs: Option<&'static str>,
}

fn opt(t: Ty) -> Ty {
    Ty::Opt(Box::new(t))
}

fn list(t: Ty) -> Ty {
    Ty::List(Box::new(t))
}

/// The builtin method `name` of a value of type `t` (language.md 6.2, 11.1).
/// Every name [`builtin`] knows, of any type (a test reads `builtin`'s arms against it).
const METHOD_NAMES: &[&str] = &[
    "len", "push", "pop", "insert", "remove", "get", "contains", "indexed", "first", "last", "sort", "map", "filter", "any", "all", "sum",
    "sort_by", "keys", "values", "add", "row", "column", "name", "chars", "split", "starts_with", "ends_with", "find", "slice", "lower",
    "upper", "parse_int", "parse_float", "bytes", "text", "read_u16", "read_u32", "read_u64", "read_f32", "read_f64", "write_u16",
    "write_u32", "write_u64", "write_f32", "write_f64", "int", "float", "bool", "list", "at", "stop",
];

/// The builtin methods of a value of type `t`: name, parameters (name, type), result, whether it changes the value and
/// whether it takes a closure (whose parameter and result the call decides).
pub(crate) fn builtins_of(t: &Ty) -> Vec<(&'static str, Vec<(String, Ty)>, Ty, bool, bool)> {
    METHOD_NAMES
        .iter()
        .filter_map(|n| builtin(t, n))
        .map(|b| (b.name, b.params.into_iter().map(|p| (p.name.unwrap_or_default(), p.ty)).collect(), b.ret, b.mutates, b.closure.is_some()))
        .collect()
}

fn builtin(t: &Ty, name: &str) -> Option<Builtin> {
    let b = |name: &'static str, params: Vec<P>, ret: Ty| Builtin { name, params, ret, mutates: false, closure: None, needs: None };
    let m = |name: &'static str, params: Vec<P>, ret: Ty| Builtin { name, params, ret, mutates: true, closure: None, needs: None };
    let c = |name: &'static str, kind: Closure, elem: &Ty, ret: Ty, mutates: bool| Builtin { name, params: Vec::new(), ret, mutates, closure: Some((kind, elem.clone())), needs: None };
    let found = match t {
        Ty::List(e) => {
            let e = (**e).clone();
            match name {
                "len" => b("len", vec![], INT),
                "push" => m("push", vec![P::new("value", e)], Ty::Unit),
                "pop" => m("pop", vec![], opt(e)),
                "insert" => m("insert", vec![P::new("at", INT), P::new("value", e)], Ty::Unit),
                "remove" => m("remove", vec![P::new("at", INT)], e),
                "get" => b("get", vec![P::new("at", INT)], opt(e)),
                "contains" => Builtin { needs: Some("=="), ..b("contains", vec![P::new("value", e)], Ty::Bool) },
                "indexed" => b("indexed", vec![], list(Ty::Tuple(vec![INT, e]))),
                "first" => b("first", vec![], opt(e)),
                "last" => b("last", vec![], opt(e)),
                "sort" => Builtin { needs: Some("<"), ..m("sort", vec![], Ty::Unit) },
                "map" => c("map", Closure::Map, &e, Ty::Error, false),
                "filter" => c("filter", Closure::Test, &e, list(e.clone()), false),
                "any" => c("any", Closure::Test, &e, Ty::Bool, false),
                "all" => c("all", Closure::Test, &e, Ty::Bool, false),
                "sum" => c("sum", Closure::Sum, &e, Ty::Error, false),
                "sort_by" => c("sort_by", Closure::Order, &e, Ty::Unit, true),
                _ => return None,
            }
        }
        Ty::Map(k, v) => {
            let (k, v) = ((**k).clone(), (**v).clone());
            match name {
                "len" => b("len", vec![], INT),
                "get" => b("get", vec![P::new("key", k)], opt(v)),
                "contains" => b("contains", vec![P::new("key", k)], Ty::Bool),
                "remove" => m("remove", vec![P::new("key", k)], opt(v)),
                "keys" => b("keys", vec![], list(k)),
                "values" => b("values", vec![], list(v)),
                _ => return None,
            }
        }
        Ty::Set(e) => {
            let e = (**e).clone();
            match name {
                "len" => b("len", vec![], INT),
                "add" => m("add", vec![P::new("value", e)], Ty::Bool),
                "remove" => m("remove", vec![P::new("value", e)], Ty::Bool),
                "contains" => b("contains", vec![P::new("value", e)], Ty::Bool),
                "values" => b("values", vec![], list(e)),
                _ => return None,
            }
        }
        // C4: a matrix's row (0 to 2) and column (0 to 3, the last its translation); outside, S0902.
        Ty::Value(Value::Mat34) => match name {
            "row" => b("row", vec![P::new("i", INT)], Ty::Value(Value::Vec4)),
            "column" => b("column", vec![P::new("i", INT)], Ty::Value(Value::Vec3)),
            _ => return None,
        },
        // C2: a content value's name (`model "x"` -> "x"), as a service's text parameter takes it.
        Ty::Content(w) if w != "nerve" => match name {
            "name" => b("name", vec![], Ty::Text),
            _ => return None,
        },
        Ty::Text => match name {
            "len" => b("len", vec![], INT),
            "chars" => b("chars", vec![], list(Ty::Text)),
            "split" => b("split", vec![P::new("by", Ty::Text)], list(Ty::Text)),
            "starts_with" => b("starts_with", vec![P::new("text", Ty::Text)], Ty::Bool),
            "ends_with" => b("ends_with", vec![P::new("text", Ty::Text)], Ty::Bool),
            "find" => b("find", vec![P::new("text", Ty::Text)], opt(INT)),
            "slice" => b("slice", vec![P::new("from", INT), P::new("to", INT)], Ty::Text),
            "lower" => b("lower", vec![], Ty::Text),
            "upper" => b("upper", vec![], Ty::Text),
            "parse_int" => b("parse_int", vec![], opt(Ty::Int(Scalar::I64))),
            "parse_float" => b("parse_float", vec![], opt(Ty::Float(Scalar::F64))),
            // Its UTF-8 (docs/modding/language/language.md).
            "bytes" => b("bytes", vec![], Ty::Bytes),
            _ => return None,
        },
        Ty::Bytes => {
            let read = |n: &'static str, t: Ty| b(n, vec![P::new("at", INT), P::new("big", Ty::Bool)], t);
            let write = |n: &'static str, t: Ty| m(n, vec![P::new("at", INT), P::new("value", t), P::new("big", Ty::Bool)], Ty::Unit);
            match name {
                "len" => b("len", vec![], INT),
                // The bytes as text when they are well-formed UTF-8, else none.
                "text" => b("text", vec![], opt(Ty::Text)),
                "slice" => b("slice", vec![P::new("from", INT), P::new("to", INT)], Ty::Bytes),
                "push" => m("push", vec![P::new("value", Ty::Int(Scalar::U8))], Ty::Unit),
                "read_u16" => read("read_u16", Ty::Int(Scalar::U16)),
                "read_u32" => read("read_u32", Ty::Int(Scalar::U32)),
                "read_u64" => read("read_u64", Ty::Int(Scalar::U64)),
                "read_f32" => read("read_f32", F32),
                "read_f64" => read("read_f64", Ty::Float(Scalar::F64)),
                "write_u16" => write("write_u16", Ty::Int(Scalar::U16)),
                "write_u32" => write("write_u32", Ty::Int(Scalar::U32)),
                "write_u64" => write("write_u64", Ty::Int(Scalar::U64)),
                "write_f32" => write("write_f32", F32),
                "write_f64" => write("write_f64", Ty::Float(Scalar::F64)),
                _ => return None,
            }
        }
        // `int` and `f32` as the language has them (3); a number that does not fit gives none.
        Ty::Json => match name {
            "text" => b("text", vec![], opt(Ty::Text)),
            "int" => b("int", vec![], opt(INT)),
            "float" => b("float", vec![], opt(F32)),
            "bool" => b("bool", vec![], opt(Ty::Bool)),
            "list" => b("list", vec![], opt(list(Ty::Json))),
            "map" => b("map", vec![], opt(Ty::Map(Box::new(Ty::Text), Box::new(Ty::Json)))),
            "get" => b("get", vec![P::new("key", Ty::Text)], opt(Ty::Json)),
            "at" => b("at", vec![P::new("at", INT)], opt(Ty::Json)),
            _ => return None,
        },
        // 6.2: a handle gives a view (or the mod's object), or none once the object is gone.
        Ty::Ref(of) => match name {
            "get" => b("get", vec![], opt((**of).clone())),
            _ => return None,
        },
        Ty::Task => match name {
            "stop" => b("stop", vec![], Ty::Unit),
            _ => return None,
        },
        // 11.1a: a result to come is cancelled by stopping its task.
        Ty::Pending(_) => match name {
            "stop" => b("stop", vec![], Ty::Unit),
            _ => return None,
        },
        _ => return None,
    };
    Some(found)
}

/// Whether a value of type `t` has the builtin method `name` (a method named without a call is no value).
pub(super) fn method_known(t: &Ty, name: &str) -> bool {
    builtin(t, name).is_some()
}

/// A game parameter as a script passes it: the type an argument has, whether it is a `var` argument
/// (a pointer or reference to a number, vector or enum the game writes), whether `none` may be passed.
/// A game type without its `?`.
fn unopt(g: &GameTy) -> &GameTy {
    match g {
        GameTy::Opt(inner) => unopt(inner),
        other => other,
    }
}

/// An expression inside its parentheses.
fn strip_parens(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren(inner) => strip_parens(inner),
        _ => e,
    }
}

/// A text literal with nothing interpolated: its text.
fn literal_text(e: &Expr) -> Option<String> {
    match &strip_parens(e).kind {
        ExprKind::Str(l) if !l.interpolates() => Some(l.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect()),
        _ => None,
    }
}

/// Whether C parameter `ci` of the game's row `row` is `const T &` ('s one place a script copy goes).
pub(crate) fn const_ref_param(row: &str, ci: usize) -> bool {
    use crate::ctype::CType;
    matches!(
        crate::ctype::parse_signature(row).ok().and_then(|s| s.params.into_iter().nth(ci)),
        Some(CType::Ref(to)) if matches!(*to, CType::Named { konst: true, .. })
    )
}

pub(super) fn game_param(g: &GameTy) -> (Ty, bool, bool) {
    match g {
        GameTy::Opt(inner) => {
            let (t, var, _) = game_param(inner);
            (t, var, true)
        }
        GameTy::Place { of, konst } => (from_game_place(of), !konst, false),
        // a marked array, a list (`var xs` when the game writes it).
        GameTy::List { of, konst, .. } => (Ty::List(Box::new(from_game_place(of))), !konst, false),
        GameTy::View { .. } | GameTy::Struct(_) | GameTy::Enum(_) => (Ty::Game(g.clone()), false, false),
        // `none` only: no script value has this type.
        GameTy::Null => (Ty::Game(GameTy::Null), false, true),
        other => (from_game_place(other), false, false),
    }
}

impl<'w, 'a> Walk<'w, 'a> {
    /// `callee(args)`.
    pub(super) fn call(&mut self, e: &'a Expr, callee: &'a Expr, args: &'a [ast::Arg], want: Want<'_>) -> Ty {
        let _ = want;
        match &callee.kind {
            ExprKind::Name { name, targs } => {
                let n = name.name.as_str();
                // Only the standard `low_bits` and `kept` take type arguments (a mod's own name of that spelling does not).
                if !targs.is_empty() && !matches!(self.name_kind(n), NameKind::Std("low_bits" | "kept" | "make_gravity" | "as_class")) {
                    self.type_args_none(callee.at, n, targs.len());
                }
                match self.name_kind(n) {
                    NameKind::Local(id, captured) => {
                        if captured {
                            self.captured.insert(callee.id);
                        }
                        self.res.insert(callee.id, Resolved::Local(id));
                        self.local_read(id, captured, callee.at);
                        let t = self.local(id).ty.clone();
                        self.tys.insert(callee.id, t.clone());
                        self.call_value(e.at, n, &t, args)
                    }
                    NameKind::Item(item) => self.call_item(e, callee, item, args),
                    NameKind::Member(members) => {
                        self.self_read(callee.at);
                        self.call_own_member(e, &members, name, args)
                    }
                    NameKind::Std(s) => self.std_call(e, s, targs, args),
                    NameKind::Game => self.game_named_call(e, n, args),
                    NameKind::Module(_) | NameKind::Type(_) => self.not_callable(e.at, n, args),
                    NameKind::Unknown => {
                        self.unknown_name(callee.at, n);
                        self.args_any(args);
                        Ty::Error
                    }
                }
            }
            ExprKind::Game(gn) => {
                let (name, written) = match gn.text.find('(') {
                    Some(k) if gn.quoted => (gn.text[..k].trim_end(), Some(&gn.text[k..])),
                    _ => (gn.text.as_str(), None),
                };
                self.game_call(e, name, written, None, args)
            }
            ExprKind::Member { base, name, optional, targs } => {
                if !targs.is_empty() && !self.is_json_decode(base, name) {
                    self.type_args_none(name.at, &name.name, targs.len());
                }
                // `base.<name>(...)` in an override's own body, where no local is named `base` (docs/modding/language/language.md, section 5.5).
                if let (ExprKind::Name { name: b, targs: bt }, false) = (&base.kind, *optional) {
                    if b.name == "base" && bt.is_empty() && !matches!(self.name_kind("base"), NameKind::Local(..)) {
                        if let Some(cx) = self.frame().override_of.clone() {
                            return self.base_call(e, callee, &cx, name, args);
                        }
                    }
                }
                self.call_member(e, callee, base, name, *optional, targs, args)
            }
            _ => {
                let t = self.expr(callee, Want::Any);
                self.call_value(e.at, "", &t, args)
            }
        }
    }

    /// `base.<name>(...)`: the base's own version of the virtual this override replaces, with its parameters.
    fn base_call(&mut self, e: &'a Expr, callee: &'a Expr, cx: &super::OverrideCx, name: &'a ast::Ident, args: &'a [ast::Arg]) -> Ty {
        if name.name != cx.name {
            self.say(Diagnostic::new(code::BASE_CALL, name.at).with("found", Arg::Text(name.name.clone())).with("name", Arg::Text(cx.name.clone())));
            self.args_any(args);
            return Ty::Error;
        }
        self.self_read(callee.at);
        let ps: Vec<P> = cx.params.iter().map(|t| P { name: None, ty: t.clone(), var: false, default: false }).collect();
        self.args_against(&format!("base.{}", cx.name), &ps, args, e.at);
        self.res.insert(e.id, Resolved::BaseCall);
        cx.ret.clone().unwrap_or(Ty::Unit)
    }

    fn type_args_none(&mut self, at: Pos, name: &str, n: usize) {
        let d = Diagnostic::new(code::TYPE_ARG_COUNT, at).with("found", Arg::Text(name.to_string())).with("expected", Arg::Text("0".into()));
        let _ = n;
        self.say(d);
    }

    fn is_json_decode(&self, base: &Expr, name: &ast::Ident) -> bool {
        name.name == "decode" && matches!(&base.kind, ExprKind::Name { name: b, .. } if b.name == "json" && matches!(self.name_kind("json"), NameKind::Std(_)))
    }

    /// Each argument checked where anything goes (after an error: its own errors are still said, none for its place).
    pub(super) fn args_any(&mut self, args: &'a [ast::Arg]) {
        for a in args {
            self.expr(&a.value, Want::Ty(&Ty::Error));
        }
    }

    fn not_callable(&mut self, at: Pos, found: &str, args: &'a [ast::Arg]) -> Ty {
        self.say(Diagnostic::new(code::NOT_CALLABLE, at).with("found", Arg::Text(found.to_string())));
        self.args_any(args);
        Ty::Error
    }

    /// A value of function type called (5.4): its arguments by position.
    fn call_value(&mut self, at: Pos, name: &str, t: &Ty, args: &'a [ast::Arg]) -> Ty {
        match t {
            Ty::Fn(ps, r) => {
                let callee = if name.is_empty() { self.ty_text(t) } else { name.to_string() };
                let ps: Vec<P> = ps.iter().map(|p| P { name: None, ty: p.clone(), var: false, default: false }).collect();
                self.args_against(&callee, &ps, args, at);
                (**r).clone()
            }
            Ty::Error | Ty::Never => {
                self.args_any(args);
                Ty::Error
            }
            other => {
                let found = if name.is_empty() { self.ty_text(other) } else { name.to_string() };
                self.not_callable(at, &found, args)
            }
        }
    }

    /// Arguments against parameters: by position first, then by name; each given once; each without a
    /// default given; `var` where the parameter is written (5.1).
    pub(super) fn args_against(&mut self, callee: &str, ps: &[P], args: &'a [ast::Arg], at: Pos) {
        let mut slots: Vec<Option<&'a ast::Arg>> = vec![None; ps.len()];
        let positional = args.iter().filter(|a| a.name.is_none()).count();
        if positional > ps.len() {
            let d = Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(callee.to_string())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text(ps.len().to_string()));
            self.say(d);
        }
        let mut k = 0;
        for a in args {
            match &a.name {
                None => {
                    if k < ps.len() {
                        slots[k] = Some(a);
                    } else {
                        self.expr(&a.value, Want::Ty(&Ty::Error));
                    }
                    k += 1;
                }
                Some(n) => match ps.iter().position(|p| p.name.as_deref() == Some(n.name.as_str())) {
                    Some(i) if slots[i].is_some() => {
                        self.say(Diagnostic::new(code::ARG_TWICE, n.at).with("name", Arg::Text(n.name.clone())));
                        self.expr(&a.value, Want::Ty(&Ty::Error));
                    }
                    Some(i) => slots[i] = Some(a),
                    None => {
                        let d = Diagnostic::new(code::ARG_UNKNOWN, n.at).with("name", Arg::Text(n.name.clone())).with("callee", Arg::Text(callee.to_string()));
                        self.say(d);
                        self.expr(&a.value, Want::Ty(&Ty::Error));
                    }
                },
            }
        }
        for (i, p) in ps.iter().enumerate() {
            match slots[i] {
                Some(a) => self.arg(callee, p, i, a),
                None if !p.default => {
                    let pname = p.name.clone().unwrap_or_else(|| (i + 1).to_string());
                    self.say(Diagnostic::new(code::ARG_MISSING, at).with("name", Arg::Text(pname)).with("callee", Arg::Text(callee.to_string())));
                }
                None => {}
            }
        }
    }

    fn arg(&mut self, callee: &str, p: &P, i: usize, a: &'a ast::Arg) {
        self.expr(&a.value, Want::Ty(&p.ty));
        self.var_arg(callee, p.name.as_deref(), i, p.var, a);
    }

    /// A `var` argument where the parameter is written, and a place that may be written; none elsewhere.
    fn var_arg(&mut self, callee: &str, pname: Option<&str>, i: usize, var: bool, a: &'a ast::Arg) {
        let param = pname.map_or_else(|| (i + 1).to_string(), str::to_string);
        if var && !a.var {
            self.say(Diagnostic::new(code::VAR_ARG_NEEDED, a.at).with("found", Arg::Text(callee.to_string())).with("param", Arg::Text(param)));
        } else if !var && a.var {
            self.say(Diagnostic::new(code::VAR_ARG_NOT_TAKEN, a.at).with("found", Arg::Text(callee.to_string())).with("param", Arg::Text(param)));
        } else if var {
            if let Err(why) = self.place(&a.value) {
                self.say(Diagnostic::new(code::NOT_A_PLACE, a.value.at).with("why", Arg::Phrase(why)));
            }
        }
    }

    /// The parameters of a function of the mod or a library (`key` "" or `.name` for a struct's or class's), without
    /// `self`, and its result.
    fn fn_params(&self, owner: ItemRef, key: &str, params: &[ast::Param], has_ret: bool) -> (Vec<P>, Ty) {
        let ps = params
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.is_self)
            .map(|(i, p)| P { name: p.name.as_ref().map(|n| n.name.clone()), ty: self.declared(owner, &super::param_key(key, p, i)), var: p.var, default: p.default.is_some() })
            .collect();
        let r = if has_ret { self.declared(owner, &format!("{key}->")) } else { Ty::Unit };
        (ps, r)
    }

    /// A call of an item by its name: a function, extern or export; a task (only with `start`); a value of function
    /// type.
    fn call_item(&mut self, e: &'a Expr, callee: &'a Expr, item: ItemRef, args: &'a [ast::Arg]) -> Ty {
        let name = self.c.item_name(item);
        let (ps, r) = match &self.item_of(item).kind {
            ast::ItemKind::Fn(f) => self.fn_params(item, "", &f.params, f.ret.is_some()),
            ast::ItemKind::Extern { params, ret, .. } | ast::ItemKind::Export { params, ret, .. } => self.fn_params(item, "", params, ret.is_some()),
            // A task with a result called: it starts, and the call gives its result to come (11.1a).
            // Without a result its pending gives `()` when the task ends by itself.
            ast::ItemKind::Task { params, ret, .. } => {
                let (ps, r) = self.fn_params(item, "", params, ret.is_some());
                (ps, Ty::Pending(Box::new(r)))
            }
            ast::ItemKind::Binding(_) | ast::ItemKind::Option { .. } => {
                let t = self.expr(callee, Want::Any);
                return self.call_value(e.at, &name, &t, args);
            }
            // C6: `T()`, the struct of its fields' defaults; a field without one is said by name.
            ast::ItemKind::Struct(s) if args.is_empty() => {
                for f in s.fields.iter().filter(|f| f.default.is_none()) {
                    self.say(Diagnostic::new(code::FIELD_MISSING, e.at).with("name", Arg::Text(f.name.name.clone())).with("found", Arg::Text(name.clone())));
                }
                self.res.insert(e.id, Resolved::Item(item));
                return Ty::Struct(item);
            }
            _ => return self.not_callable(e.at, &name, args),
        };
        self.res.insert(e.id, Resolved::Item(item));
        self.res.insert(callee.id, Resolved::Item(item));
        self.args_against(&name, &ps, args, e.at);
        r
    }

    /// A function of the class or game record the body is in, called without `self.` (its `self` is this body's).
    fn call_own_member(&mut self, e: &'a Expr, members: &Members, name: &'a ast::Ident, args: &'a [ast::Arg]) -> Ty {
        let self_ty = self.frames.iter().rev().find_map(|f| f.self_ty.clone()).unwrap_or(Ty::Error);
        match members {
            Members::Mod(class) => {
                let class = *class;
                if let Some(t) = self.class_method(e, class, name, args, None) {
                    return t;
                }
                if self.has_own_field_named(class, &name.name) {
                    let t = self.class_field_ty(class, &name.name, name.at);
                    return self.call_value(e.at, &name.name, &t, args);
                }
                match self.class_base(class) {
                    Some(base) => self.game_method_call(e, &base, false, name, args, Ty::Class(class)),
                    None => self.not_callable(e.at, &name.name, args),
                }
            }
            Members::Game(record) => {
                let konst = matches!(self_ty, Ty::Game(GameTy::View { konst: true, .. }));
                let record = record.clone();
                self.game_method_call(e, &record, konst, name, args, self_ty)
            }
        }
    }

    fn has_own_field_named(&self, class: ItemRef, name: &str) -> bool {
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

    /// A function of the mod's class `class` by name, called on `base` (None: this body's `self`); None when the
    /// class has no function of that name. An override is not one: it is the game's virtual (its parameters the
    /// game's, no `self` written), called as the game's member, which runs the object's own class's function, this
    /// override included (docs/modding/language/objects.md).
    fn class_method(&mut self, e: &'a Expr, class: ItemRef, name: &'a ast::Ident, args: &'a [ast::Arg], base: Option<&'a Expr>) -> Option<Ty> {
        let ast::ItemKind::Class(cd) = &self.item_of(class).kind else { return None };
        let f = cd.members.iter().find_map(|m| match m {
            ast::ClassMember::Fn(f) if f.name.name == name.name => Some(f),
            _ => None,
        })?;
        Some(self.member_fn(e, class, f, name, args, base))
    }

    /// A struct's or class's function called on a value: `self` first, written only from a place that may be.
    fn member_fn(&mut self, e: &'a Expr, owner: ItemRef, f: &'a ast::FnDecl, name: &'a ast::Ident, args: &'a [ast::Arg], base: Option<&'a Expr>) -> Ty {
        let key = format!(".{}", f.name.name);
        let shown = format!("{}.{}", self.c.item_name(owner), f.name.name);
        let Some(self_param) = f.params.iter().find(|p| p.is_self) else {
            self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(self.c.item_name(owner))));
            self.args_any(args);
            return Ty::Error;
        };
        if self_param.var {
            if let Some(b) = base {
                if let Err(why) = self.place(b) {
                    self.say(Diagnostic::new(code::NOT_A_PLACE, b.at).with("why", Arg::Phrase(why)));
                }
            }
        }
        let (ps, r) = self.fn_params(owner, &key, &f.params, f.ret.is_some());
        self.res.insert(e.id, Resolved::Member { owner, name: f.name.name.clone() });
        self.args_against(&shown, &ps, args, e.at);
        r
    }

    /// `base.name(args)`.
    #[allow(clippy::too_many_arguments)]
    fn call_member(&mut self, e: &'a Expr, callee: &'a Expr, base: &'a Expr, name: &'a ast::Ident, optional: bool, targs: &'a [ast::Type], args: &'a [ast::Arg]) -> Ty {
        // `module.Type.name(args)`: as `Type.name(args)` in that module.
        match self.qualified_type(base) {
            Some(Some(item)) => return self.call_on_type(e, item, name, args),
            Some(None) => {
                self.args_any(args);
                return Ty::Error;
            }
            None => {}
        }
        if let ExprKind::Name { name: b, .. } = &base.kind {
            match self.name_kind(&b.name) {
                NameKind::Module(ModTarget::Service(service)) => {
                    // An optional service may not be given: a call needs a test before it (`if x != none`, `guard`).
                    if self.optional_service(&b.name).is_some() && !self.is_given(&b.name) {
                        let found = format!("optional svc.{service}");
                        self.say(Diagnostic::new(code::MAY_BE_NONE, base.at).with("found", Arg::Text(found)));
                        // Said once: the calls after it in this scope are not said again.
                        self.given.push(b.name.clone());
                    }
                    return self.service_call(e, &service, name, args);
                }
                NameKind::Module(target) => {
                    let Some(item) = self.module_item(&target, name) else {
                        self.args_any(args);
                        return Ty::Error;
                    };
                    return self.call_item(e, callee, item, args);
                }
                NameKind::Item(item) if matches!(self.item_of(item).kind, ast::ItemKind::Enum(_) | ast::ItemKind::Struct(_) | ast::ItemKind::Class(_)) => {
                    self.res.insert(base.id, Resolved::Item(item));
                    return self.call_on_type(e, item, name, args);
                }
                NameKind::Std("json") => return self.json_call(e, name, targs, args),
                NameKind::Type(t) => {
                    self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(t.to_string())));
                    self.args_any(args);
                    return Ty::Error;
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
                self.args_any(args);
                return Ty::Error;
            }
            (Ty::Error | Ty::Never, _) => {
                self.args_any(args);
                return Ty::Error;
            }
            (other, true) => {
                let found = self.ty_text(&other);
                self.say(Diagnostic::new(code::UNWRAP_NOT_OPTIONAL, base.at).with("found", Arg::Text(found)));
                other
            }
            (other, false) => other,
        };
        let t = self.method_call(e, callee, base, &inner, name, args);
        match (optional, t) {
            (true, Ty::Opt(x)) => Ty::Opt(x),
            (true, t @ (Ty::Error | Ty::Unit | Ty::Never)) => t,
            (true, t) => Ty::Opt(Box::new(t)),
            (false, t) => t,
        }
    }

    /// `module.Type` (`base`) naming a used module's enum, struct or class: Some(item), resolved on `base`; Some(None)
    /// when it names one that is not `pub` (said); None when `base` is not of that form.
    pub(super) fn qualified_type(&mut self, base: &Expr) -> Option<Option<ItemRef>> {
        let ExprKind::Member { base: inner, name: ty_name, optional: false, .. } = &base.kind else { return None };
        let ExprKind::Name { name: b, .. } = &inner.kind else { return None };
        let NameKind::Module(target @ ModTarget::Module { owner, module }) = self.name_kind(&b.name) else { return None };
        let m = ModRef { owner, module };
        let Some(Name::Item { item, .. }) = self.c.scope(m).names.get(&ty_name.name) else { return None };
        if !matches!(self.item_of(m.item(*item)).kind, ast::ItemKind::Enum(_) | ast::ItemKind::Struct(_) | ast::ItemKind::Class(_)) {
            return None;
        }
        let item = self.module_item(&target, ty_name);
        if let Some(item) = item {
            self.res.insert(base.id, Resolved::Item(item));
        }
        Some(item)
    }

    /// An item of a used module, `pub` (said when not).
    fn module_item(&mut self, target: &ModTarget, name: &ast::Ident) -> Option<ItemRef> {
        let ModTarget::Module { owner, module } = target else { return None };
        let m = ModRef { owner: *owner, module: *module };
        match self.c.scope(m).names.get(&name.name) {
            Some(Name::Item { item, public: true, .. }) => Some(m.item(*item)),
            Some(Name::Item { .. }) => {
                let module = self.module_path(m);
                self.say(Diagnostic::new(code::NAME_NOT_PUB, name.at).with("name", Arg::Text(name.name.clone())).with("module", Arg::Text(module)));
                None
            }
            _ => {
                let module = self.module_path(m);
                self.say(Diagnostic::new(code::NO_NAME_IN_MODULE, name.at).with("name", Arg::Text(name.name.clone())).with("module", Arg::Text(module)));
                None
            }
        }
    }

    /// `Type.name(args)`: an enum's case with fields, or a struct's function without `self`.
    fn call_on_type(&mut self, e: &'a Expr, item: ItemRef, name: &'a ast::Ident, args: &'a [ast::Arg]) -> Ty {
        let type_name = self.c.item_name(item);
        match &self.item_of(item).kind {
            ast::ItemKind::Enum(decl) => {
                let Some(case) = decl.cases.iter().find(|c| c.name.name == name.name) else {
                    self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(type_name)));
                    self.args_any(args);
                    return Ty::Error;
                };
                let shown = format!("{type_name}.{}", case.name.name);
                if case.fields.is_empty() {
                    return self.not_callable(e.at, &shown, args);
                }
                let ps: Vec<P> = case
                    .fields
                    .iter()
                    .map(|f| P { name: Some(f.name.name.clone()), ty: self.declared(item, &format!("{}.{}", case.name.name, f.name.name)), var: false, default: f.default.is_some() })
                    .collect();
                self.res.insert(e.id, Resolved::Case { item, case: case.name.name.clone() });
                self.args_against(&shown, &ps, args, e.at);
                Ty::Enum(item)
            }
            ast::ItemKind::Struct(s) => match s.fns.iter().find(|f| f.name.name == name.name) {
                Some(f) if !f.params.iter().any(|p| p.is_self) => {
                    let key = format!(".{}", f.name.name);
                    let (ps, r) = self.fn_params(item, &key, &f.params, f.ret.is_some());
                    self.res.insert(e.id, Resolved::Member { owner: item, name: f.name.name.clone() });
                    self.args_against(&format!("{type_name}.{}", f.name.name), &ps, args, e.at);
                    r
                }
                _ => {
                    self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(type_name)));
                    self.args_any(args);
                    Ty::Error
                }
            },
            _ => {
                self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(type_name)));
                self.args_any(args);
                Ty::Error
            }
        }
    }

    /// A method of a value: the mod's struct's or class's function, a game record's member function, a builtin, or a
    /// field of function type.
    fn method_call(&mut self, e: &'a Expr, callee: &'a Expr, base: &'a Expr, t: &Ty, name: &'a ast::Ident, args: &'a [ast::Arg]) -> Ty {
        match t {
            Ty::Struct(item) => {
                let item = *item;
                if let ast::ItemKind::Struct(s) = &self.item_of(item).kind {
                    if let Some(f) = s.fns.iter().find(|f| f.name.name == name.name) {
                        return self.member_fn(e, item, f, name, args, Some(base));
                    }
                }
            }
            Ty::Class(class) => {
                let class = *class;
                if let Some(t) = self.class_method(e, class, name, args, Some(base)) {
                    return t;
                }
                if !self.has_own_field_named(class, &name.name) {
                    if let Some(record) = self.class_base(class) {
                        if self.game_method(&record, &name.name).is_some() {
                            return self.game_method_call(e, &record, false, name, args, t.clone());
                        }
                    }
                }
            }
            Ty::Game(GameTy::View { record, konst }) => {
                if self.game_method(record, &name.name).is_some() {
                    let (record, konst) = (record.clone(), *konst);
                    return self.game_method_call(e, &record, konst, name, args, t.clone());
                }
            }
            Ty::Game(GameTy::Struct(record)) => {
                if self.game_method(record, &name.name).is_some() {
                    let record = record.clone();
                    return self.game_method_call(e, &record, false, name, args, t.clone());
                }
            }
            other => {
                if let Some(b) = builtin(other, &name.name) {
                    return self.builtin_call(e, base, other, b, args);
                }
            }
        }
        // A field of function type, else no such member.
        let copy = !matches!(self.tys.get(&base.id), Some(Ty::Game(GameTy::View { .. })) | None) && matches!(t, Ty::Game(GameTy::Struct(_)));
        let ft = self.member_of(callee, t, name, copy);
        self.tys.insert(callee.id, ft.clone());
        match ft {
            Ty::Error => {
                self.args_any(args);
                Ty::Error
            }
            ft => self.call_value(e.at, &name.name, &ft, args),
        }
    }

    /// A builtin method (language.md 6.2, 11.1).
    fn builtin_call(&mut self, e: &'a Expr, base: &'a Expr, t: &Ty, b: Builtin, args: &'a [ast::Arg]) -> Ty {
        self.res.insert(e.id, Resolved::Method(b.name));
        let shown = format!("{}.{}", self.ty_text(t), b.name);
        if b.mutates {
            if let Err(why) = self.place(base) {
                self.say(Diagnostic::new(code::NOT_A_PLACE, base.at).with("why", Arg::Phrase(why)));
            }
        }
        if let Some(need) = b.needs {
            let elem = match t {
                Ty::List(x) => (**x).clone(),
                _ => Ty::Error,
            };
            let ok = match need {
                "==" => self.equatable(&elem),
                _ => matches!(elem, Ty::Int(_) | Ty::Float(_) | Ty::Text | Ty::Error),
            };
            if !ok {
                let found = self.ty_text(&elem);
                let c = if need == "==" { code::NOT_EQUATABLE } else { code::NOT_ORDERED };
                self.say(Diagnostic::new(c, e.at).with("found", Arg::Text(found)));
            }
        }
        let Some((kind, elem)) = b.closure else {
            self.args_against(&shown, &b.params, args, e.at);
            return b.ret;
        };
        if args.len() != 1 || args[0].name.is_some() {
            let d = Diagnostic::new(code::ARG_COUNT, e.at).with("name", Arg::Text(shown)).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text("1".into()));
            self.say(d);
            self.args_any(args);
            return match kind {
                Closure::Map | Closure::Sum => Ty::Error,
                _ => b.ret,
            };
        }
        let a = &args[0];
        // A closure written here runs at once, inside this statement: it may read the views around it (S0103).
        if matches!(a.value.kind, ExprKind::Closure { .. }) {
            self.next_runs_now = true;
        }
        let ft = self.expr(&a.value, Want::Any);
        self.next_runs_now = false;
        let ps = if kind == Closure::Order { vec![elem.clone(), elem.clone()] } else { vec![elem.clone()] };
        let (fps, fr) = match &ft {
            Ty::Fn(fps, fr) => (fps.clone(), (**fr).clone()),
            Ty::Error | Ty::Never => return if matches!(kind, Closure::Map | Closure::Sum) { Ty::Error } else { b.ret },
            _ => (Vec::new(), Ty::Error),
        };
        let params_ok = matches!(ft, Ty::Fn(..)) && fps.len() == ps.len() && fps.iter().zip(ps.iter()).all(|(f, p)| self.fits(p, f));
        let (want_ret, ret) = match kind {
            Closure::Test | Closure::Order => (Ty::Bool, b.ret),
            Closure::Map => (fr.clone(), list(fr.clone())),
            Closure::Sum => (fr.clone(), fr.clone()),
        };
        let ret_ok = match kind {
            Closure::Sum => matches!(fr, Ty::Int(_) | Ty::Float(_) | Ty::Error),
            Closure::Map => !matches!(fr, Ty::Unit),
            _ => self.fits(&fr, &Ty::Bool),
        };
        if !params_ok || !ret_ok {
            let wanted = Ty::Fn(ps, Box::new(if kind == Closure::Sum && !ret_ok { Ty::Int(Scalar::I32) } else { want_ret }));
            let (f, w) = (self.ty_text(&ft), self.ty_text(&wanted));
            self.say(Diagnostic::new(code::TYPE_MISMATCH, a.value.at).with("found", Arg::Text(f)).with("expected", Arg::Text(w)));
            return if matches!(kind, Closure::Map | Closure::Sum) { Ty::Error } else { ret };
        }
        ret
    }

    // ------------------------------------------------------------ standard names

    /// Exactly `n` arguments by position (16a's functions name none).
    pub(super) fn std_args(&mut self, at: Pos, name: &str, args: &'a [ast::Arg], n: &[usize]) -> bool {
        let mut ok = true;
        for a in args {
            if let Some(nm) = &a.name {
                self.say(Diagnostic::new(code::ARG_UNKNOWN, nm.at).with("name", Arg::Text(nm.name.clone())).with("callee", Arg::Text(name.to_string())));
                ok = false;
            }
            if a.var {
                self.say(Diagnostic::new(code::VAR_ARG_NOT_TAKEN, a.at).with("found", Arg::Text(name.to_string())).with("param", Arg::Text("1".into())));
            }
        }
        if !n.contains(&args.len()) {
            let expected = n.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
            self.say(Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(name.to_string())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text(expected)));
            ok = false;
        }
        if !ok {
            self.args_any(args);
        }
        ok
    }

    /// The arguments' shared type (literals adapting to the first that is no literal's).
    fn std_unify(&mut self, args: &'a [ast::Arg], at: Pos) -> Ty {
        self.unify(args.iter().map(|a| &a.value).collect(), at)
    }

    fn std_refuse(&mut self, at: Pos, name: &str, t: &Ty) -> Ty {
        if !matches!(t, Ty::Error | Ty::Never) {
            let found = self.ty_text(t);
            self.say(Diagnostic::new(code::OPERATOR_ONE, at).with("op", Arg::Text(name.to_string())).with("found", Arg::Text(found)));
        }
        Ty::Error
    }

    /// A standard function (16a).
    fn std_call(&mut self, e: &'a Expr, s: &'static str, targs: &'a [ast::Type], args: &'a [ast::Arg]) -> Ty {
        use Value::{Color, Quat, Vec2, Vec3, Vec4};
        let at = e.at;
        match s {
            // Its argument is a state's name, not a value.
            "is_state" => return self.is_state(e, args),
            // D11.5 (reload G-RL5).
            "keep" => return self.keep_call(e, args),
            "kept" => return self.kept_call(e, targs, args),
            // Its table and field names are read at build time to lay the bytes out.
            "set_table" | "table" => return self.table_call(e, s, args),
            "as_class" => return self.as_class_call(e, targs, args),
            _ => {}
        }
        let counts: &[usize] = match s {
            "caller" => &[0],
            "make" => &[2, 3],
            "alias_class" => &[3],
            "set_number" => &[2, 4],
            "placement_of" | "placed_at" | "make_gravity" => &[1],
            "model_named" | "effect_named" | "sound_named" | "layout_named" | "font_named" | "movie_named" => &[1],
            "abs" | "sqrt" | "sin" | "cos" | "length" | "normal" | "hash32" | "low_bits" => &[1],
            "min" | "max" | "atan2" | "dot" | "cross" | "same" | "vec2" => &[2],
            "clamp" | "lerp" | "vec3" | "mat34" => &[3],
            "vec4" | "quat" => &[4],
            "color" => &[3, 4],
            _ => {
                // `json` itself.
                return self.not_callable(at, s, args);
            }
        };
        if !self.std_args(at, s, args, counts) {
            return Ty::Error;
        }
        self.res.insert(e.id, Resolved::Std(s));
        let vector = |t: &Ty| matches!(t, Ty::Value(Vec2 | Vec3 | Vec4));
        match s {
            "min" | "max" | "clamp" => {
                let t = self.std_unify(args, at);
                if matches!(t, Ty::Int(_) | Ty::Float(_)) {
                    t
                } else {
                    self.std_refuse(at, s, &t)
                }
            }
            "abs" => {
                let t = self.std_unify(args, at);
                match t {
                    Ty::Int(Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::I64) | Ty::Float(_) => t,
                    other => self.std_refuse(at, s, &other),
                }
            }
            "sqrt" | "sin" | "cos" | "atan2" => {
                let t = if args.iter().all(|a| shape(&a.value).is_some()) {
                    for a in args {
                        self.expr(&a.value, Want::Ty(&F32));
                    }
                    F32
                } else {
                    self.std_unify(args, at)
                };
                if matches!(t, Ty::Float(_)) {
                    t
                } else {
                    self.std_refuse(at, s, &t)
                }
            }
            "lerp" => {
                let ends: Vec<&'a Expr> = args[..2].iter().map(|a| &a.value).collect();
                let t = if ends.iter().all(|x| shape(x).is_some()) {
                    for x in &ends {
                        self.expr(x, Want::Ty(&F32));
                    }
                    F32
                } else {
                    self.unify(ends, at)
                };
                let by = match &t {
                    Ty::Float(f) => Ty::Float(*f),
                    t if vector(t) || *t == Ty::Value(Color) => F32,
                    other => {
                        self.expr(&args[2].value, Want::Any);
                        return self.std_refuse(at, s, &other.clone());
                    }
                };
                self.expr(&args[2].value, Want::Ty(&by));
                t
            }
            "dot" | "cross" | "length" | "normal" => {
                let t = self.std_unify(args, at);
                let ok = match s {
                    "cross" => t == Ty::Value(Vec3),
                    _ => vector(&t),
                };
                if !ok {
                    return self.std_refuse(at, s, &t);
                }
                match s {
                    "dot" | "length" => F32,
                    _ => t,
                }
            }
            "hash32" => {
                self.expr(&args[0].value, Want::Ty(&Ty::Text));
                Ty::Int(Scalar::U32)
            }
            // The mod a library's export was called by (docs/modding/language/compiled-form.md): a number that
            // stays the same while that mod is on and that `on mods.stopped` gives when it goes; outside an export, this mod.
            "caller" => Ty::Int(Scalar::U64),
            // C2: content by a name made at run time; none for an empty text (no file is looked at: a missing one is
            // the game's placeholder, as a literal's would be past part f).
            "model_named" | "effect_named" | "sound_named" | "layout_named" | "font_named" | "movie_named" => {
                self.expr(&args[0].value, Want::Ty(&Ty::Text));
                Ty::Opt(Box::new(Ty::Content(s.trim_end_matches("_named").to_string())))
            }
            // The docs/modding/language/classes.md's make: `count` objects by a name the factory makes, each at the place (the std
            // module's MakePlace); their LiveActor parts, in the order made. A literal name is one the factory makes: a
            // row of the game's (docs/modding/language/game-description.md, section 9) or a mod class's name; with no placement facts the kernel says it.
            "make" => {
                self.expr(&args[0].value, Want::Ty(&Ty::Text));
                if let ExprKind::Str(l) = &strip_parens(&args[0].value).kind {
                    if !l.interpolates() && self.game().factory.is_some() {
                        let name: String = l.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect();
                        if self.game().factory_row(&name).is_none() && !self.class_named(&name) {
                            self.say(Diagnostic::new(code::CLASS_NAME_UNKNOWN, args[0].value.at).with("found", Arg::Text(name)));
                        }
                    }
                }
                match self.c.std_item("MakePlace") {
                    Some(place) => self.expr(&args[1].value, Want::Ty(&Ty::Struct(place))),
                    // No standard module: the literal says its name is unknown.
                    None => self.expr(&args[1].value, Want::Any),
                };
                if let Some(count) = args.get(2) {
                    self.expr(&count.value, Want::Ty(&INT));
                }
                Ty::List(Box::new(Ty::Game(GameTy::View { record: "LiveActor".into(), konst: false })))
            }
            // The docs/modding/language/classes.md's alias_class: a stage that places `name`, and make by it, make an object of the
            // live mod class `class` (any mod's), its placement arguments from Obj_arg0 `args` (at most 8; the rest,
            // and any -1, the placement's own). The alias is the calling mod's (caller()). true: it stands; the kernel
            // says every refusal with the name.
            "alias_class" => {
                self.expr(&args[0].value, Want::Ty(&Ty::Text));
                self.expr(&args[1].value, Want::Ty(&Ty::Text));
                self.expr(&args[2].value, Want::Ty(&Ty::List(Box::new(INT))));
                Ty::Bool
            }
            // The docs/modding/language/numbers.md's set_number: a number in the game's code set for the calling mod, put
            // back when it goes. `set_number(name, value)` a file's constant ("File.cpp#name"); `set_number(name,
            // signature, place, value)` the place-th number written in a function. true: set; false: no such number,
            // several, or a value not of its kind (the kernel says which in the mod log).
            "set_number" => {
                self.expr(&args[0].value, Want::Ty(&Ty::Text));
                if args.len() == 4 {
                    self.expr(&args[1].value, Want::Ty(&Ty::Text));
                    self.expr(&args[2].value, Want::Ty(&Ty::Int(Scalar::U32)));
                }
                self.expr(&args[args.len() - 1].value, Want::Ty(&Ty::Float(Scalar::F64)));
                Ty::Bool
            }
            // The docs/modding/language/classes.md's placement: where a stage placed the object (the std module's Placement), none for
            // one made by code or by make, or with no scene.
            "placement_of" => {
                self.expr(&args[0].value, Want::Ty(&Ty::Game(GameTy::View { record: "LiveActor".into(), konst: true })));
                match self.c.std_item("Placement") {
                    Some(p) => Ty::Opt(Box::new(Ty::Struct(p))),
                    None => Ty::Error,
                }
            }
            // The object the scene made from a placement (its stage empty or the main stage's name), none when there
            // is none (yet).
            "placed_at" => {
                match self.c.std_item("Placement") {
                    Some(p) => self.expr(&args[0].value, Want::Ty(&Ty::Struct(p))),
                    None => self.expr(&args[0].value, Want::Any),
                };
                Ty::Opt(Box::new(Ty::Game(GameTy::View { record: "LiveActor".into(), konst: false })))
            }
            // The docs/modding/language/classes.md's gravity: `make_gravity<C>(place)` an object of the mod's gravity class C (base
            // PlanetGravity) registered with the scene's gravities; none when the kernel makes none (said).
            "make_gravity" => {
                let class = match targs {
                    [t] => {
                        let m = self.m;
                        Some(self.c.resolve(m, t))
                    }
                    _ => {
                        let d = Diagnostic::new(code::TYPE_ARG_COUNT, at).with("found", Arg::Text("make_gravity".into())).with("expected", Arg::Text("1".into()));
                        self.say(d);
                        None
                    }
                };
                match self.c.std_item("GravityPlace") {
                    Some(p) => self.expr(&args[0].value, Want::Ty(&Ty::Struct(p))),
                    None => self.expr(&args[0].value, Want::Any),
                };
                match class {
                    Some(Ty::Class(item)) if self.class_base(item).as_deref() == Some("PlanetGravity") => Ty::Opt(Box::new(Ty::Class(item))),
                    Some(Ty::Error) | None => Ty::Error,
                    Some(other) => {
                        let found = self.ty_text(&other);
                        self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(found)).with("expected", Arg::Text("PlanetGravity".into())));
                        Ty::Error
                    }
                }
            }
            // C4: a matrix by its three rows (each x, y, z, w; w the translation).
            "mat34" => {
                for a in args {
                    self.expr(&a.value, Want::Ty(&Ty::Value(Vec4)));
                }
                Ty::Value(Value::Mat34)
            }
            "same" => {
                let a = self.expr(&args[0].value, Want::Any);
                let b = self.expr(&args[1].value, Want::Any);
                if !self.sameable(&a, &b) && !self.sameable(&b, &a) {
                    let (l, r) = (self.ty_text(&a), self.ty_text(&b));
                    self.say(Diagnostic::new(code::OPERATOR, at).with("op", Arg::Text("same".into())).with("left", Arg::Text(l)).with("right", Arg::Text(r)));
                }
                Ty::Bool
            }
            "low_bits" => {
                let target = match targs {
                    [t] => {
                        let m = self.m;
                        self.c.resolve(m, t)
                    }
                    _ => {
                        let d = Diagnostic::new(code::TYPE_ARG_COUNT, at).with("found", Arg::Text("low_bits".into())).with("expected", Arg::Text("1".into()));
                        self.say(d);
                        self.expr(&args[0].value, Want::Any);
                        return Ty::Error;
                    }
                };
                let n = if shape(&args[0].value).is_some() { self.expr(&args[0].value, Want::Ty(&Ty::Int(Scalar::I64))) } else { self.expr(&args[0].value, Want::Any) };
                if !matches!(n, Ty::Int(_) | Ty::Error | Ty::Never) {
                    self.std_refuse(args[0].value.at, s, &n);
                }
                match target {
                    Ty::Int(_) | Ty::Error => target,
                    other => self.std_refuse(at, s, &other),
                }
            }
            _ => {
                for a in args {
                    self.expr(&a.value, Want::Ty(&F32));
                }
                match s {
                    "vec2" => Ty::Value(Vec2),
                    "vec3" => Ty::Value(Vec3),
                    "vec4" => Ty::Value(Vec4),
                    "quat" => Ty::Value(Quat),
                    _ => Ty::Value(Color),
                }
            }
        }
    }

    /// The docs/modding/language/numbers.md's tables: `set_table(name, steps..., value)` writes one place of a table of
    /// the game's data for the calling mod (put back when it goes), `table(name, steps...)` reads it (`T?`, none when
    /// the game has no such table). The name and each field are text written in the call; each step after the name is
    /// a field of a record by name or an element of an array by index (from 0; a literal is checked here, any other
    /// at run time, S0902). The place reached is a number, a game enum or a value (vector, quaternion, matrix,
    /// colour), whose type the value takes. A table the game does not list is S0150; a field it does not have,
    /// S0084; a step that does not fit or a place that is no number (a pointer, a bit-field), S0151; a literal index
    /// outside, S0152.
    fn table_call(&mut self, e: &'a Expr, s: &'static str, args: &'a [ast::Arg]) -> Ty {
        let write = s == "set_table";
        let least = if write { 2 } else { 1 };
        let mut ok = true;
        for a in args {
            if let Some(nm) = &a.name {
                self.say(Diagnostic::new(code::ARG_UNKNOWN, nm.at).with("name", Arg::Text(nm.name.clone())).with("callee", Arg::Text(s.to_string())));
                ok = false;
            }
            if a.var {
                self.say(Diagnostic::new(code::VAR_ARG_NOT_TAKEN, a.at).with("found", Arg::Text(s.to_string())).with("param", Arg::Text("1".into())));
            }
        }
        if args.len() < least {
            let d = Diagnostic::new(code::ARG_COUNT, e.at).with("name", Arg::Text(s.to_string())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text(format!("{least}+")));
            self.say(d);
            ok = false;
        }
        if !ok {
            self.args_any(args);
            return Ty::Error;
        }
        let steps_end = if write { args.len() - 1 } else { args.len() };
        let refuse = |w: &mut Self, from: usize| {
            w.args_any(&args[from..]);
            Ty::Error
        };
        let Some(name) = literal_text(&args[0].value) else {
            self.expr(&args[0].value, Want::Ty(&Ty::Text));
            self.say(Diagnostic::new(code::TABLE_PLACE, args[0].value.at).with("found", Arg::Text(s.to_string())).with("why", Arg::Phrase("Script.Table.NotLiteral")));
            return refuse(self, 1);
        };
        self.expr(&args[0].value, Want::Ty(&Ty::Text));
        let Some(table) = self.game().tables.as_ref().and_then(|t| t.get(&name)).cloned() else {
            self.say(Diagnostic::new(code::TABLE_UNKNOWN, args[0].value.at).with("found", Arg::Text(name)));
            return refuse(self, 1);
        };
        let mut shown = name.clone();
        let mut cur = match self.game().table_element(&table.ty) {
            Ok(p) if table.count > 1 => TablePart::Array { of: Box::new(p), extents: vec![table.count] },
            Ok(p) => p,
            Err(why) => {
                self.say(Diagnostic::new(code::TABLE_PLACE, args[0].value.at).with("found", Arg::Text(shown)).with("why", Arg::Phrase(why)));
                return refuse(self, 1);
            }
        };
        let mut offset = 0u64;
        let mut indexes = Vec::new();
        for (i, a) in args.iter().enumerate().take(steps_end).skip(1) {
            let at = a.value.at;
            let place_refused = |w: &mut Self, shown: &str, why: &'static str| {
                w.say(Diagnostic::new(code::TABLE_PLACE, at).with("found", Arg::Text(shown.to_string())).with("why", Arg::Phrase(why)));
                refuse(w, i + 1)
            };
            if let Some(field) = literal_text(&a.value) {
                self.expr(&a.value, Want::Ty(&Ty::Text));
                let TablePart::Record { name: record, .. } = &cur else {
                    let why = if matches!(cur, TablePart::Array { .. }) { "Script.Table.NeedsIndex" } else { "Script.Table.NotRecord" };
                    return place_refused(self, &shown, why);
                };
                match self.game().table_field(record, &field) {
                    None => {
                        let record = record.clone();
                        self.say(Diagnostic::new(code::NO_MEMBER, at).with("name", Arg::Text(field)).with("found", Arg::Text(record)));
                        return refuse(self, i + 1);
                    }
                    Some((at_offset, part)) => {
                        shown = format!("{shown}.{field}");
                        match part {
                            Ok(p) => {
                                offset += at_offset;
                                cur = p;
                            }
                            Err(why) => return place_refused(self, &shown, why),
                        }
                    }
                }
                continue;
            }
            self.expr(&a.value, Want::Ty(&INT));
            let TablePart::Array { of, extents } = &cur else {
                let why = if matches!(cur, TablePart::Record { .. }) { "Script.Table.NeedsField" } else { "Script.Table.NotArray" };
                return place_refused(self, &shown, why);
            };
            let count = extents[0];
            let inner = if extents.len() > 1 { TablePart::Array { of: of.clone(), extents: extents[1..].to_vec() } } else { (**of).clone() };
            let stride = inner.size();
            match super::expr::literal_whole(&a.value) {
                Some(n) if n < 0 || n >= i128::from(count) => {
                    self.say(Diagnostic::new(code::TABLE_INDEX, at).with("found", Arg::Text(n.to_string())).with("count", Arg::Text(count.to_string())));
                    return refuse(self, i + 1);
                }
                Some(n) => {
                    offset += n as u64 * stride;
                    shown = format!("{shown}[{n}]");
                }
                None => {
                    indexes.push((i, stride, count));
                    shown = format!("{shown}[]");
                }
            }
            cur = inner;
        }
        let leaf = match cur {
            TablePart::Leaf(l) => l,
            other => {
                let why = if matches!(other, TablePart::Array { .. }) { "Script.Table.NeedsIndex" } else { "Script.Table.NeedsField" };
                let at = args[steps_end - 1].value.at;
                self.say(Diagnostic::new(code::TABLE_PLACE, at).with("found", Arg::Text(shown)).with("why", Arg::Phrase(why)));
                return refuse(self, steps_end);
            }
        };
        let ty = match &leaf {
            TableLeaf::Scalar(sc) => Ty::from_game(GameTy::Scalar(*sc)),
            TableLeaf::Enum(name, _) => Ty::Game(GameTy::Enum(name.clone())),
            TableLeaf::Value(v) => Ty::Value(*v),
        };
        if write {
            self.expr(&args[steps_end].value, Want::Ty(&ty));
        }
        self.res.insert(e.id, Resolved::Table { write, name, offset, indexes, leaf });
        if write {
            Ty::Bool
        } else {
            Ty::Opt(Box::new(ty))
        }
    }

    /// The docs/modding/language/objects.md's downcast: `as_class<T>(x)`, a view (or an optional view) of a game object
    /// seen as the game class T or the mod's class T, of the same line of classes as x's record; `T?`, none when the
    /// object is not a T (the kernel's as_class at run time) or x is none. An upcast needs no call.
    fn as_class_call(&mut self, e: &'a Expr, targs: &'a [ast::Type], args: &'a [ast::Arg]) -> Ty {
        let at = e.at;
        let target = match targs {
            [t] => {
                let m = self.m;
                self.c.resolve(m, t)
            }
            _ => {
                let d = Diagnostic::new(code::TYPE_ARG_COUNT, at).with("found", Arg::Text("as_class".into())).with("expected", Arg::Text("1".into()));
                self.say(d);
                self.args_any(args);
                return Ty::Error;
            }
        };
        if !self.std_args(at, "as_class", args, &[1]) {
            return Ty::Error;
        }
        let t = self.expr(&args[0].value, Want::Any);
        let from = match &t {
            Ty::Opt(x) => (**x).clone(),
            x => x.clone(),
        };
        let related = |w: &Self, a: &str, b: &str| w.derives(a, b) || w.derives(b, a);
        let fits = match (&from, &target) {
            (Ty::Error | Ty::Never, _) | (_, Ty::Error) => true,
            (Ty::Game(GameTy::View { record: a, .. }), Ty::Game(GameTy::View { record: b, .. })) => related(self, a, b),
            (Ty::Game(GameTy::View { record: a, .. }), Ty::Class(c)) => self.class_base(*c).is_some_and(|b| related(self, a, &b)),
            (Ty::Class(c), Ty::Game(GameTy::View { record: b, .. })) => self.class_base(*c).is_some_and(|a| related(self, &a, b)),
            (Ty::Class(x), Ty::Class(y)) => x == y,
            _ => false,
        };
        if !fits {
            let (f, w) = (self.ty_text(&t), self.ty_text(&target));
            self.say(Diagnostic::new(code::AS_CLASS, at).with("found", Arg::Text(f)).with("expected", Arg::Text(w)));
            return Ty::Error;
        }
        self.res.insert(e.id, Resolved::Std("as_class"));
        match target {
            Ty::Error => Ty::Error,
            Ty::Game(GameTy::View { record, .. }) => Ty::Opt(Box::new(Ty::Game(GameTy::View { record, konst: matches!(from, Ty::Game(GameTy::View { konst: true, .. })) }))),
            target => Ty::Opt(Box::new(target)),
        }
    }

    /// Whether a class of the mod or of a library it uses is named `name` (the factory makes it by that name once
    /// defined; case is ignored, as the factory ignores it).
    fn class_named(&self, name: &str) -> bool {
        let named = |m: &crate::check::Source| m.module.items.iter().any(|i| matches!(&i.kind, ast::ItemKind::Class(c) if c.name.name.eq_ignore_ascii_case(name)));
        self.c.modules.iter().any(named) || self.c.libraries.iter().any(|l| l.modules.iter().any(named))
    }

    /// Whether `same(a, b)` compares these: views of related records, the mod's object and its base's view,
    /// a nerve by name and the game's `const Nerve *`, handles of one service type.
    fn sameable(&self, a: &Ty, b: &Ty) -> bool {
        let strip = |t: &Ty| match t {
            Ty::Opt(x) => (**x).clone(),
            other => other.clone(),
        };
        let (a, b) = (strip(a), strip(b));
        match (&a, &b) {
            (Ty::Error | Ty::Never, _) | (_, Ty::Error | Ty::Never) => true,
            (Ty::Game(GameTy::View { record: x, .. }), Ty::Game(GameTy::View { record: y, .. })) => self.derives(x, y) || self.derives(y, x),
            (Ty::Class(c), Ty::Game(GameTy::View { record, .. })) => self.class_base(*c).is_some_and(|base| self.derives(&base, record) || self.derives(record, &base)),
            (Ty::Class(x), Ty::Class(y)) => x == y,
            (Ty::Content(w), Ty::Game(GameTy::View { record, .. })) => w == "nerve" && record == "Nerve",
            (Ty::Content(x), Ty::Content(y)) => x == "nerve" && y == "nerve",
            (Ty::Service { .. }, Ty::Service { .. }) => a == b,
            _ => false,
        }
    }

    /// `json.encode(v)`, `json.decode<T>(text)` (16a).
    fn json_call(&mut self, e: &'a Expr, name: &'a ast::Ident, targs: &'a [ast::Type], args: &'a [ast::Arg]) -> Ty {
        let at = e.at;
        match name.name.as_str() {
            "encode" => {
                if !self.std_args(at, "json.encode", args, &[1]) {
                    return Ty::Error;
                }
                self.res.insert(e.id, Resolved::Std("json.encode"));
                // A list or map written here is a json.Value, each value by its kind.
                let written = matches!(args[0].value.kind, ExprKind::List(_) | ExprKind::Braces(_));
                let t = self.expr(&args[0].value, if written { Want::Ty(&Ty::Json) } else { Want::Any });
                if !self.json_data(&t) {
                    let found = self.ty_text(&t);
                    self.say(Diagnostic::new(code::NOT_JSON_DATA, args[0].value.at).with("found", Arg::Text(found)));
                }
                Ty::Text
            }
            "decode" => {
                let target = match targs {
                    [t] => {
                        let m = self.m;
                        Some(self.c.resolve(m, t))
                    }
                    _ => {
                        let d = Diagnostic::new(code::TYPE_ARG_COUNT, name.at).with("found", Arg::Text("json.decode".into())).with("expected", Arg::Text("1".into()));
                        self.say(d);
                        None
                    }
                };
                if !self.std_args(at, "json.decode", args, &[1]) {
                    return Ty::Error;
                }
                self.res.insert(e.id, Resolved::Std("json.decode"));
                // Text, or bytes read as UTF-8 (`data "f.json"`).
                let from = self.expr(&args[0].value, Want::Any);
                if !matches!(from, Ty::Text | Ty::Bytes | Ty::Error | Ty::Never) {
                    let found = self.ty_text(&from);
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, args[0].value.at).with("found", Arg::Text(found)).with("expected", Arg::Text("text".into())));
                }
                match target {
                    Some(t) if self.json_data(&t) => opt(t),
                    Some(t) => {
                        let found = self.ty_text(&t);
                        self.say(Diagnostic::new(code::NOT_JSON_DATA, name.at).with("found", Arg::Text(found)));
                        Ty::Error
                    }
                    None => Ty::Error,
                }
            }
            "Value" => self.not_callable(at, "json.Value", args),
            _ => {
                self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text("json".into())));
                self.args_any(args);
                Ty::Error
            }
        }
    }

    /// Whether json reads and writes values of `t`: numbers, `bool`, text, vectors and colours (as lists), `json.Value`,
    /// containers, tuples, options, the mod's structs and enums of those, game enums and the script's own copies.
    pub(super) fn json_data(&self, t: &Ty) -> bool {
        self.json_data_in(t, &mut Vec::new())
    }

    fn json_data_in(&self, t: &Ty, seen: &mut Vec<ItemRef>) -> bool {
        match t {
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Value(_) | Ty::Text | Ty::Json | Ty::Error => true,
            // A game struct by value is not: its fields may be pointers, and none is looked into here.
            Ty::Game(GameTy::Enum(_)) => true,
            Ty::List(x) | Ty::Set(x) | Ty::Opt(x) => self.json_data_in(x, seen),
            Ty::Map(k, v) => matches!(**k, Ty::Text | Ty::Int(_) | Ty::Bool | Ty::Enum(_) | Ty::Game(GameTy::Enum(_))) && self.json_data_in(v, seen),
            Ty::Tuple(items) => items.iter().all(|i| self.json_data_in(i, seen)),
            Ty::Struct(item) | Ty::Enum(item) => {
                if seen.contains(item) {
                    return true;
                }
                seen.push(*item);
                let parts: Vec<Ty> = match &self.item_of(*item).kind {
                    ast::ItemKind::Struct(s) => s.fields.iter().map(|f| self.declared(*item, &format!(".{}", f.name.name))).collect(),
                    ast::ItemKind::Enum(d) => d.cases.iter().flat_map(|c| c.fields.iter().map(move |f| (c, f))).map(|(c, f)| self.declared(*item, &format!("{}.{}", c.name.name, f.name.name))).collect(),
                    _ => Vec::new(),
                };
                parts.iter().all(|p| self.json_data_in(p, seen))
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------ game functions

    /// A game name written bare and called: a function (a value of one is no function).
    fn game_named_call(&mut self, e: &'a Expr, name: &str, args: &'a [ast::Arg]) -> Ty {
        if self.game().functions.contains_key(name) {
            return self.game_call(e, name, None, None, args);
        }
        self.not_callable(e.at, name, args)
    }

    /// A game record's member function called on `receiver` (a view, the mod's object, or the script's copy).
    fn game_method_call(&mut self, e: &'a Expr, record: &str, konst: bool, name: &ast::Ident, args: &'a [ast::Arg], receiver: Ty) -> Ty {
        let Some(full) = self.game_method(record, &name.name) else {
            self.say(Diagnostic::new(code::NO_MEMBER, name.at).with("name", Arg::Text(name.name.clone())).with("found", Arg::Text(record.to_string())));
            self.args_any(args);
            return Ty::Error;
        };
        let receiver = match receiver {
            Ty::Game(GameTy::View { record, .. }) => Ty::Game(GameTy::View { record, konst }),
            other => other,
        };
        self.game_call(e, &full, None, Some(receiver), args)
    }

    /// A game function (7.2): its rows' signatures that cross, the one the arguments choose (by their types; among
    /// several, the one each argument fits exactly; a const receiver's const one), each argument checked against it.
    fn game_call(&mut self, e: &'a Expr, name: &str, written: Option<&str>, receiver: Option<Ty>, args: &'a [ast::Arg]) -> Ty {
        let at = e.at;
        let Some(rows) = self.game().functions.get(name) else {
            self.unknown_name(at, name);
            self.args_any(args);
            return Ty::Error;
        };
        let rows: Vec<&'a str> = rows.iter().filter(|(_, tell)| !tell).map(|(s, _)| s.as_str()).collect();
        if rows.is_empty() {
            return self.not_callable(at, name, args);
        }
        for a in args {
            if let Some(n) = &a.name {
                self.say(Diagnostic::new(code::ARG_NAMED_GAME, n.at).with("name", Arg::Text(n.name.clone())));
            }
        }
        // without the rows' facts (luma_functions.json) a record's function is read as the script writes it (on a
        // value: a member), and said once per name.
        if let (Some((qualifier, _)), None) = (name.rsplit_once("::"), &self.game().facts) {
            let is_record = self.game().types.as_ref().is_some_and(|t| t.record(qualifier).is_some());
            if is_record && self.c.noted.insert(name.to_string()) {
                self.say(Diagnostic::new(code::NOT_DESCRIBED, at).with("found", Arg::Text(name.to_string())).with("what", Arg::Phrase("Script.What.FunctionKinds")));
            }
        }
        let signatures = rows.join("; ");
        let game = self.game();
        let mut sigs: Vec<(&'a str, Result<FnTy, SigNotCrossing>)> = rows.iter().map(|r| (*r, game.call_signature(name, r))).collect();
        if let Some(w) = written {
            let wanted = crate::ctype::parse_signature(&format!("void{w}"));
            sigs.retain(|(r, _)| match (&wanted, crate::ctype::parse_signature(r)) {
                (Ok(a), Ok(b)) => a.params == b.params && a.konst == b.konst,
                _ => false,
            });
            if sigs.is_empty() {
                self.say(Diagnostic::new(code::GAME_NO_SIGNATURE, at).with("found", Arg::Text(format!("{name}{w}"))).with("signatures", Arg::Text(signatures)));
                self.args_any(args);
                return Ty::Error;
            }
        }
        let crossing: Vec<(&'a str, FnTy)> = sigs.iter().filter_map(|(r, s)| s.as_ref().ok().map(|f| (*r, f.clone()))).collect();
        if crossing.is_empty() {
            match sigs.as_slice() {
                [(_, Err(SigNotCrossing { param: Some(p), why }))] => {
                    let d = Diagnostic::new(code::GAME_PARAM_NOT_CROSSING, at).with("found", Arg::Text(name.to_string())).with("param", Arg::Text((p + 1).to_string())).with("why", Arg::Phrase(crate::check::why_phrase(why)));
                    self.say(d);
                }
                // The result's type (a variadic or unreadable text is the whole signature's: NONE_CROSSING below).
                [(_, Err(SigNotCrossing { param: None, why }))] if !matches!(why, NotCrossing::Variadic | NotCrossing::Unreadable(_)) => {
                    self.say(Diagnostic::new(code::GAME_RESULT_NOT_CROSSING, at).with("found", Arg::Text(name.to_string())).with("why", Arg::Phrase(crate::check::why_phrase(why))));
                }
                _ => self.say(Diagnostic::new(code::GAME_NONE_CROSSING, at).with("found", Arg::Text(name.to_string())).with("signatures", Arg::Text(signatures))),
            }
            self.args_any(args);
            return Ty::Error;
        }
        // Each argument's kind before a signature is chosen: a literal's shape, else its own type.
        let probes: Vec<Ty> = args.iter().map(|a| shape(&a.value).unwrap_or_else(|| self.expr(&a.value, Want::Any))).collect();
        let viable: Vec<&(&'a str, FnTy)> = crossing.iter().filter(|(_, f)| f.params.len() == args.len() && f.params.iter().zip(probes.iter()).all(|(p, t)| self.probe_fits(t, p, false))).collect();
        let chosen: Option<(&'a str, FnTy)> = match viable.as_slice() {
            [one] => Some((*one).clone()),
            [] => None,
            many => {
                let exact: Vec<&&(&'a str, FnTy)> = many.iter().filter(|(_, f)| f.params.iter().zip(probes.iter()).all(|(p, t)| self.probe_fits(t, p, true))).collect();
                let exact: Vec<&(&'a str, FnTy)> = if exact.is_empty() { many.to_vec() } else { exact.into_iter().copied().collect() };
                let rkonst = matches!(receiver, Some(Ty::Game(GameTy::View { konst: true, .. })));
                let by_const: Vec<&(&'a str, FnTy)> = if exact.len() > 1 && receiver.is_some() { exact.iter().filter(|(_, f)| f.konst == rkonst).copied().collect() } else { exact.clone() };
                match by_const.as_slice() {
                    [one] => Some((*one).clone()),
                    _ => {
                        let listed = exact.iter().map(|(r, _)| *r).collect::<Vec<_>>().join("; ");
                        self.say(Diagnostic::new(code::GAME_AMBIGUOUS, at).with("found", Arg::Text(name.to_string())).with("signatures", Arg::Text(listed)));
                        self.literal_args_any(args);
                        return Ty::Error;
                    }
                }
            }
        };
        let (row, f) = match chosen {
            Some(c) => c,
            // One signature of this many parameters: its arguments are checked against it, each error said where it is.
            None => match crossing.iter().filter(|(_, f)| f.params.len() == args.len()).collect::<Vec<_>>().as_slice() {
                [one] => (*one).clone(),
                _ if crossing.len() == 1 => {
                    let d = Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(name.to_string())).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text(crossing[0].1.params.len().to_string()));
                    self.say(d);
                    self.literal_args_any(args);
                    return Ty::Error;
                }
                _ => {
                    self.say(Diagnostic::new(code::GAME_NO_SIGNATURE, at).with("found", Arg::Text(name.to_string())).with("signatures", Arg::Text(signatures)));
                    self.literal_args_any(args);
                    return Ty::Error;
                }
            },
        };
        // a member function runs on an object: the one it is called on, else `self` where `self` is of its record
        // (`LiveActor::appear()` in a hook of a class derived from LiveActor: C++'s call of the base's own). A static
        // or free one takes no object, whatever it is written on.
        let mut receiver = receiver;
        if let Some(facts) = self.game().facts.as_ref().and_then(|ff| ff.get(name, row)) {
            match facts.kind {
                FnKind::Method if receiver.is_none() => {
                    let self_ty = self.frames.iter().rev().find_map(|fr| fr.self_ty.clone());
                    let of_owner = match (&self_ty, &facts.owner) {
                        (Some(Ty::Game(GameTy::View { record, .. })), Some(owner)) => self.derives(record, owner),
                        (Some(Ty::Class(c)), Some(owner)) => self.class_base(*c).is_some_and(|b| self.derives(&b, owner)),
                        _ => false,
                    };
                    if of_owner {
                        receiver = self_ty;
                    } else {
                        self.say(Diagnostic::new(code::METHOD_NEEDS_OBJECT, at).with("found", Arg::Text(name.to_string())));
                    }
                }
                FnKind::Free | FnKind::Static => receiver = None,
                _ => {}
            }
        } else if self.game().facts.is_some() && self.c.noted.insert(format!("{name}{row}")) {
            // A row the facts file lacks (gen_types.py stops on one: a file from another build).
            self.say(Diagnostic::new(code::NOT_DESCRIBED, at).with("found", Arg::Text(format!("{name}{row}"))).with("what", Arg::Phrase("Script.What.FunctionKinds")));
        }
        match &receiver {
            Some(Ty::Game(GameTy::View { konst: true, .. })) if !f.konst => {
                self.say(Diagnostic::new(code::CONST_VIEW_CALL, at).with("found", Arg::Text(name.to_string())));
            }
            Some(Ty::Game(GameTy::Struct(r))) => {
                self.say(Diagnostic::new(code::COPY_NOT_VIEW, at).with("found", Arg::Text(r.clone())));
            }
            _ => {}
        }
        for (i, (a, p)) in args.iter().zip(f.params.iter()).enumerate() {
            let (want, var, nullable) = game_param(p);
            match &probes[i] {
                Ty::Open if nullable => {
                    self.expr(&a.value, Want::Ty(&opt(want)));
                    if a.var {
                        self.say(Diagnostic::new(code::VAR_ARG_NOT_TAKEN, a.at).with("found", Arg::Text(name.to_string())).with("param", Arg::Text((f.c_index(i) + 1).to_string())));
                    }
                    continue;
                }
                Ty::IntLit | Ty::FloatLit | Ty::Open => {
                    self.expr(&a.value, Want::Ty(&want));
                }
                // the script's copy of a record may go to a `const T &` of the same record
                // (C++ binds it for the call; through const the game cannot change the script's value). A `T &`, a
                // pointer, or a base class of the copy still takes the game's object only (COPY_NOT_VIEW).
                Ty::Game(GameTy::Struct(r))
                    if matches!(&want, Ty::Game(GameTy::View { record, konst: true }) if record == r) && const_ref_param(row, f.c_index(i)) => {}
                probe => {
                    let probe = probe.clone();
                    self.check_fit(a.value.at, probe, &want);
                }
            }
            // A parameter's number is its place in the row's text ('s counts are not written but counted).
            let ci = f.c_index(i);
            self.var_arg(name, None, ci, var, a);
            // a list literal for an array of a fixed count is that many (any other list is checked when called).
            if let (GameTy::List { fixed: Some(n), .. }, ExprKind::List(items)) = (unopt(p), &strip_parens(&a.value).kind) {
                if items.len() != *n as usize {
                    let d = Diagnostic::new(code::FIXED_COUNT, a.value.at).with("found", Arg::Text(name.to_string())).with("param", Arg::Text((ci + 1).to_string())).with("expected", Arg::Text(n.to_string())).with("count", Arg::Text(items.len().to_string()));
                    self.say(d);
                }
            }
        }
        self.res.insert(e.id, Resolved::GameFn { name: name.to_string(), signature: row.to_string() });
        self.game_result(e, &f.result)
    }

    /// Literal-shaped arguments checked once nothing was chosen (the others were checked as probes).
    fn literal_args_any(&mut self, args: &'a [ast::Arg]) {
        for a in args {
            if let Some(s) = shape(&a.value) {
                let t = match s {
                    Ty::FloatLit => F32,
                    Ty::IntLit => INT,
                    _ => Ty::Error,
                };
                self.expr(&a.value, Want::Ty(&t));
            }
        }
    }

    /// Whether an argument of kind `t` may go to a game parameter `p` (`exact`: as it is, nothing adapted but a
    /// literal to its own kind's type).
    fn probe_fits(&self, t: &Ty, p: &GameTy, exact: bool) -> bool {
        let (want, _, nullable) = game_param(p);
        match t {
            Ty::Open => nullable,
            // Exactly: a literal's own default type, as C++ takes `1` for `int` and `1.5` (f) for `float`.
            Ty::IntLit if exact => want == INT,
            Ty::IntLit => matches!(want, Ty::Int(_) | Ty::Float(_)),
            Ty::FloatLit if exact => want == F32,
            Ty::FloatLit => matches!(want, Ty::Float(_)),
            Ty::Error | Ty::Never => true,
            // A `T?` is said MAY_BE_NONE at the argument once this signature is chosen.
            Ty::Opt(x) if !exact => self.probe_fits(x, p, false),
            // The script's copy where the game's object goes: COPY_NOT_VIEW once chosen (revision 3).
            Ty::Game(GameTy::Struct(r)) if !exact => match &want {
                Ty::Game(GameTy::View { record, .. }) => self.derives(r, record),
                w => self.fits(t, w),
            },
            _ if exact => match (t, &want) {
                (Ty::Game(GameTy::View { record: a, .. }), Ty::Game(GameTy::View { record: b, .. })) => a == b,
                _ => *t == want,
            },
            _ => self.fits(t, &want),
        }
    }

    /// A game function's result as a script reads it: a reference to a number is a place it writes through.
    fn game_result(&mut self, e: &Expr, g: &GameTy) -> Ty {
        match g {
            GameTy::Unit => Ty::Unit,
            GameTy::Place { of, konst } => {
                if !konst {
                    self.game_places.insert(e.id);
                }
                from_game_place(of)
            }
            GameTy::Opt(inner) => {
                if matches!(&**inner, GameTy::Place { konst: false, .. }) {
                    self.game_places.insert(e.id);
                }
                from_game_place(g)
            }
            other => from_game_place(other),
        }
    }

    // ------------------------------------------------------------ start

    /// `start task(args)` (11.1): a task's handle.
    pub(super) fn start_task(&mut self, inner: &'a Expr) -> Ty {
        if let ExprKind::Call { callee, args } = &inner.kind {
            if let Some(item) = self.task_target(callee) {
                let ast::ItemKind::Task { params, .. } = &self.item_of(item).kind else { unreachable!("task_target gives tasks") };
                let name = self.c.item_name(item);
                let (ps, _) = self.fn_params(item, "", params, false);
                self.res.insert(inner.id, Resolved::Item(item));
                self.res.insert(callee.id, Resolved::Item(item));
                self.args_against(&name, &ps, args, inner.at);
                self.tys.insert(inner.id, Ty::Task);
                return Ty::Task;
            }
        }
        // Checked as a value of no place (a function giving none is not NO_VALUE here: the error is `start`'s).
        let t = self.expr(inner, Want::Ty(&Ty::Error));
        if !matches!(t, Ty::Error | Ty::Never) {
            let found = match &inner.kind {
                ExprKind::Call { callee, .. } => match &callee.kind {
                    ExprKind::Name { name, .. } => name.name.clone(),
                    ExprKind::Member { name, .. } => name.name.clone(),
                    ExprKind::Game(g) => g.text.clone(),
                    _ => self.ty_text(&t),
                },
                _ => self.ty_text(&t),
            };
            self.say(Diagnostic::new(code::START_NOT_TASK, inner.at).with("found", Arg::Text(found)));
        }
        Ty::Error
    }

    /// The task a callee names (`blink`, `effects.blink`), said nothing.
    fn task_target(&self, callee: &Expr) -> Option<ItemRef> {
        let item = match &callee.kind {
            ExprKind::Name { name, .. } => match self.name_kind(&name.name) {
                NameKind::Item(item) => item,
                _ => return None,
            },
            ExprKind::Member { base, name, .. } => {
                let ExprKind::Name { name: b, .. } = &base.kind else { return None };
                let NameKind::Module(ModTarget::Module { owner, module }) = self.name_kind(&b.name) else { return None };
                let m = ModRef { owner, module };
                match self.c.scope(m).names.get(&name.name) {
                    Some(Name::Item { item, public: true, .. }) => m.item(*item),
                    _ => return None,
                }
            }
            _ => return None,
        };
        matches!(self.item_of(item).kind, ast::ItemKind::Task { .. }).then_some(item)
    }
}

#[cfg(test)]
mod tests {
    /// Each name `builtin` matches is in METHOD_NAMES, so a value's method list leaves none out.
    #[test]
    fn every_builtin_name_is_listed() {
        let source = include_str!("call.rs");
        let start = source.find("fn builtin(t: &Ty, name: &str)").unwrap();
        let end = start + source[start..].find("Some(found)").unwrap();
        let mut missing = Vec::new();
        for line in source[start..end].lines() {
            let line = line.trim_start();
            if let Some(rest) = line.strip_prefix('"') {
                if let Some((name, after)) = rest.split_once('"') {
                    if after.trim_start().starts_with("=>") && !super::METHOD_NAMES.contains(&name) {
                        missing.push(name.to_string());
                    }
                }
            }
        }
        assert!(missing.is_empty(), "not in METHOD_NAMES: {missing:?}");
    }
}
