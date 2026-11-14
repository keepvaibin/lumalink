//! Expressions: numbers, names, operators (integers checked, docs/modding/language/compiled-form.md, section 6), conversions, the mod's values (tuples, structs,
//! enum cases, options), calls of the mod's functions and the game's (docs/modding/language/compiled-form.md, section 5), the standard functions, and text
//! and JSON as log and facts write them.

use super::{lost, counted, c_string, c_bytes, components, f32_c, f64_c, float_bounds, int_fits, is_view, limit_c, scalar_c, scalar_name, signed, unescape, value_c, Ex, Fx, Gen, NotWritten};
use crate::ast::{self, BinOp, Expr, ExprKind, ItemKind, Pos, StrPart, StrPrefix};
use crate::check::{ItemRef, Owner, Resolved, Ty};
use crate::ctype::{self, CType};
use crate::game::{GameTy, Scalar, TableLeaf, Value};
use luma_validate::names::FunctionKind;
use std::fmt::Write as _;

impl<'a> Gen<'a> {
    pub(super) fn not_yet<T>(&mut self, fx: &Fx, e: &Expr, what: &str) -> Result<T, NotWritten> {
        self.say(fx.module, e.at, what);
        Err(NotWritten)
    }

    pub(super) fn say_at<T>(&mut self, fx: &Fx, at: Pos, what: &str) -> Result<T, NotWritten> {
        self.say(fx.module, at, what);
        Err(NotWritten)
    }

    pub(super) fn item_value(&self, item: ItemRef) -> String {
        let ItemKind::Binding(b) = &self.item(item).kind else { return "0".into() };
        match &b.pattern.kind {
            ast::PatternKind::Path { path, .. } => super::global_name(item.module, &path[0].name),
            _ => "0".into(),
        }
    }

    /// `v` of type `from` where a `to` goes: the same, or a value put in an option. Other differences the
    /// checker has refused.
    pub(super) fn coerce(&mut self, fx: &mut Fx, v: &str, from: &Ty, to: &Ty, at: Pos) -> Ex {
        if from == to || matches!(from, Ty::Never | Ty::Error) {
            return Ok(v.to_string());
        }
        // The mod's class's object where a view (or an optional view) of its game base, or of a base of that, goes:
        // the object is its game base; a base further up is that base's subobject (core->as_class: a base need not
        // be at offset 0).
        if let Ty::Class(item) = from {
            let view = match to {
                Ty::Opt(x) => &**x,
                t => t,
            };
            if let Ty::Game(GameTy::View { record, .. }) = view {
                let ItemKind::Class(c) = &self.item(*item).kind else { return Err(lost()) };
                if c.base.text == *record {
                    return Ok(v.to_string());
                }
                let record = record.clone();
                let class = self.class_slot(&record);
                return Ok(fx.temp("void*", &format!("(void*)luma_rt_core->as_class(luma_rt_self, {v}, {class})")));
            }
        }
        match (from, to) {
            (_, Ty::Opt(x)) if **x == *from => {
                if is_view(x) {
                    return Ok(v.to_string());
                }
                let ct = self.c_type(to).ok_or_else(lost)?;
                Ok(fx.temp(&ct, &format!("({ct}){{1, {v}}}")))
            }
            // A view where a view of a base goes, a const view where a view goes: the same address.
            (Ty::Game(GameTy::View { .. }), Ty::Game(GameTy::View { .. })) => Ok(v.to_string()),
            (Ty::Game(GameTy::View { .. }), Ty::Opt(x)) if is_view(x) => Ok(v.to_string()),
            (Ty::Opt(a), Ty::Opt(b)) if is_view(a) && is_view(b) => Ok(v.to_string()),
            // C6: a handle where a handle of a base goes (6.2): the same handle; `get()` narrows by the class its type
            // names (lumart_ref_get through as_class).
            (Ty::Ref(_), Ty::Ref(_)) => Ok(v.to_string()),
            // An option's C type is its own per type: the same handle in the other's option.
            (Ty::Opt(a), Ty::Opt(b)) if matches!((&**a, &**b), (Ty::Ref(_), Ty::Ref(_))) => {
                let (fc, tc) = (self.c_type(from).ok_or_else(lost)?, self.c_type(to).ok_or_else(lost)?);
                let o = fx.temp(&fc, v);
                Ok(fx.temp(&tc, &format!("({tc}){{{o}.some, {o}.value}}")))
            }
            (Ty::Ref(_), Ty::Opt(x)) if matches!(**x, Ty::Ref(_)) => {
                let inner = (**x).clone();
                self.coerce(fx, v, &inner, to, at)
            }
            // A plain game record held in the game (a view of it) where the script's copy goes (D9.x): its size
            // row's bytes copied; through `?.`, copied when there is one.
            (Ty::Game(GameTy::View { record, .. }), Ty::Game(GameTy::Struct(r))) if record == r => {
                let r = r.clone();
                Ok(self.record_copy(fx, at.line, v, &r))
            }
            (Ty::Opt(a), Ty::Opt(b)) if matches!((&**a, &**b), (Ty::Game(GameTy::View { record, .. }), Ty::Game(GameTy::Struct(r))) if record == r) => {
                let Ty::Game(GameTy::Struct(r)) = &**b else { return Err(lost()) };
                let r = r.clone();
                let ct = self.c_type(to).ok_or_else(lost)?;
                let o = fx.temp("void*", v);
                let size = self.size_slot(&r);
                let copy = fx.fresh("lumart_bytes");
                fx.emit(format!("if ({o} != 0) {{"));
                fx.depth += 1;
                fx.fail_if(&format!("lumart_record_copy(&{copy}, (const void*){o}, {size}, {}, {})", c_string(&fx.file), at.line));
                fx.depth -= 1;
                fx.emit("}".into());
                Ok(fx.temp(&ct, &format!("({ct}){{{o} != 0, {copy}}}")))
            }
            _ => {
                let what = format!("{} where {} goes", self.ty_text(from), self.ty_text(to));
                self.say_at(fx, at, &what)
            }
        }
    }

    /// An expression's value as C: a temporary or a constant. Its reads and its errors come before, in order.
    pub(super) fn expr(&mut self, fx: &mut Fx, e: &Expr) -> Ex {
        if self.pre_read.as_ref().is_some_and(|(id, _)| *id == e.id) {
            return Ok(self.pre_read.take().map(|(_, v)| v).unwrap_or_default());
        }
        let m = fx.module;
        let t = self.ty(m, e);
        // A plain game record read from the game is the script's copy: its size row's bytes into a new block.
        // Its fields read along a chain of places need no copy (game.rs); a copy the script holds is a value
        // like any other (a local, a field, an element).
        if let Ty::Game(GameTy::Struct(record)) = &t {
            if matches!(e.kind, ExprKind::Member { optional: false, .. } | ExprKind::Game(_) | ExprKind::Index { .. } | ExprKind::Name { .. }) {
                if let Some(p) = self.game_place_of(fx, e) {
                    let super::game::GamePlace::Address(a) = p? else { return self.not_yet(fx, e, "a copy of a game struct") };
                    let record = record.clone();
                    return Ok(self.record_copy(fx, e.at.line, &a, &record));
                }
            }
        }
        // A game name: a folded constant, an enum's value, a field or a global (each read through its import row).
        if matches!(e.kind, ExprKind::Name { .. } | ExprKind::Member { optional: false, .. } | ExprKind::Game(_) | ExprKind::Index { .. }) {
            match self.resolved(m, e) {
                Some(Resolved::GameConstant { value, .. }) => return self.game_constant(fx, e, value),
                Some(Resolved::GameEnumValue { number, .. }) => return Ok(format!("((int32_t){number})")),
                _ => {}
            }
            if let Some(p) = self.game_place_of(fx, e) {
                let p = p?;
                if let super::game::GamePlace::Array { .. } = p {
                    return self.not_yet(fx, e, "a game array as a whole");
                }
                let v = self.game_read(fx, p)?;
                let ct = self.c_type(&t).ok_or_else(lost)?;
                return Ok(fx.temp(&ct, &v));
            }
        }
        // A mod class's own field (class.rs): in its data beside the object.
        if let Some(p) = self.class_field_place(fx, e) {
            let p = p?;
            let ct = self.c_type(&t).ok_or_else(lost)?;
            return Ok(fx.temp(&ct, &p));
        }
        // A literal where a json.Value goes is one.
        if t == Ty::Json && super::json::json_literal(&e.kind) {
            return self.json_value(fx, e);
        }
        match &e.kind {
            ExprKind::Number(text) => self.number(fx, e, text, &t),
            ExprKind::True => Ok("((uint8_t)1)".into()),
            ExprKind::False => Ok("((uint8_t)0)".into()),
            ExprKind::Paren(x) => self.expr(fx, x),
            ExprKind::Color(text) => {
                let hex = text.trim_start_matches('#');
                let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f32 / 255.0;
                let a = if hex.len() >= 8 { byte(6) } else { 1.0 };
                Ok(format!("((lm_color){{{}, {}, {}, {}}})", f32_c(byte(0)), f32_c(byte(2)), f32_c(byte(4)), f32_c(a)))
            }
            ExprKind::None => match &t {
                Ty::Opt(x) if is_view(x) => Ok("((void*)0)".into()),
                Ty::Opt(_) => {
                    let ct = self.c_type(&t).ok_or_else(lost)?;
                    Ok(fx.temp(&ct, &format!("({ct}){{0}}")))
                }
                _ => self.not_yet(fx, e, "none of this type"),
            },
            ExprKind::SelfValue => match (&fx.self_c, &fx.hook) {
                (Some(c), _) => Ok(c.clone()),
                (None, Some(_)) => Ok("call->args[0]".into()),
                (None, None) => self.not_yet(fx, e, "self here"),
            },
            ExprKind::Name { .. } => match self.resolved(m, e) {
                Some(Resolved::Local(id)) => Ok(fx.local(*id)),
                Some(Resolved::Item(item)) if item.owner == Owner::Mod && matches!(self.item(*item).kind, ItemKind::Binding(_)) => Ok(self.item_value(*item)),
                // A function of the mod as a value (5.4): its static block.
                Some(Resolved::Item(item)) if item.owner == Owner::Mod && matches!(self.item(*item).kind, ItemKind::Fn(_)) => {
                    let item = *item;
                    let ItemKind::Fn(f) = &self.item(item).kind else { return Err(lost()) };
                    let c_name = self.item_c(item, &f.name.name);
                    self.fn_item_value(fx, e, &c_name, &t)
                }
                // An option: its global, as start or its OPTION event last read it.
                Some(Resolved::Item(item)) if item.owner == Owner::Mod && matches!(self.item(*item).kind, ItemKind::Option { .. }) => {
                    let ItemKind::Option { key, .. } = &self.item(*item).kind else { return Err(lost()) };
                    Ok(format!("lm_opt_{}", key.name))
                }
                Some(Resolved::GameEnumValue { number, .. }) => Ok(format!("((int32_t){number})")),
                Some(Resolved::Limit { ty, max }) => Ok(limit_c(*ty, *max)),
                _ => self.not_yet(fx, e, "this name as a value"),
            },
            ExprKind::Result => match fx.hook.as_ref().and_then(|h| h.result_c.clone()) {
                Some(rc) => Ok(fx.temp(&rc, &format!("LUMA_RESULT(call, {rc})"))),
                None => self.not_yet(fx, e, "result"),
            },
            ExprKind::Member { base, optional: false, .. } => match self.resolved(m, e) {
                Some(Resolved::Component(i)) => {
                    let Ty::Value(v) = self.ty(m, base) else { return self.not_yet(fx, e, "a component of this value") };
                    let b = self.expr(fx, base)?;
                    Ok(format!("{b}.{}", components(v)[*i]))
                }
                Some(Resolved::Limit { ty, max }) => Ok(limit_c(*ty, *max)),
                Some(Resolved::TupleIndex(i)) => {
                    let i = *i;
                    let b = self.expr(fx, base)?;
                    Ok(format!("{b}._{i}"))
                }
                Some(Resolved::Member { name, .. }) if matches!(self.ty(m, base), Ty::Struct(_)) => {
                    let name = name.clone();
                    let b = self.expr(fx, base)?;
                    Ok(format!("{b}.{name}"))
                }
                Some(Resolved::Case { item, case }) => self.case_value(fx, e, *item, &case.clone(), &[]),
                Some(Resolved::GameEnumValue { number, .. }) => Ok(format!("((int32_t){number})")),
                // A service's enum case and constant, folded.
                Some(Resolved::ServiceCase { number, .. }) => Ok(format!("((int32_t){number})")),
                Some(Resolved::ServiceConst { value, .. }) if u32::try_from(*value).is_ok() => Ok(format!("((uint32_t){value}u)")),
                Some(Resolved::ServiceConst { value, .. }) => Ok(format!("((uint64_t){value}ull)")),
                // A field of a service's struct: the header's struct is the value.
                Some(Resolved::ServiceField { service, record, field }) => {
                    let (service, record, field) = (service.clone(), record.clone(), field.clone());
                    let bt = self.ty(m, base);
                    let Some(ct) = self.c_type(&bt) else { return self.not_yet(fx, e, "a field of this service struct") };
                    let b = self.expr(fx, base)?;
                    let b = fx.temp(&ct, &b);
                    match self.service_field_get(fx, e.at.line, &b, &service, &record, &field) {
                        Some(v) => Ok(v),
                        None => self.not_yet(fx, e, "this field of a service struct"),
                    }
                }
                _ => self.not_yet(fx, e, "this member"),
            },
            // `a?.b`: none when a is none, else a's b (an option of it, unless it is one already).
            ExprKind::Member { base, optional: true, .. } => {
                let bt = self.ty(m, base);
                let Ty::Opt(inner_t) = &bt else { return self.not_yet(fx, e, "?. on this type") };
                let inner_t = (**inner_t).clone();
                let bct = self.c_type(&bt).ok_or_else(lost)?;
                let ct = self.c_type(&t).ok_or_else(lost)?;
                let b = self.expr(fx, base)?;
                let tmp = fx.temp(&bct, &b);
                let (has, val) = if is_view(&inner_t) { (format!("{tmp} != 0"), tmp.clone()) } else { (format!("{tmp}.some"), format!("{tmp}.value")) };
                let r = fx.fresh(&ct);
                fx.emit(format!("if ({has}) {{"));
                fx.depth += 1;
                let own_t = match &t {
                    Ty::Opt(x) => (**x).clone(),
                    other => other.clone(),
                };
                let (v, own) = self.member_value(fx, e, &val, &inner_t, &own_t)?;
                let own_c = self.c_type(&own).ok_or_else(lost)?;
                if own_c == ct {
                    fx.set(&r, &v, &ct);
                } else {
                    fx.set(&r, &format!("({ct}){{1, {v}}}"), &ct);
                }
                fx.depth -= 1;
                // None: written each time (a loop's next round must not see the last one's value).
                let none = if ct == "void*" { "(void*)0".to_string() } else { format!("({ct}){{0}}") };
                fx.emit("} else {".into());
                fx.depth += 1;
                fx.set(&r, &none, &ct);
                fx.depth -= 1;
                fx.emit("}".into());
                Ok(r)
            }
            ExprKind::Neg(x) => {
                let v = self.expr(fx, x)?;
                match &t {
                    Ty::Int(s) if signed(*s) => {
                        let ct = scalar_c(*s);
                        let r = fx.fresh(ct);
                        let src = expr_text(e);
                        fx.fail_if(&format!("__builtin_sub_overflow(({ct})0, {v}, &{r}) && lumart_overflow({}, {}, \"{}\", {})", c_string(&fx.file), e.at.line, scalar_name(*s), c_string(&src)));
                        Ok(r)
                    }
                    Ty::Float(_) => Ok(format!("(-{v})")),
                    Ty::Value(val) if !components(*val).is_empty() => {
                        let parts: Vec<String> = components(*val).iter().map(|c| format!("-{v}.{c}")).collect();
                        Ok(fx.temp(value_c(*val), &format!("({}){{{}}}", value_c(*val), parts.join(", "))))
                    }
                    _ => self.not_yet(fx, e, "- on this type"),
                }
            }
            ExprKind::Not(x) => {
                let v = self.expr(fx, x)?;
                Ok(format!("((uint8_t)!({v}))"))
            }
            ExprKind::Binary { op: op @ (BinOp::And | BinOp::Or), left, right } => {
                // Short-circuit: the right side's reads and errors happen only when it is read.
                let is_and = *op == BinOp::And;
                let l = self.expr(fx, left)?;
                let r = fx.temp("uint8_t", &l);
                fx.emit(format!("if ({}{r}) {{", if is_and { "" } else { "!" }));
                fx.depth += 1;
                let rv = self.expr(fx, right);
                if let Ok(rv) = &rv {
                    fx.emit(format!("{r} = {rv};"));
                }
                fx.depth -= 1;
                fx.emit("}".into());
                rv.map(|_| r)
            }
            ExprKind::Binary { op: BinOp::Coalesce, left, right } => {
                // `a ?? b`: b read only when a is none.
                let lt = self.ty(m, left);
                let ct = self.c_type(&t).ok_or_else(lost)?;
                let l = self.expr(fx, left)?;
                let r = fx.fresh(&ct);
                let lc = self.c_type(&lt).ok_or_else(lost)?;
                let a = fx.temp(&lc, &l);
                let (has, inner) = match &lt {
                    Ty::Opt(x) if is_view(x) => (format!("{a} != 0"), a.clone()),
                    _ => (format!("{a}.some"), format!("{a}.value")),
                };
                let inner_t = match &lt {
                    Ty::Opt(x) => (**x).clone(),
                    other => other.clone(),
                };
                fx.emit(format!("if ({has}) {{"));
                fx.depth += 1;
                let v = self.coerce(fx, &inner, &inner_t, &t, e.at)?;
                fx.set(&r, &v, &ct);
                fx.depth -= 1;
                fx.emit("} else {".into());
                fx.depth += 1;
                self.value_into(fx, right, &r, &t);
                fx.depth -= 1;
                fx.emit("}".into());
                Ok(r)
            }
            ExprKind::Binary { op, left, right } => {
                // An optional service's alias against `none`: its slot, null when the service was not given.
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    let pair = [(left, right), (right, left)].into_iter().find(|(_, n)| matches!(strip(n).kind, ExprKind::None));
                    if let Some((x, _)) = pair {
                        if let Some(Resolved::ServiceGiven(service)) = self.resolved(m, x) {
                            let slot = self.service_slot(service).ok_or_else(lost)?;
                            return Ok(format!("((uint8_t)({slot} {} 0))", if *op == BinOp::Eq { "==" } else { "!=" }));
                        }
                    }
                }
                let lt = self.ty(m, left);
                let rt = self.ty(m, right);
                // `x == none`, `x != none`.
                if matches!(op, BinOp::Eq | BinOp::Ne) && matches!(strip(right).kind, ExprKind::None) && matches!(lt, Ty::Opt(_)) {
                    let l = self.expr(fx, left)?;
                    let has = if matches!(&lt, Ty::Opt(x) if is_view(x)) { format!("({l} != 0)") } else { format!("{l}.some") };
                    return Ok(if *op == BinOp::Eq { format!("((uint8_t)!{has})") } else { format!("((uint8_t){has})") });
                }
                // The left side is read before the right (a call on the right may change a place on the left).
                let l = self.expr(fx, left)?;
                let l = match self.c_type(&lt) {
                    Some(ct) => fx.temp(&ct, &l),
                    None => l,
                };
                let r = self.expr(fx, right)?;
                self.binary_values(fx, *op, &l, &lt, &r, &rt, &t, &expr_text(e), e.at)
            }
            ExprKind::As { value, narrow: true, .. } => {
                // `x as? T`: the object's T part by the kernel's as_class, null (none) when it is no T or x is none.
                let Ty::Opt(inner) = &t else { return Err(lost()) };
                let Ty::Game(GameTy::View { record, .. }) = &**inner else { return Err(lost()) };
                let record = record.clone();
                let v = self.expr(fx, value)?;
                let class = self.class_slot(&record);
                let object = fx.temp("void*", &format!("(void*)({v})"));
                Ok(fx.temp("void*", &format!("{object} == 0 ? (void*)0 : (void*)luma_rt_core->as_class(luma_rt_self, {object}, {class})")))
            }
            ExprKind::As { value, .. } => {
                let from = self.ty(m, value);
                let v = self.expr(fx, value)?;
                self.convert(fx, e, &v, &from, &t)
            }
            ExprKind::Unwrap(x) => {
                // `x!`: none is S0906 naming the expression.
                let xt = self.ty(m, x);
                let ct = self.c_type(&xt).ok_or_else(lost)?;
                let v = self.expr(fx, x)?;
                let tmp = fx.temp(&ct, &v);
                let (has, inner) = match &xt {
                    Ty::Opt(y) if is_view(y) => (format!("{tmp} != 0"), tmp.clone()),
                    Ty::Opt(_) => (format!("{tmp}.some"), format!("{tmp}.value")),
                    _ => return self.not_yet(fx, e, "! on this type"),
                };
                let src = c_string(&expr_text(x));
                fx.fail_if(&format!("!({has}) && lumart_raise({}, {}, 906, 1, (const char* const[]){{{src}}})", c_string(&fx.file), e.at.line));
                Ok(inner)
            }
            ExprKind::Tuple(items) => {
                let ct = self.c_type(&t).ok_or_else(lost)?;
                let Ty::Tuple(ts) = &t else { return self.not_yet(fx, e, "this tuple") };
                let ts = ts.clone();
                let mut parts = Vec::new();
                let last = items.len().saturating_sub(1);
                for (n, (x, xt)) in items.iter().zip(ts.iter()).enumerate() {
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    let v = self.coerce(fx, &v, &from, xt, x.at)?;
                    // Each part read where it is written: a part after it may change what it read (`(xs, xs.pop())`).
                    let v = match self.c_type(xt) {
                        Some(pc) if n < last => fx.temp(&pc, &v),
                        _ => v,
                    };
                    parts.push(v);
                }
                Ok(fx.temp(&ct, &format!("({ct}){{{}}}", parts.join(", "))))
            }
            ExprKind::StructLit { path, fields } => self.struct_lit(fx, e, path, fields, &t),
            // `ref(view)`: the object's slot in the mod's handle table, keyed by its NameObj (docs/modding/language/compiled-form.md, section 7.3).
            ExprKind::Ref(args) => {
                let Some(a) = args.first() else { return Err(lost()) };
                // A mod's class's object is its game base's object (the kernel made it so): the same slot.
                if !matches!(self.ty(m, &a.value), Ty::Game(GameTy::View { .. }) | Ty::Class(_)) {
                    return self.not_yet(fx, e, "ref of this value");
                }
                let v = self.expr(fx, &a.value)?;
                let nameobj = self.class_slot("NameObj");
                let r = fx.fresh("lumart_ref");
                fx.fail_if(&format!("lumart_ref_make(&{r}, {v}, {nameobj}, {}, {})", c_string(&fx.file), e.at.line));
                Ok(r)
            }
            ExprKind::If { .. } | ExprKind::Match { .. } => {
                if matches!(t, Ty::Unit | Ty::Never) {
                    // As a statement, in an expression's place.
                    let s = ast::Stmt { at: e.at, kind: ast::StmtKind::Expr(e.clone()) };
                    self.stmt(fx, &s);
                    return Ok(String::new());
                }
                let ct = self.c_type(&t).ok_or_else(|| {
                    let what = format!("if or match giving {}", self.ty_text(&t));
                    self.say(m, e.at, &what);
                    NotWritten
                })?;
                let r = fx.fresh(&ct);
                self.value_into(fx, e, &r, &t);
                Ok(r)
            }
            ExprKind::Call { callee, args } => self.call(fx, e, callee, args, &t),
            // A closure where a value goes (5.4; closure.rs); one given to a builtin is written in place (flow.rs).
            ExprKind::Closure { params, body, .. } => self.closure_value(fx, e, params, body, &t),
            // In a wrap: the next body of this mod's wraps on the function, at the last the game's own code (docs/modding/language/compiled-form.md, section 4).
            ExprKind::Proceed => match fx.hook.as_ref().map(|h| h.kind) {
                Some(ast::HookKind::Wrap) => {
                    // The texts the wrap wrote into its `const char *` parameters go to the call first.
                    fx.text_args_back(false);
                    fx.place_args_back();
                    fx.emit("*lm_proceeded = 1;".into());
                    // lm_next is the next body's wrapper (or the kernel's proceed): a script error in that body ends
                    // that body alone, its wrapper proceeding for it, so it always gives 0 and this wrap goes on.
                    fx.emit("(void)lm_next(call);".into());
                    fx.place_args_in();
                    Ok(String::new())
                }
                _ => self.not_yet(fx, e, "proceed() outside a wrap"),
            },
            ExprKind::Str(_) => self.text_value(fx, e, &t),
            ExprKind::Content { word, path, args } if word == "text" => self.text_row(fx, e, path, args.as_deref().unwrap_or(&[])),
            ExprKind::Content { word, path, args: None } if t == Ty::Text && !matches!(word.as_str(), "text" | "data" | "nerve") => {
                // C2: `model "x"` is the text "x" (part f checked the name against the mod's content).
                let mut bytes = Vec::new();
                for part in &path.parts {
                    match part {
                        StrPart::Text(raw) => bytes.extend(if path.prefix == StrPrefix::Raw { raw.as_bytes().to_vec() } else { unescape(raw) }),
                        _ => return self.not_yet(fx, e, "a content name made at run time (write `<kind>_named(..)`)"),
                    }
                }
                // A model is the game's resource name: a mod's own "Luma/<id>/<path>" (its cooked archive), a needed
                // mod's `<id>:<path>` the same with that id; a game model's name as written.
                if word == "model" {
                    let name = String::from_utf8_lossy(&bytes).into_owned();
                    if let Some((id, rest)) = name.split_once(':') {
                        bytes = format!("Luma/{id}/{rest}").into_bytes();
                    } else if self.own_models.contains(&name) {
                        bytes = format!("Luma/{}/{name}", self.mod_id).into_bytes();
                    }
                }
                Ok(self.text_literal(fx.module, &bytes))
            }
            ExprKind::List(items) => self.list_value(fx, e, items, &t),
            ExprKind::Braces(entries) => self.braces_value(fx, e, entries, &t),
            ExprKind::Index { base, index } => self.index_value(fx, e, base, index),
            ExprKind::Start(inner) => self.start_expr(fx, e, inner),
            ExprKind::Wait { amount, unit } => self.wait_expr(fx, e, amount, *unit),
            ExprKind::WaitUntil(c) => self.wait_until(fx, e, c),
            _ => self.not_yet(fx, e, expr_name(&e.kind)),
        }
    }

    /// Member `e` of a value already read (`b`, of type `bt`): its value and its own type (`own`: the type the
    /// checker gives the member, an option's inside for `?.`).
    fn member_value(&mut self, fx: &mut Fx, e: &Expr, b: &str, bt: &Ty, own: &Ty) -> Result<(String, Ty), NotWritten> {
        let m = fx.module;
        match self.resolved(m, e).cloned() {
            Some(Resolved::Component(i)) => match bt {
                Ty::Value(v) => Ok((format!("{b}.{}", components(*v)[i]), Ty::Float(Scalar::F32))),
                _ => self.not_yet(fx, e, "a component of this value"),
            },
            Some(Resolved::TupleIndex(i)) => match bt {
                Ty::Tuple(xs) if i < xs.len() => Ok((format!("{b}._{i}"), xs[i].clone())),
                _ => self.not_yet(fx, e, "this tuple index"),
            },
            Some(Resolved::Member { owner, name }) if matches!(bt, Ty::Struct(_)) => Ok((format!("{b}.{name}"), self.declared_ty(owner, &format!(".{name}")))),
            Some(Resolved::GameField { record, field }) if is_view(bt) => {
                let object = fx.temp("void*", b);
                let p = self.game_field_place(fx, e, object, own, &record, &field)?;
                // A plain game struct is the script's copy, here as after `.`: never its address in the
                // game, which a count taken or given back would write over.
                if let Ty::Game(GameTy::Struct(rec)) = own {
                    let super::game::GamePlace::Address(a) = p else { return self.not_yet(fx, e, "a copy of this game struct after ?.") };
                    let rec = rec.clone();
                    return Ok((self.record_copy(fx, e.at.line, &a, &rec), own.clone()));
                }
                let v = self.game_read(fx, p)?;
                Ok((v, own.clone()))
            }
            _ => self.not_yet(fx, e, "this member after ?."),
        }
    }

    fn number(&mut self, fx: &mut Fx, e: &Expr, text: &str, t: &Ty) -> Ex {
        let (digits, radix) = number_text(text);
        match t {
            Ty::Int(s) => {
                let Ok(v) = u128::from_str_radix(&digits, radix) else { return self.not_yet(fx, e, "this number") };
                // A literal is non-negative (`-` is an operator); written unsigned and cast, exact for every width.
                Ok(format!("(({})0x{v:x}ull)", scalar_c(*s)))
            }
            Ty::Float(Scalar::F64) => {
                let v: f64 = if radix == 10 { digits.parse().unwrap_or(0.0) } else { u128::from_str_radix(&digits, radix).unwrap_or(0) as f64 };
                Ok(f64_c(v))
            }
            Ty::Float(_) => {
                let v: f32 = if radix == 10 { digits.parse().unwrap_or(0.0) } else { u128::from_str_radix(&digits, radix).unwrap_or(0) as f32 };
                Ok(f32_c(v))
            }
            _ => self.not_yet(fx, e, "a number of this type"),
        }
    }

    /// `l op r` with both read: numbers (integers checked, docs/modding/language/compiled-form.md, section 6), vectors, colours; comparisons.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn binary_values(&mut self, fx: &mut Fx, op: BinOp, l: &str, lt: &Ty, r: &str, rt: &Ty, t: &Ty, source: &str, at: Pos) -> Ex {
        let file = c_string(&fx.file);
        let src = c_string(source);
        let line = at.line;
        let cmp = |o: &str| format!("((uint8_t)({l} {o} {r}))");
        match (op, lt, rt) {
            (BinOp::Eq | BinOp::Ne, Ty::Value(v), Ty::Value(_)) if !components(*v).is_empty() => {
                let all: Vec<String> = components(*v).iter().map(|c| format!("{l}.{c} == {r}.{c}")).collect();
                let eq = format!("({})", all.join(" && "));
                Ok(if op == BinOp::Eq { format!("((uint8_t){eq})") } else { format!("((uint8_t)!{eq})") })
            }
            // The matrix: its twelve floats, each as `==` on f32.
            (BinOp::Eq | BinOp::Ne, Ty::Value(Value::Mat34), Ty::Value(_)) => {
                let all: Vec<String> = (0..3).flat_map(|i| (0..4).map(move |j| (i, j))).map(|(i, j)| format!("{l}.m[{i}][{j}] == {r}.m[{i}][{j}]")).collect();
                let eq = format!("({})", all.join(" && "));
                Ok(if op == BinOp::Eq { format!("((uint8_t){eq})") } else { format!("((uint8_t)!{eq})") })
            }
            // Text by its bytes; tuples, structs, options and enums with values part by part (docs/modding/language/language.md, section 3).
            (BinOp::Eq | BinOp::Ne, Ty::Text | Ty::Tuple(_) | Ty::Struct(_) | Ty::Opt(_) | Ty::List(_) | Ty::Map(..) | Ty::Set(_) | Ty::Ref(_) | Ty::Bytes, _) | (BinOp::Eq | BinOp::Ne, Ty::Enum(_), _) if !matches!(lt, Ty::Enum(i) if self.enum_plain(*i)) => {
                let ct = self.c_type(lt).ok_or_else(lost)?;
                let a = fx.temp(&ct, l);
                let r = if rt == lt { r.to_string() } else { self.coerce(fx, r, rt, lt, at)? };
                let b = fx.temp(&ct, &r);
                let Some(eq) = self.eq_c(&a, &b, lt) else {
                    let what = format!("== on {}", self.ty_text(lt));
                    return self.say_at(fx, at, &what);
                };
                Ok(fx.temp("uint8_t", &if op == BinOp::Eq { format!("(uint8_t){eq}") } else { format!("(uint8_t)!{eq}") }))
            }
            // Text in byte order, which is code point order for UTF-8.
            (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Ty::Text, _) => {
                let o = binop_text(op);
                Ok(fx.temp("uint8_t", &format!("(uint8_t)(lumart_text_cmp({l}, {r}) {o} 0)")))
            }
            (BinOp::Eq | BinOp::Ne, Ty::Enum(item), _) if self.enum_plain(*item) => {
                Ok(if op == BinOp::Eq { format!("((uint8_t)({l}.tag == {r}.tag))") } else { format!("((uint8_t)({l}.tag != {r}.tag))") })
            }
            // A service's enum or handle: its C number (J, FB 2026-11-12).
            (BinOp::Eq | BinOp::Ne, Ty::Service { service, name }, _) if self.service_number(service, name) => Ok(cmp(if op == BinOp::Eq { "==" } else { "!=" })),
            (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Game(GameTy::Enum(_)), _) => Ok(cmp(match op {
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                _ => ">=",
            })),
            (BinOp::Add | BinOp::Sub | BinOp::Mul, Ty::Int(s), _) => {
                let ct = scalar_c(*s);
                let res = fx.fresh(ct);
                let f = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    _ => "mul",
                };
                fx.fail_if(&format!("__builtin_{f}_overflow(({ct}){l}, ({ct}){r}, &{res}) && lumart_overflow({file}, {line}, \"{}\", {src})", scalar_name(*s)));
                Ok(res)
            }
            (BinOp::Div | BinOp::Rem, Ty::Int(s), _) => {
                let ct = scalar_c(*s);
                let rv = fx.temp(ct, r);
                fx.fail_if(&format!("{rv} == 0 && lumart_div_zero({file}, {line}, {src})"));
                if signed(*s) {
                    fx.fail_if(&format!("{l} == {} && {rv} == -1 && lumart_overflow({file}, {line}, \"{}\", {src})", limit_c(*s, false), scalar_name(*s)));
                }
                let o = if op == BinOp::Div { "/" } else { "%" };
                Ok(fx.temp(ct, &format!("({ct})({l} {o} {rv})")))
            }
            (BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor, Ty::Int(s), _) => {
                let o = match op {
                    BinOp::BitAnd => "&",
                    BinOp::BitOr => "|",
                    _ => "^",
                };
                Ok(format!("(({})({l} {o} {r}))", scalar_c(*s)))
            }
            (BinOp::Shl | BinOp::Shr, Ty::Int(s), Ty::Int(rs)) => {
                // A count past the width is an overflow (docs/modding/language/language.md, section 3: no undefined shifts); `<<` keeps the low bits.
                let ct = scalar_c(*s);
                let bits = match s {
                    Scalar::I8 | Scalar::U8 => 8,
                    Scalar::I16 | Scalar::U16 => 16,
                    Scalar::I32 | Scalar::U32 => 32,
                    _ => 64,
                };
                let n = fx.temp("uint64_t", &format!("(uint64_t){r}"));
                let negative = if signed(*rs) { format!("(int64_t){r} < 0 || ") } else { String::new() };
                fx.fail_if(&format!("({negative}{n} >= {bits}u) && lumart_overflow({file}, {line}, \"{}\", {src})", scalar_name(*s)));
                let ut = scalar_c(unsigned_of(*s));
                Ok(if op == BinOp::Shl { format!("(({ct})(({ut}){l} << {n}))") } else { format!("(({ct})({l} >> {n}))") })
            }
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, Ty::Float(s), Ty::Float(_)) => {
                let o = binop_text(op);
                Ok(fx.temp(scalar_c(*s), &format!("{l} {o} {r}")))
            }
            (BinOp::Rem, Ty::Float(_), _) => self.say_at(fx, at, "% on floats"),
            (BinOp::Add | BinOp::Sub, Ty::Value(v), Ty::Value(_)) if !components(*v).is_empty() => {
                let o = binop_text(op);
                let parts: Vec<String> = components(*v).iter().map(|c| format!("{l}.{c} {o} {r}.{c}")).collect();
                let out = colour_clamp(*v, parts);
                Ok(fx.temp(value_c(*v), &format!("({}){{{}}}", value_c(*v), out.join(", "))))
            }
            (BinOp::Mul | BinOp::Div, Ty::Value(v), Ty::Float(_)) if !components(*v).is_empty() => {
                let o = binop_text(op);
                let parts: Vec<String> = components(*v).iter().map(|c| format!("{l}.{c} {o} {r}")).collect();
                let out = colour_clamp(*v, parts);
                Ok(fx.temp(value_c(*v), &format!("({}){{{}}}", value_c(*v), out.join(", "))))
            }
            (BinOp::Mul, Ty::Float(_), Ty::Value(v)) if !components(*v).is_empty() => {
                let parts: Vec<String> = components(*v).iter().map(|c| format!("{l} * {r}.{c}")).collect();
                let out = colour_clamp(*v, parts);
                Ok(fx.temp(value_c(*v), &format!("({}){{{}}}", value_c(*v), out.join(", "))))
            }
            _ => {
                let _ = t;
                let what = format!("{} on {} and {}", binop_text(op), self.ty_text(lt), self.ty_text(rt));
                self.say_at(fx, at, &what)
            }
        }
    }

    /// Whether an enum's cases carry no values (its equality is its tag's).
    pub(super) fn enum_plain(&self, item: ItemRef) -> bool {
        matches!(&self.item(item).kind, ItemKind::Enum(e) if e.cases.iter().all(|c| c.fields.is_empty()))
    }

    /// `v as T` between numbers: an integer that does not fit is S0908; a float to an integer is cut
    /// toward zero and must fit too.
    fn convert(&mut self, fx: &mut Fx, e: &Expr, v: &str, from: &Ty, to: &Ty) -> Ex {
        let file = c_string(&fx.file);
        let line = e.at.line;
        match (from, to) {
            (a, b) if a == b => Ok(v.to_string()),
            // A game's or service's enum is its `int32_t` number, converted as an i32 is.
            (Ty::Game(GameTy::Enum(_)) | Ty::Service { .. }, Ty::Int(_)) => self.convert(fx, e, &format!("((int32_t){v})"), &Ty::Int(Scalar::I32), to),
            (Ty::Int(_), Ty::Game(GameTy::Enum(_)) | Ty::Service { .. }) => {
                let n = self.convert(fx, e, v, from, &Ty::Int(Scalar::I32))?;
                Ok(format!("((int32_t){n})"))
            }
            (Ty::Int(a), Ty::Int(b)) => {
                let ct = scalar_c(*b);
                let tmp = fx.temp(scalar_c(*a), v);
                if let Some(fits) = int_fits(*a, *b, &tmp) {
                    fx.emit(format!("if (!({fits})) {{"));
                    fx.depth += 1;
                    let shown = fx.temp_text_int(&tmp, *a);
                    fx.fail_if(&format!("lumart_raise({file}, {line}, 908, 2, (const char* const[]){{{shown}, \"{}\"}})", scalar_name(*b)));
                    fx.depth -= 1;
                    fx.emit("}".into());
                }
                Ok(format!("(({ct}){tmp})"))
            }
            (Ty::Int(_), Ty::Float(b)) | (Ty::Float(_), Ty::Float(b)) => Ok(format!("(({}){v})", scalar_c(*b))),
            (Ty::Float(_), Ty::Int(b)) => {
                // In range when -2^w-1 < v < 2^w-1 (signed) or -1 < v < 2^w (unsigned); NaN never.
                let (lo, hi) = float_bounds(*b);
                let tmp = fx.temp("double", &format!("(double){v}"));
                fx.emit(format!("if (!({tmp} > {lo} && {tmp} < {hi})) {{"));
                fx.depth += 1;
                let shown = fx.temp_text_float(&tmp);
                fx.fail_if(&format!("lumart_raise({file}, {line}, 908, 2, (const char* const[]){{{shown}, \"{}\"}})", scalar_name(*b)));
                fx.depth -= 1;
                fx.emit("}".into());
                Ok(format!("(({}){tmp})", scalar_c(*b)))
            }
            _ => self.not_yet(fx, e, "this conversion"),
        }
    }

    // ------------------------------------------------------------ the mod's values

    /// `E.Case` or `E.Case(a, b)`: the tag, and the fields given in order.
    fn case_value(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, case: &str, args: &[ast::Arg]) -> Ex {
        let ct = self.c_type(&Ty::Enum(item)).ok_or_else(lost)?;
        let tag = self.case_tag(item, case).ok_or_else(lost)?;
        let ItemKind::Enum(decl) = &self.item(item).kind else { return Err(lost()) };
        let fields = decl.cases[tag].fields.clone();
        let values = self.fill(fx, e, item, &format!("{case}."), &fields, args)?;
        let r = fx.fresh(&ct);
        // The temporary's last value (a loop's last round) given back whole, then filled: its fields take counts.
        if counted(&ct) {
            fx.emit(format!("lm_rel_{ct}(&{r});"));
        }
        fx.emit(format!("{r}.tag = {tag}u;"));
        for (f, v) in fields.iter().zip(values) {
            let fc = self.field_c(item, &format!("{case}.{}", f.name.name))?;
            fx.set(&format!("{r}.u.{case}.{}", f.name.name), &v, &fc);
        }
        Ok(r)
    }

    /// The values of declared fields or parameters from call arguments: positional in order, then named, then each
    /// missing one's default (read in its own module), each made to its declared type.
    fn fill(&mut self, fx: &mut Fx, e: &Expr, owner: ItemRef, prefix: &str, fields: &[ast::Field], args: &[ast::Arg]) -> Result<Vec<String>, NotWritten> {
        let m = fx.module;
        let mut given: Vec<Option<&Expr>> = vec![None; fields.len()];
        let mut next = 0;
        for a in args {
            match &a.name {
                None => {
                    if next < given.len() {
                        given[next] = Some(&a.value);
                    }
                    next += 1;
                }
                Some(n) => {
                    if let Some(i) = fields.iter().position(|f| f.name.name == n.name) {
                        given[i] = Some(&a.value);
                    }
                }
            }
        }
        let mut out = Vec::new();
        for (f, g) in fields.iter().zip(given) {
            let ft = self.declared_ty(owner, &format!("{prefix}{}", f.name.name));
            let x = match g.or(f.default.as_ref()) {
                Some(x) => x,
                None => return self.not_yet(fx, e, "a value missing its field"),
            };
            let in_owner = g.is_none();
            // A library's default is filled in by the caller, a literal (it has no expression types here).
            if in_owner && matches!(owner.owner, Owner::Library(_)) {
                // A function value or a list with elements: the library's own export makes it.
                if super::library::made_by_library(x, &ft) && matches!(self.item(owner).kind, ItemKind::Struct(_)) {
                    out.push(self.library_made_default(fx, e, owner, &f.name.name, &ft)?);
                    continue;
                }
                out.push(self.library_default(fx, e, x, &ft)?);
                continue;
            }
            let saved = fx.module;
            if in_owner {
                fx.module = owner.module;
            }
            let v = self.expr(fx, x);
            let from = self.ty(fx.module, x);
            fx.module = saved;
            let v = v?;
            out.push(self.coerce(fx, &v, &from, &ft, x.at)?);
        }
        let _ = m;
        Ok(out)
    }

    fn struct_lit(&mut self, fx: &mut Fx, e: &Expr, path: &[ast::Ident], fields: &[(ast::Ident, Option<Expr>)], t: &Ty) -> Ex {
        let m = fx.module;
        // Each field as an argument by name; a shorthand `{ x }` reads the name `x` (recorded at its place).
        let mut args = Vec::new();
        for (ident, value) in fields {
            let value = match value {
                Some(v) => v.clone(),
                None => {
                    let x = Expr { id: u32::MAX, at: ident.at, kind: ExprKind::Name { name: ident.clone(), targs: Vec::new() } };
                    match self.checked.named.get(&(m, ident.at)) {
                        Some(Resolved::Local(id)) => {
                            let place = fx.local(*id);
                            // Read straight from the local: the shorthand has no expression of its own in the tree.
                            args.push((ident.clone(), Err(place)));
                            let _ = x;
                            continue;
                        }
                        Some(Resolved::Item(item)) => {
                            args.push((ident.clone(), Err(self.item_value(*item))));
                            continue;
                        }
                        _ => return self.not_yet(fx, e, "this shorthand field"),
                    }
                }
            };
            args.push((ident.clone(), Ok(value)));
        }
        let (item, prefix, decl_fields, case) = match t {
            Ty::Struct(item) => {
                let ItemKind::Struct(s) = &self.item(*item).kind else { return Err(lost()) };
                (*item, ".".to_string(), s.fields.clone(), None)
            }
            Ty::Enum(item) => {
                let case = match self.checked.named.get(&(m, path[path.len() - 1].at)) {
                    Some(Resolved::Case { case, .. }) => case.clone(),
                    _ => return self.not_yet(fx, e, "this case"),
                };
                let ItemKind::Enum(d) = &self.item(*item).kind else { return Err(lost()) };
                let c = d.cases.iter().find(|c| c.name.name == case).ok_or_else(lost)?;
                (*item, format!("{case}."), c.fields.clone(), Some(case))
            }
            Ty::Service { service, name } => {
                let (service, name) = (service.clone(), name.clone());
                return self.service_struct_lit(fx, e, &args, &service, &name);
            }
            _ => return self.not_yet(fx, e, "this literal"),
        };
        let ct = self.c_type(t).ok_or_else(lost)?;
        // Fields in the order written (their reads and errors), then the defaults of the rest in declared order.
        let mut values: Vec<Option<String>> = vec![None; decl_fields.len()];
        for (ident, value) in &args {
            let Some(i) = decl_fields.iter().position(|f| f.name.name == ident.name) else { continue };
            let ft = self.declared_ty(item, &format!("{prefix}{}", ident.name));
            let v = match value {
                Ok(x) => {
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    self.coerce(fx, &v, &from, &ft, x.at)?
                }
                Err(place) => place.clone(),
            };
            values[i] = Some(fx.temp(&self.c_type(&ft).ok_or_else(lost)?, &v));
        }
        for (i, f) in decl_fields.iter().enumerate() {
            if values[i].is_some() {
                continue;
            }
            let Some(d) = &f.default else { return self.not_yet(fx, e, "a value missing its field") };
            let ft = self.declared_ty(item, &format!("{prefix}{}", f.name.name));
            // A library's struct or case: its default is a literal the caller fills in (its expression was not
            // checked in this mod).
            if matches!(item.owner, Owner::Library(_)) {
                // A function value or a list with elements: the library's own export makes it.
                if super::library::made_by_library(d, &ft) && matches!(self.item(item).kind, ItemKind::Struct(_)) {
                    values[i] = Some(self.library_made_default(fx, e, item, &f.name.name, &ft)?);
                    continue;
                }
                values[i] = Some(self.library_default(fx, e, d, &ft)?);
                continue;
            }
            let saved = fx.module;
            fx.module = item.module;
            let v = self.expr(fx, d);
            let from = self.ty(fx.module, d);
            fx.module = saved;
            let v = v?;
            values[i] = Some(self.coerce(fx, &v, &from, &ft, d.at)?);
        }
        let r = fx.fresh(&ct);
        if counted(&ct) {
            fx.emit(format!("lm_rel_{ct}(&{r});"));
        }
        match case {
            None => {
                for (f, v) in decl_fields.iter().zip(values) {
                    let fc = self.field_c(item, &format!("{prefix}{}", f.name.name))?;
                    fx.set(&format!("{r}.{}", f.name.name), &v.unwrap_or_default(), &fc);
                }
            }
            Some(case) => {
                let tag = self.case_tag(item, &case).ok_or_else(lost)?;
                fx.emit(format!("{r}.tag = {tag}u;"));
                for (f, v) in decl_fields.iter().zip(values) {
                    let fc = self.field_c(item, &format!("{prefix}{}", f.name.name))?;
                    fx.set(&format!("{r}.u.{case}.{}", f.name.name), &v.unwrap_or_default(), &fc);
                }
            }
        }
        Ok(r)
    }

    // ------------------------------------------------------------ calls

    fn call(&mut self, fx: &mut Fx, e: &Expr, callee: &Expr, args: &[ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let res = self.resolved(m, e).or_else(|| self.resolved(m, callee)).cloned();
        if let ExprKind::Member { base, optional: true, .. } = &callee.kind {
            return self.optional_call(fx, e, base, res, args, t);
        }
        match res {
            Some(Resolved::Case { item, case }) => self.case_value(fx, e, item, &case, args),
            Some(Resolved::Item(item)) if item.owner == Owner::Mod => match &self.item(item).kind {
                ItemKind::Fn(f) => {
                    let f = f.clone();
                    let c_name = self.item_c(item, &f.name.name);
                    self.call_fn(fx, e, item, "", &c_name, &f, None, args)
                }
                // Language 13: the mod's C function, called directly.
                ItemKind::Extern { name, params, ret } => {
                    let (name, params, ret) = (name.name.clone(), params.clone(), ret.is_some());
                    self.call_extern(fx, e, item, &name, &params, ret, args)
                }
                // An exported function called from the script: the script's own function (docs/modding/language/compiled-form.md, section 2's `lm_m__f`).
                ItemKind::Export { name, params, ret, body } => {
                    let f = ast::FnDecl { name: name.clone(), params: params.clone(), ret: ret.clone(), body: body.clone() };
                    let c_name = self.item_c(item, &f.name.name);
                    self.call_fn(fx, e, item, "", &c_name, &f, None, args)
                }
                // A module `let` or `var` holding a function value.
                ItemKind::Binding(_) if matches!(self.ty(m, callee), Ty::Fn(..)) => self.call_fn_value(fx, e, callee, args),
                // A task: started, its pending the value (11.1a), or `()` when it ends by itself.
                ItemKind::Task { .. } => self.task_pending(fx, e, item, args),
                // C6: `T()`, the struct of its defaults.
                ItemKind::Struct(_) => self.struct_lit(fx, e, &[], &[], t),
                _ => self.not_yet(fx, e, "a call of this item"),
            },
            // A needed library mod's function, through its kind 6 import (docs/modding/language/compiled-form.md).
            Some(Resolved::Item(item)) if matches!(item.owner, Owner::Library(_)) => match &self.item(item).kind {
                ItemKind::Fn(f) => {
                    let f = f.clone();
                    self.call_library(fx, e, item, "", None, &f, args, false)
                }
                // A library's task: its export starts it and gives its pending (11.1a).
                ItemKind::Task { name, params, body, .. } => {
                    let f = ast::FnDecl { name: name.clone(), params: params.clone(), ret: None, body: body.clone() };
                    self.call_library(fx, e, item, "", None, &f, args, true)
                }
                ItemKind::Struct(_) => self.struct_lit(fx, e, &[], &[], t),
                _ => self.not_yet(fx, e, "a call of this item of another mod"),
            },
            // A needed library's struct's function: `v.f(...)` through its kind 6 import, v its `self`.
            Some(Resolved::Member { owner, name }) if matches!(owner.owner, Owner::Library(_)) && matches!(self.item(owner).kind, ItemKind::Struct(_)) => {
                let ItemKind::Struct(st) = &self.item(owner).kind else { return Err(lost()) };
                let Some(f) = st.fns.iter().find(|f| f.name.name == name).cloned() else {
                    if matches!(self.ty(m, callee), Ty::Fn(..)) {
                        return self.call_fn_value(fx, e, callee, args);
                    }
                    return self.not_yet(fx, e, "this struct function of another mod");
                };
                let recv = match &callee.kind {
                    ExprKind::Member { base, .. } if matches!(self.ty(m, base), Ty::Struct(_)) => Some(&**base),
                    _ => None,
                };
                self.call_library(fx, e, owner, &format!(".{name}"), recv, &f, args, false)
            }
            // A struct's function: `v.f(...)` (self is v) or `S.f(...)` (no self).
            Some(Resolved::Member { owner, name }) if owner.owner == Owner::Mod && matches!(self.item(owner).kind, ItemKind::Struct(_)) => {
                let ItemKind::Struct(st) = &self.item(owner).kind else { return Err(lost()) };
                let Some(f) = st.fns.iter().find(|f| f.name.name == name).cloned() else {
                    // A field of function type, called (5.4).
                    if matches!(self.ty(m, callee), Ty::Fn(..)) {
                        return self.call_fn_value(fx, e, callee, args);
                    }
                    return self.not_yet(fx, e, "this struct function");
                };
                let c_name = self.struct_fn_c(owner, &f.name.name);
                let receiver = match &callee.kind {
                    ExprKind::Member { base, .. } if matches!(self.ty(m, base), Ty::Struct(_)) => Some(Recv::Expr(base)),
                    _ => None,
                };
                self.call_fn(fx, e, owner, &format!(".{name}"), &c_name, &f, receiver, args)
            }
            Some(Resolved::GameFn { name, signature }) => {
                let receiver = match &callee.kind {
                    ExprKind::Member { base, .. } => Some(Recv::Expr(base)),
                    _ => None,
                };
                self.call_game(fx, e, receiver, &name, &signature, args, t)
            }
            Some(Resolved::Std("is_state")) => self.is_state_value(fx, e, args),
            Some(Resolved::Std(s)) => self.call_std(fx, e, s, args, t),
            Some(Resolved::Table { write, name, offset, indexes, leaf }) => self.table_place(fx, e, args, write, &name, offset, &indexes, &leaf, t),
            Some(Resolved::Method(name)) => match &callee.kind {
                ExprKind::Member { base, .. } => self.call_method(fx, e, base, name, args, t),
                _ => Err(lost()),
            },
            Some(Resolved::BaseCall) => self.base_call(fx, e, args),
            // A service's function (emit/service.rs).
            Some(Resolved::ServiceFn { service, name, waited }) => self.call_service(fx, e, &service, &name, waited, args),
            // A value of function type: a local, a field, an element, any expression (5.4; closure.rs).
            _ if matches!(self.ty(m, callee), Ty::Fn(..)) => self.call_fn_value(fx, e, callee, args),
            _ => self.not_yet(fx, e, "a call of this kind"),
        }
    }

    /// `a?.f(...)`: `a` read once; when it is none nothing is called and the result is none; else `f` is
    /// called on its value, and its result made an option unless it is one already (docs/modding/language/language.md, section 3: `?.` does not nest
    /// options).
    fn optional_call(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, res: Option<Resolved>, args: &[ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let bt = self.ty(m, base);
        let Ty::Opt(x) = &bt else { return self.not_yet(fx, e, "?. on this type") };
        let x = (**x).clone();
        // What the call gives on a value, and how the value is passed.
        enum Target {
            Game { name: String, signature: String, inner: Ty },
            Method { name: &'static str, inner: Ty },
            Struct { owner: ItemRef, f: ast::FnDecl, var_self: bool, inner: Ty },
        }
        let target = match res {
            Some(Resolved::GameFn { name, signature }) => {
                let Ok(fnty) = self.game.call_signature(&name, &signature) else { return self.not_yet(fx, e, "a game call whose signature does not cross") };
                let inner = match (&fnty.result, t) {
                    (GameTy::Opt(_), _) => t.clone(),
                    (_, Ty::Opt(y)) => (**y).clone(),
                    _ => t.clone(),
                };
                Target::Game { name, signature, inner }
            }
            Some(Resolved::Member { owner, name }) if owner.owner == Owner::Mod && matches!(self.item(owner).kind, ItemKind::Struct(_)) => {
                let ItemKind::Struct(st) = &self.item(owner).kind else { return Err(lost()) };
                let Some(f) = st.fns.iter().find(|f| f.name.name == name).cloned() else { return self.not_yet(fx, e, "this struct function") };
                let inner = self.checked.types.get(&(owner, format!(".{name}->"))).cloned().unwrap_or(Ty::Unit);
                let var_self = f.params.iter().any(|p| p.is_self && p.var);
                Target::Struct { owner, f, var_self, inner }
            }
            // A builtin method that reads its receiver (text, json.Value): an option it gives stays one.
            Some(Resolved::Method(name)) if matches!(x, Ty::Text | Ty::Json) => {
                let gives_opt = x == Ty::Json || matches!(name, "find" | "parse_int" | "parse_float");
                let inner = match t {
                    Ty::Opt(y) if !gives_opt => (**y).clone(),
                    _ => t.clone(),
                };
                Target::Method { name, inner }
            }
            // A handle's method (`h?.get()`): get gives an option already (none for an emptied handle).
            Some(Resolved::Method(name)) if matches!(x, Ty::Ref(_)) => {
                let inner = match t {
                    Ty::Opt(y) if name != "get" => (**y).clone(),
                    _ => t.clone(),
                };
                Target::Method { name, inner }
            }
            _ => return self.not_yet(fx, e, "?. on a call of this kind"),
        };
        // The value: a view is its address; an option of a value is read once, or by its place for `var self`.
        let var_self = matches!(target, Target::Struct { var_self: true, .. });
        let (has, value) = if is_view(&x) {
            let b = self.expr(fx, base)?;
            let a = fx.temp("void*", &b);
            (format!("{a} != 0"), a)
        } else if var_self {
            let p = self.place(fx, base)?;
            (format!("{p}.some"), format!("&{p}.value"))
        } else {
            let ct = self.c_type(&bt).ok_or_else(lost)?;
            let b = self.expr(fx, base)?;
            let a = fx.temp(&ct, &b);
            (format!("{a}.some"), format!("{a}.value"))
        };
        let r = match t {
            Ty::Unit | Ty::Never | Ty::Error => None,
            _ => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                Some((fx.fresh(&ct), ct))
            }
        };
        fx.emit(format!("if ({has}) {{"));
        fx.depth += 1;
        let (v, inner) = match target {
            Target::Game { name, signature, inner } => (self.call_game(fx, e, Some(Recv::Value(value)), &name, &signature, args, &inner), inner),
            Target::Method { name, inner } => {
                self.pre_read = Some((base.id, value));
                let v = self.call_method(fx, e, base, name, args, &inner);
                self.pre_read = None;
                (v, inner)
            }
            Target::Struct { owner, f, inner, .. } => {
                let c_name = self.struct_fn_c(owner, &f.name.name);
                let key = format!(".{}", f.name.name);
                (self.call_fn(fx, e, owner, &key, &c_name, &f, Some(Recv::Value(value)), args), inner)
            }
        };
        let v = match v {
            Ok(v) => v,
            Err(n) => {
                fx.depth -= 1;
                fx.emit("}".into());
                return Err(n);
            }
        };
        if let Some((r, ct)) = &r {
            // An option of a view is the pointer itself: the view is stored as it is, null being none.
            if inner == *t || matches!(t, Ty::Opt(y) if is_view(y)) {
                fx.set(r, &v, ct);
            } else {
                fx.set(r, &format!("({ct}){{1, {v}}}"), ct);
            }
        }
        fx.depth -= 1;
        match &r {
            Some((r, ct)) => {
                let none = if matches!(t, Ty::Opt(y) if is_view(y)) { "0".to_string() } else { format!("({ct}){{0}}") };
                fx.emit("} else {".into());
                fx.depth += 1;
                fx.set(r, &none, ct);
                fx.depth -= 1;
                fx.emit("}".into());
            }
            None => fx.emit("}".into()),
        }
        Ok(r.map(|(r, _)| r).unwrap_or_default())
    }

    /// A mod's function (a struct's: `key` ".f", `receiver` its `self`): its arguments read in the order written,
    /// a `var` one by its place's address; a script error in it ends this run too (it has reported).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_fn(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, key: &str, c_name: &str, f: &ast::FnDecl, receiver: Option<Recv>, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        // `start`'s call: taken before the arguments, which may hold calls of their own.
        let start = std::mem::take(&mut self.start_ret);
        // `self` first, read before the arguments as written (`p.grow(x)`: p, then x).
        let mut list: Vec<String> = Vec::new();
        // The counted copies made for the call (parameters are borrowed), given back as soon as it returns: a count
        // held past it would make the caller's next write to that container copy it.
        let mut held: Vec<(String, String)> = Vec::new();
        if let Some(sp) = f.params.iter().find(|p| p.is_self) {
            let v = match receiver {
                None => return self.not_yet(fx, e, "a struct function called without its value"),
                // Read already (`a?.f()`): the value, or for `var self` its place's address.
                Some(Recv::Value(v)) => v,
                Some(Recv::Expr(base)) => {
                    let st = self.ty(m, base);
                    if sp.var {
                        format!("&{}", self.place(fx, base)?)
                    } else {
                        let v = self.expr(fx, base)?;
                        let ct = self.c_type(&st).ok_or_else(lost)?;
                        let t = fx.temp(&ct, &v);
                        held.push((t.clone(), ct));
                        t
                    }
                }
            };
            list.push(v);
        }
        // The parameters a call gives (self is not one), by their place in the declaration.
        let params: Vec<(usize, &ast::Param)> = f.params.iter().enumerate().filter(|(_, p)| !p.is_self).collect();
        let pkey = |i: usize, p: &ast::Param| format!("{key}({})", p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone()));
        let mut slots: Vec<Option<String>> = vec![None; params.len()];
        let mut next = 0;
        for a in args {
            let k = match &a.name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(n) => params.iter().position(|(_, p)| p.name.as_ref().is_some_and(|pn| pn.name == n.name)).ok_or_else(lost)?,
            };
            let (i, p) = *params.get(k).ok_or_else(lost)?;
            let pt = self.declared_ty(item, &pkey(i, p));
            let v = if p.var {
                format!("&{}", self.place(fx, &a.value)?)
            } else {
                let v = self.expr(fx, &a.value)?;
                let from = self.ty(m, &a.value);
                let v = self.coerce(fx, &v, &from, &pt, a.value.at)?;
                let ct = self.c_type(&pt).ok_or_else(lost)?;
                let t = fx.temp(&ct, &v);
                held.push((t.clone(), ct));
                t
            };
            slots[k] = Some(v);
        }
        for (k, (i, p)) in params.iter().enumerate() {
            if slots[k].is_some() {
                continue;
            }
            let Some(d) = &p.default else { return self.not_yet(fx, e, "a call missing an argument") };
            let pt = self.declared_ty(item, &pkey(*i, p));
            let saved = fx.module;
            fx.module = item.module;
            let v = self.expr(fx, d);
            let from = self.ty(fx.module, d);
            fx.module = saved;
            let v = v?;
            slots[k] = Some(self.coerce(fx, &v, &from, &pt, d.at)?);
        }
        list.extend(slots.into_iter().map(Option::unwrap_or_default));
        let r = match self.checked.types.get(&(item, format!("{key}->"))) {
            // A task's start function gives its handle.
            _ if start => {
                let r = fx.fresh("lumart_task");
                list.push(format!("&{r}"));
                Some(r)
            }
            Some(Ty::Unit) | None => None,
            Some(rt) => {
                let rt = rt.clone();
                let ct = self.c_type(&rt).ok_or_else(lost)?;
                let r = fx.fresh(&ct);
                list.push(format!("&{r}"));
                Some(r)
            }
        };
        fx.fail_if(&format!("{c_name}({}) != 0", list.join(", ")));
        for (t, ct) in held {
            if super::counted(&ct) {
                fx.emit(format!("lm_rel_{ct}(&{t});"));
            }
        }
        Ok(r.unwrap_or_default())
    }

    /// A game function through its slot (docs/modding/language/compiled-form.md, section 5): `args[0]` the object (or null), `args[n]` the address of each
    /// argument's copy (a reference: the place's own address; a pointer: the address of the pointer), the result
    /// into a temporary.
    #[allow(clippy::too_many_arguments)]
    fn call_game(&mut self, fx: &mut Fx, e: &Expr, receiver: Option<Recv>, name: &str, signature: &str, args: &[ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let Ok(fnty) = self.game.call_signature(name, signature) else { return self.not_yet(fx, e, "a game call whose signature does not cross") };
        let Ok(sig) = ctype::parse_signature(signature) else { return self.not_yet(fx, e, "a game call whose signature does not read") };
        if !fnty.counts.is_empty() {
            return self.not_yet(fx, e, "a game call with a counted list");
        }
        // One argument per parameter: the invoker reads exactly that many.
        if args.len() != fnty.params.len() {
            return self.not_yet(fx, e, "a game call with another number of arguments than its parameters");
        }
        // docs/modding/language/objects.md: a member called on an object (`obj.f()`, `obj?.f()`, or by its bare name in a hook) that the
        // facts mark virtual runs the object's own class's function, as in C++ (kind 8); one written qualified
        // (`Record::f(..)`) is that row itself (kind 0). The record the object is seen as, when known: a view's, a
        // mod's class's game base, a hook's own record.
        let dispatch = match &e.kind {
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Member { .. } => true,
                ExprKind::Name { name: n, .. } => !n.name.contains("::"),
                _ => false,
            },
            _ => false,
        };
        let seen_as: Option<String> = match &receiver {
            Some(Recv::Expr(base)) => match self.ty(m, base) {
                Ty::Game(GameTy::View { record, .. }) => Some(record),
                Ty::Class(item) => match &self.item(item).kind {
                    ItemKind::Class(c) => Some(c.base.text.clone()),
                    _ => None,
                },
                _ => None,
            },
            Some(Recv::Value(_)) => None,
            None => fx.hook.as_ref().and_then(|h| h.name.rsplit_once("::").map(|(r, _)| r.to_string())),
        };
        // The object: written (`obj.f()`), read already (`obj?.f()`), or `self` for a member function called by
        // its name in a hook.
        let receiver = match receiver {
            Some(Recv::Expr(base)) => {
                let b = self.expr(fx, base)?;
                Some(fx.temp("void*", &b))
            }
            Some(Recv::Value(v)) => Some(v),
            None => {
                let method = self.game.facts.as_ref().and_then(|f| f.get(name, signature)).is_some_and(|f| matches!(f.kind, FunctionKind::Method));
                if method {
                    match &fx.hook {
                        Some(_) => Some("call->args[0]".to_string()),
                        None => return self.not_yet(fx, e, "a member function called on self outside a hook"),
                    }
                } else {
                    None
                }
            }
        };
        let mut slots = Vec::new();
        // The scratch's mark before the first text made for the call alone, given back once it returns.
        let mut scratch_mark: Option<String> = None;
        for (i, (a, p)) in args.iter().zip(fnty.params.iter()).enumerate() {
            let Some(c) = sig.params.get(fnty.c_index(i)).cloned() else { return self.not_yet(fx, e, "a game argument past its signature") };
            let at = &a.value;
            let from = self.ty(m, at);
            // A marked pointer (`T *` marked in, out or inout; nullable or not): the place it points at.
            let marked = match p {
                GameTy::Place { of, .. } => Some(of),
                GameTy::Opt(x) => match &**x {
                    GameTy::Place { of, .. } => Some(of),
                    _ => None,
                },
                _ => None,
            };
            let slot = match (p, &c) {
                // args[n] the address of the pointer (`T *`, `T x[]`, `MtxPtr`; luma::arg reads one): `var x` the
                // place's own address, a value a copy's, `none` null. A reference's is the place itself (below).
                (_, _) if marked.is_some() && !matches!(c, CType::Ref(_) | CType::RRef(_)) => {
                    let of = marked.ok_or_else(lost)?;
                    let addr = if a.var {
                        format!("(void*)&{}", self.place(fx, at)?)
                    } else if matches!(strip(at).kind, ExprKind::None) {
                        "(void*)0".to_string()
                    } else {
                        let want = game_scalar_ty(of).ok_or_else(lost)?;
                        let v = self.expr(fx, at)?;
                        let v = self.coerce(fx, &v, &from, &want, at.at)?;
                        let ct = self.c_type(&want).ok_or_else(lost)?;
                        let tmp = fx.temp(&ct, &v);
                        format!("(void*)&{tmp}")
                    };
                    let ptr = fx.temp("void*", &addr);
                    format!("&{ptr}")
                }
                (GameTy::Scalar(_) | GameTy::Value(_) | GameTy::Enum(_), _) | (GameTy::Place { konst: true, .. }, _) => {
                    let want = match p {
                        GameTy::Place { of, .. } => game_scalar_ty(of).ok_or_else(lost)?,
                        other => game_scalar_ty(other).ok_or_else(lost)?,
                    };
                    let v = self.expr(fx, at)?;
                    let v = self.coerce(fx, &v, &from, &want, at.at)?;
                    let ct = self.c_type(&want).ok_or_else(lost)?;
                    let tmp = fx.temp(&ct, &v);
                    format!("&{tmp}")
                }
                (GameTy::Place { konst: false, .. }, _) if a.var => format!("&{}", self.place(fx, at)?),
                // A `const char *` the game reads only during the call (`in`, docs/modding/language/compiled-form.md): a text made here
                // is written in the scratch and given back after the call (docs/modding/language/language.md, section 6.4), never kept; null for none,
                // a NUL byte inside S0918. A literal is passed as its static bytes (below).
                (GameTy::Opt(x), CType::Ptr { .. }) if **x == GameTy::Text && fnty.transient.contains(&fnty.c_index(i)) && !text_literal_arg(at) => {
                    let (b, mark) = fx.scratch_builder();
                    scratch_mark.get_or_insert(mark);
                    let k = fx.fresh("const char*");
                    let done = format!("lumart_sb_done(&{b}, {}, {})", c_string(&fx.file), at.at.line);
                    let cstr = format!("{k} = lumart_sb_cstr(&{b}, {}, {}, {}, \"{}\");", c_string(&fx.file), at.at.line, c_string(name), i + 1);
                    if from == Ty::Text {
                        self.value_text_into(fx, &b, at, false)?;
                        fx.fail_if(&done);
                        fx.emit(cstr);
                        fx.fail_if(&format!("{k} == 0"));
                    } else {
                        let want = Ty::Opt(Box::new(Ty::Text));
                        let v = self.expr(fx, at)?;
                        let v = self.coerce(fx, &v, &from, &want, at.at)?;
                        let ct = self.c_type(&want).ok_or_else(lost)?;
                        let o = fx.temp(&ct, &v);
                        fx.emit(format!("if ({o}.some) {{"));
                        fx.depth += 1;
                        fx.emit(format!("lumart_sb_bytes(&{b}, lumart_text_p({o}.value), lumart_text_n({o}.value));"));
                        fx.fail_if(&done);
                        fx.emit(cstr);
                        fx.fail_if(&format!("{k} == 0"));
                        fx.depth -= 1;
                        fx.emit("}".into());
                    }
                    format!("&{k}")
                }
                // `const char *`: the text's kept copy (docs/modding/language/compiled-form.md: one per distinct text, never freed, so the
                // game may hold the pointer), null for none; a NUL byte inside is S0918.
                (GameTy::Opt(x), CType::Ptr { .. }) if **x == GameTy::Text => {
                    if !text_literal_arg(at) && !matches!(strip(at).kind, ExprKind::None) {
                        self.kept.push((fx.file.clone(), at.at.line, name.to_string()));
                    }
                    let want = Ty::Opt(Box::new(Ty::Text));
                    let v = self.expr(fx, at)?;
                    let v = self.coerce(fx, &v, &from, &want, at.at)?;
                    let ct = self.c_type(&want).ok_or_else(lost)?;
                    let o = fx.temp(&ct, &v);
                    let k = fx.fresh("const char*");
                    fx.emit(format!("{k} = 0;"));
                    fx.emit(format!("if ({o}.some) {{"));
                    fx.depth += 1;
                    fx.emit(format!("{k} = lumart_text_keep({o}.value, {}, {}, {}, \"{}\");", c_string(&fx.file), at.at.line, c_string(name), i + 1));
                    fx.fail_if(&format!("{k} == 0"));
                    fx.depth -= 1;
                    fx.emit("}".into());
                    format!("&{k}")
                }
                // `nullable` where nothing else crosses: the checker let only `none` through.
                (GameTy::Null, _) => {
                    if !matches!(strip(at).kind, ExprKind::None) {
                        return Err(lost());
                    }
                    let tmp = fx.temp("void*", "(void*)0");
                    format!("&{tmp}")
                }
                (GameTy::View { .. } | GameTy::Opt(_), CType::Ptr { .. }) => {
                    let v = self.expr(fx, at)?;
                    let tmp = fx.temp("void*", &v);
                    format!("&{tmp}")
                }
                // the script's copy of the record to a `const T &`: its bytes, bound for the
                // call (a copy with none yet made zeroed; out of memory S0911).
                (GameTy::View { record, konst: true }, CType::Ref(_)) if matches!(&from, Ty::Game(GameTy::Struct(r)) if r == record) => {
                    let record = record.clone();
                    let v = self.expr(fx, at)?;
                    let b = fx.temp("lumart_bytes", &v);
                    let size = self.size_slot(&record);
                    let p = fx.temp("void*", &format!("(void*)lumart_record_at(&{b}, {size}, 0, {}, {}u)", c_string(&fx.file), at.at.line));
                    fx.fail_if(&format!("{p} == 0"));
                    p
                }
                (GameTy::View { .. }, CType::Ref(_) | CType::RRef(_)) => {
                    let v = self.expr(fx, at)?;
                    fx.temp("void*", &v)
                }
                // A record by value: args[n] points at its bytes, the game's object itself when the argument is one
                // (the callee takes its own copy), else the script's copy.
                (GameTy::Struct(_), _) => match self.game_place_of(fx, at) {
                    Some(Ok(super::game::GamePlace::Address(a))) => fx.temp("void*", &format!("(void*)({a})")),
                    Some(Ok(_)) => return self.not_yet(fx, e, "a game record argument of this kind"),
                    Some(Err(x)) => return Err(x),
                    None => {
                        let v = self.expr(fx, at)?;
                        let b = fx.temp("lumart_bytes", &v);
                        fx.temp("void*", &format!("(void*)lumart_bytes_p({b})"))
                    }
                },
                _ => return self.not_yet(fx, e, &format!("a game argument of type {p:?}")),
            };
            slots.push(slot);
        }
        // A virtual: LumaCore.method's function, args[0] the object as the row's record (core->as_class unless that
        // part is at the object's own address).
        let virtual_of = match (&receiver, dispatch) {
            (Some(_), true) => self.game.dispatched(name, signature),
            _ => None,
        };
        let (slot, this) = match (virtual_of, receiver) {
            (Some(record), Some(recv)) => {
                let this = if seen_as.as_deref().is_some_and(|s| self.game.same_address(s, &record)) {
                    recv
                } else {
                    let class = self.class_slot(&record);
                    fx.temp("void*", &format!("(void*)luma_rt_core->as_class(luma_rt_self, {recv}, {class})"))
                };
                (self.method_slot(name, signature), this)
            }
            (_, recv) => (self.slot(name, signature), recv.unwrap_or_else(|| "(void*)0".into())),
        };
        let argv = fx.array("void*", 1 + slots.len());
        fx.emit(format!("{argv}[0] = {this};"));
        for (i, s) in slots.iter().enumerate() {
            fx.emit(format!("{argv}[{}] = (void*){s};", i + 1));
        }
        let (r, out) = match &fnty.result {
            GameTy::Unit => {
                let d = fx.fresh("uint64_t");
                (String::new(), format!("&{d}"))
            }
            GameTy::Scalar(_) | GameTy::Value(_) | GameTy::Enum(_) => {
                let rt = game_scalar_ty(&fnty.result).ok_or_else(lost)?;
                let ct = self.c_type(&rt).ok_or_else(lost)?;
                let r = fx.fresh(&ct);
                (r.clone(), format!("&{r}"))
            }
            GameTy::View { .. } | GameTy::Opt(_) => {
                let r = fx.fresh("void*");
                (r.clone(), format!("&{r}"))
            }
            // A record by value: made in a new block of its size row's bytes, the script's copy.
            GameTy::Struct(record) => {
                let size = self.size_slot(record);
                let r = fx.fresh("lumart_bytes");
                fx.fail_if(&format!("lumart_record_new(&{r}, {size}, {}, {})", c_string(&fx.file), e.at.line));
                (r.clone(), format!("(void*)lumart_bytes_p({r})"))
            }
            // A reference to a number or a vector (`const TVec3f &`): the address it gives, read once the call
            // returns (below).
            GameTy::Place { of, .. } if place_result_c(of).is_some() => {
                let r = fx.fresh("void*");
                (r.clone(), format!("&{r}"))
            }
            other => return self.not_yet(fx, e, &format!("a game result of type {other:?}")),
        };
        fx.emit(format!("{slot}->invoke({argv}, {out});"));
        if let Some(mark) = scratch_mark {
            fx.emit(format!("lumart_scratch_release({mark});"));
        }
        // The value at the reference, copied: the game's storage may change or go after the call (a script keeps no
        // place, D4.7's reads).
        if let GameTy::Place { of, .. } = &fnty.result {
            let pc = place_result_c(of).ok_or_else(lost)?;
            let ct = self.c_type(t).ok_or_else(lost)?;
            return Ok(fx.temp(&ct, &format!("(*(const {pc}*){r})")));
        }
        // `const char *`: a new text of its bytes up to the NUL (the game's may change or go after the call), none
        // for null.
        if let (GameTy::Opt(g), Ty::Opt(_)) = (&fnty.result, t) {
            if **g == GameTy::Text {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let tx = fx.fresh("lumart_text");
                if fnty.text16_result {
                    fx.fail_if(&format!("lumart_text_from_game16(&{tx}, (const uint16_t*){r}, {}, {})", c_string(&fx.file), e.at.line));
                    return Ok(fx.temp(&ct, &format!("({ct}){{{r} != 0, {tx}}}")));
                }
                fx.fail_if(&format!("lumart_text_from_game(&{tx}, (const char*){r}, {}, {})", c_string(&fx.file), e.at.line));
                return Ok(fx.temp(&ct, &format!("({ct}){{{r} != 0, {tx}}}")));
            }
        }
        // A pointer to a number or vector (`Vec3f *`) is `T?` to the script: the value copied when the call returns,
        // none for a null pointer (docs/modding/language/compiled-form.md, section 5).
        if let (GameTy::Opt(_), Ty::Opt(x)) = (&fnty.result, t) {
            if !is_view(x) {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let inner = self.c_type(x).ok_or_else(lost)?;
                return Ok(fx.temp(&ct, &format!("{r} != 0 ? ({ct}){{1, *({inner}*){r}}} : ({ct}){{0}}")));
            }
        }
        Ok(r)
    }

    /// `set_table(...)` and `table(...)` (docs/modding/language/numbers.md) as the checker laid them out: the offset of
    /// the literal steps plus each computed index times its stride (an index outside its array is S0902), and the
    /// place's bytes as the game holds them (a bool one byte, an enum its storage, a colour four u8 channels).
    /// set_table: lumart_set_table for the calling mod, the call's bool; table: lumart_table, `T?`.
    #[allow(clippy::too_many_arguments)]
    fn table_place(&mut self, fx: &mut Fx, e: &Expr, args: &[ast::Arg], write: bool, name: &str, offset: u64, indexes: &[(usize, u64, u64)], leaf: &TableLeaf, t: &Ty) -> Ex {
        let m = fx.module;
        let file = c_string(&fx.file);
        let line = e.at.line;
        let mut off = format!("{offset}ull");
        for (i, stride, count) in indexes {
            let x = &args[*i].value;
            let iv = self.expr(fx, x)?;
            let from = self.ty(m, x);
            if !matches!(from, Ty::Int(_)) {
                return Err(lost());
            }
            let iv = fx.temp("int64_t", &format!("(int64_t)({iv})"));
            fx.fail_if(&format!("({iv} < 0 || (uint64_t){iv} >= {count}ull) && lumart_index_error({iv}, {count}ull, {file}, {line})"));
            off = format!("{off} + (uint64_t){iv} * {stride}ull");
        }
        let off = fx.temp("uint32_t", &format!("(uint32_t)({off})"));
        let table = c_string(name);
        let size = leaf.size();
        let held = match leaf {
            TableLeaf::Scalar(s) | TableLeaf::Enum(_, s) => scalar_c(*s),
            TableLeaf::Value(Value::Color) => "uint32_t",
            TableLeaf::Value(v) => value_c(*v),
        };
        let script = match leaf {
            TableLeaf::Scalar(s) => Ty::from_game(GameTy::Scalar(*s)),
            TableLeaf::Enum(n, _) => Ty::Game(GameTy::Enum(n.clone())),
            TableLeaf::Value(v) => Ty::Value(*v),
        };
        if write {
            let x = &args[args.len() - 1].value;
            let vv = self.expr(fx, x)?;
            let from = self.ty(m, x);
            let vv = self.coerce(fx, &vv, &from, &script, x.at)?;
            let bytes = match leaf {
                // r, g, b, a in that order in memory (every platform is little-endian).
                TableLeaf::Value(Value::Color) => {
                    let c = fx.temp("lm_color", &vv);
                    fx.temp("uint32_t", &format!("lumart_unit_byte({c}.r) | lumart_unit_byte({c}.g) << 8 | lumart_unit_byte({c}.b) << 16 | lumart_unit_byte({c}.a) << 24"))
                }
                TableLeaf::Value(_) => fx.temp(held, &vv),
                _ => fx.temp(held, &format!("({held})({vv})")),
            };
            let r = fx.fresh("uint8_t");
            fx.fail_if(&format!("lumart_set_table({table}, {off}, &{bytes}, {size}u, &{r}, {file}, {line})"));
            return Ok(r);
        }
        let buf = fx.fresh(held);
        let some = fx.fresh("uint8_t");
        fx.fail_if(&format!("lumart_table({table}, {off}, &{buf}, {size}u, &{some}, {file}, {line})"));
        let value = match leaf {
            TableLeaf::Value(Value::Color) => format!(
                "(lm_color){{(float)({buf} & 255u) / 255.0f, (float)(({buf} >> 8) & 255u) / 255.0f, (float)(({buf} >> 16) & 255u) / 255.0f, (float)({buf} >> 24) / 255.0f}}"
            ),
            TableLeaf::Enum(..) => format!("(int32_t){buf}"),
            _ => buf.clone(),
        };
        let ct = self.c_type(t).ok_or_else(lost)?;
        Ok(fx.temp(&ct, &format!("{some} ? ({ct}){{1, {value}}} : ({ct}){{0}}")))
    }

    /// The standard functions that are values and arithmetic.
    fn call_std(&mut self, fx: &mut Fx, e: &Expr, s: &str, args: &[ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let mut vals = Vec::new();
        for a in args {
            let v = self.expr(fx, &a.value)?;
            let from = self.ty(m, &a.value);
            let v = match (&from, t, s) {
                // Literals the checker typed to the call's own type; vectors' components are f32.
                (_, _, "vec2" | "vec3" | "vec4" | "quat" | "color") => self.coerce(fx, &v, &from, &Ty::Float(Scalar::F32), a.value.at)?,
                _ => v,
            };
            let ct = self.c_type(&from).ok_or_else(lost)?;
            vals.push((fx.temp(&ct, &v), from));
        }
        let file = c_string(&fx.file);
        let line = e.at.line;
        let src = c_string(&expr_text(e));
        let v = |i: usize| vals[i].0.clone();
        match s {
            // C2: content by a name made at run time; none for an empty text.
            "model_named" | "effect_named" | "sound_named" | "layout_named" | "font_named" | "movie_named" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let r = fx.fresh(&ct);
                let x = v(0);
                fx.emit(format!("if ({x} != 0 && {x}->len > 0) {{ {r}.some = 1; lm_cp_lumart_text(&{r}.value, {x}); }}"));
                Ok(r)
            }
            // The docs/modding/language/classes.md's make (lumart_make): the objects' LiveActor parts, a new list; one object unless
            // the count is given.
            "make" => {
                let Ty::List(x) = t else { return Err(lost()) };
                let d = self.desc(x).ok_or_else(lost)?;
                let class = self.class_slot("LiveActor");
                let p = v(1);
                let count = if vals.len() == 3 { format!("(int64_t){}", v(2)) } else { "1".into() };
                let r = fx.fresh("lumart_list");
                fx.emit(format!("lm_rel_lumart_list(&{r});"));
                fx.fail_if(&format!(
                    "lumart_make({}, {p}.at, {p}.rotation, {p}.scale, {p}.args, {p}.dead, {count}, {class}, {d}, &{r}, {file}, {line})",
                    v(0)
                ));
                Ok(r)
            }
            // The docs/modding/language/numbers.md's set_number (lumart_set_number), for the calling mod: a file's constant (signature
            // "", place 0) or a function's place-th number.
            "set_number" => {
                let r = fx.fresh("uint8_t");
                let (sig, place, value) = if vals.len() == 4 {
                    (v(1), format!("(uint32_t){}", v(2)), v(3))
                } else {
                    ("0".to_string(), "0u".to_string(), v(1))
                };
                fx.fail_if(&format!("lumart_set_number({}, {sig}, {place}, (double){value}, &{r}, {file}, {line})", v(0)));
                Ok(r)
            }
            // The docs/modding/language/objects.md's as_class<T>(x): the object's T part by the kernel's as_class (a game
            // class's number, kind 3, or a mod class's own, define_class's), null (none) when it is no T or x is none.
            "as_class" => {
                let x = v(0);
                let object = match &vals[0].1 {
                    Ty::Opt(inner) if !is_view(inner) => fx.temp("void*", &format!("({x}.some ? {x}.value : (void*)0)")),
                    _ => fx.temp("void*", &format!("(void*)({x})")),
                };
                let Ty::Opt(target) = t else { return Err(lost()) };
                let class = match &**target {
                    Ty::Game(GameTy::View { record, .. }) => {
                        let record = record.clone();
                        self.class_slot(&record)
                    }
                    Ty::Class(item) => {
                        let ItemKind::Class(c) = &self.item(*item).kind else { return Err(lost()) };
                        let name = c.name.name.clone();
                        format!("{}_n", self.item_c(*item, &name))
                    }
                    _ => return Err(lost()),
                };
                let p = fx.temp("void*", &format!("{object} == 0 ? (void*)0 : (void*)luma_rt_core->as_class(luma_rt_self, {object}, {class})"));
                let ct = self.c_type(t).ok_or_else(lost)?;
                if ct == "void*" {
                    return Ok(p);
                }
                Ok(fx.temp(&ct, &format!("{p} != 0 ? ({ct}){{1, {p}}} : ({ct}){{0}}")))
            }
            // The docs/modding/language/classes.md's alias_class (lumart_alias_class), for the calling mod.
            "alias_class" => {
                let r = fx.fresh("uint8_t");
                fx.fail_if(&format!("lumart_alias_class({}, {}, {}, &{r}, {file}, {line})", v(0), v(1), v(2)));
                Ok(r)
            }
            // The docs/modding/language/classes.md's placement (lumart_placement_of): the std module's Placement?, its texts new.
            "placement_of" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let r = fx.fresh(&ct);
                fx.fail_if(&format!(
                    "lumart_placement_of({}, &{r}.some, &{r}.value.stage, &{r}.value.zone, &{r}.value.layer, &{r}.value.index, {file}, {line})",
                    v(0)
                ));
                Ok(r)
            }
            // The object made from a placement, as its LiveActor part (a view's option is the pointer).
            "placed_at" => {
                let class = self.class_slot("LiveActor");
                let p = v(0);
                let r = fx.fresh("void*");
                fx.fail_if(&format!("lumart_placed_at({p}.stage, {p}.zone, {p}.layer, {p}.index, {class}, &{r}, {file}, {line})"));
                Ok(r)
            }
            // The docs/modding/language/classes.md's gravity (lumart_make_gravity): the class by its own name, the object as the class
            // (its number), `C?`'s pair.
            "make_gravity" => {
                let Ty::Opt(c) = t else { return Err(lost()) };
                let Ty::Class(item) = &**c else { return Err(lost()) };
                let ItemKind::Class(cd) = &self.item(*item).kind else { return Err(lost()) };
                let name = cd.name.name.clone();
                let number = format!("{}_n", self.item_c(*item, &name));
                let ct = self.c_type(t).ok_or_else(lost)?;
                let p = v(0);
                let o = fx.fresh("void*");
                fx.fail_if(&format!(
                    "lumart_make_gravity({}, {p}.range, {p}.priority, {p}.types, {p}.inverse, {number}, &{o}, {file}, {line})",
                    c_string(&name)
                ));
                Ok(fx.temp(&ct, &format!("{o} != 0 ? ({ct}){{1, {o}}} : ({ct}){{0}}")))
            }
            // C4: lm_mat34's rows.
            "mat34" => {
                let row = |r: String| format!("{{{r}.x, {r}.y, {r}.z, {r}.w}}");
                Ok(fx.temp("lm_mat34", &format!("(lm_mat34){{{{{}, {}, {}}}}}", row(v(0)), row(v(1)), row(v(2)))))
            }
            "vec2" | "vec3" | "vec4" | "quat" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let list: Vec<String> = (0..vals.len()).map(v).collect();
                Ok(fx.temp(&ct, &format!("({ct}){{{}}}", list.join(", "))))
            }
            "color" => {
                let mut list: Vec<String> = (0..vals.len()).map(|i| clamp01(&v(i))).collect();
                if list.len() == 3 {
                    list.push("1.0f".into());
                }
                Ok(fx.temp("lm_color", &format!("(lm_color){{{}}}", list.join(", "))))
            }
            "json.encode" => {
                let (x, xt) = vals[0].clone();
                self.json_encode(fx, e, &x, &xt, &args[0].value)
            }
            "json.decode" => {
                let (x, xt) = vals[0].clone();
                self.json_decode(fx, e, &x, &xt, t)
            }
            "min" | "max" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let o = if s == "min" { "<" } else { ">" };
                Ok(fx.temp(&ct, &format!("({} {o} {}) ? {} : {}", v(0), v(1), v(0), v(1))))
            }
            "clamp" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                Ok(fx.temp(&ct, &format!("{} < {} ? {} : ({} > {} ? {} : {})", v(0), v(1), v(1), v(0), v(2), v(2), v(0))))
            }
            "abs" => match t {
                Ty::Int(sc) => {
                    let ct = scalar_c(*sc);
                    fx.fail_if(&format!("{} == {} && lumart_overflow({file}, {line}, \"{}\", {src})", v(0), limit_c(*sc, false), scalar_name(*sc)));
                    Ok(fx.temp(ct, &format!("{} < 0 ? ({ct})-{} : {}", v(0), v(0), v(0))))
                }
                Ty::Float(sc) => Ok(fx.temp(scalar_c(*sc), &format!("{} < 0 ? -{} : {}", v(0), v(0), v(0)))),
                _ => self.not_yet(fx, e, "abs of this type"),
            },
            "lerp" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                match t {
                    Ty::Float(_) => Ok(fx.temp(&ct, &format!("{} + ({} - {}) * {}", v(0), v(1), v(0), v(2)))),
                    Ty::Value(val) if !components(*val).is_empty() => {
                        let parts: Vec<String> = components(*val).iter().map(|c| format!("{a}.{c} + ({b}.{c} - {a}.{c}) * {k}", a = v(0), b = v(1), k = v(2))).collect();
                        Ok(fx.temp(&ct, &format!("({ct}){{{}}}", parts.join(", "))))
                    }
                    _ => self.not_yet(fx, e, "lerp of this type"),
                }
            }
            "dot" => {
                let Ty::Value(val) = vals[0].1 else { return self.not_yet(fx, e, "dot of this type") };
                let parts: Vec<String> = components(val).iter().map(|c| format!("{}.{c} * {}.{c}", v(0), v(1))).collect();
                Ok(fx.temp("float", &parts.join(" + ")))
            }
            "cross" => {
                let (a, b) = (v(0), v(1));
                Ok(fx.temp("lm_vec3", &format!("(lm_vec3){{{a}.y * {b}.z - {a}.z * {b}.y, {a}.z * {b}.x - {a}.x * {b}.z, {a}.x * {b}.y - {a}.y * {b}.x}}")))
            }
            "length" | "normal" => {
                let Ty::Value(val) = vals[0].1 else { return self.not_yet(fx, e, "length of this type") };
                let sq: Vec<String> = components(val).iter().map(|c| format!("{}.{c} * {}.{c}", v(0), v(0))).collect();
                let len = fx.temp("float", &format!("lumart_sqrtf({})", sq.join(" + ")));
                if s == "length" {
                    return Ok(len);
                }
                // A zero vector's normal is zero (no NaN from 0/0).
                let ct = value_c(val);
                let parts: Vec<String> = components(val).iter().map(|c| format!("{len} > 0.0f ? {}.{c} / {len} : 0.0f", v(0))).collect();
                Ok(fx.temp(ct, &format!("({ct}){{{}}}", parts.join(", "))))
            }
            "sqrt" => match t {
                Ty::Float(Scalar::F64) => Ok(fx.temp("double", &format!("lumart_sqrt({})", v(0)))),
                _ => Ok(fx.temp("float", &format!("lumart_sqrtf({})", v(0)))),
            },
            // lumart's own (fdlibm's method, no C library): within an ulp, the same on every platform.
            "sin" | "cos" => match t {
                Ty::Float(Scalar::F64) => Ok(fx.temp("double", &format!("lumart_{s}((double){})", v(0)))),
                _ => Ok(fx.temp("float", &format!("lumart_{s}f((float){})", v(0)))),
            },
            "atan2" => match t {
                Ty::Float(Scalar::F64) => Ok(fx.temp("double", &format!("lumart_atan2((double){}, (double){})", v(0), v(1)))),
                _ => Ok(fx.temp("float", &format!("lumart_atan2f((float){}, (float){})", v(0), v(1)))),
            },
            // Two views (or optional views) of one object (docs/modding/language/language.md, section 6.2): of one record, the same address; of a base
            // and a class made from it, the same base subobject (core->as_class: a base need not be at offset 0).
            "same" if vals.iter().all(|(_, t)| view_record(t).is_some()) => {
                let (ra, rb) = (view_record(&vals[0].1).unwrap_or_default(), view_record(&vals[1].1).unwrap_or_default());
                if ra == rb {
                    return Ok(format!("((uint8_t)({} == {}))", v(0), v(1)));
                }
                let base = if self.derives(&ra, &rb) { rb } else { ra };
                let class = self.class_slot(&base);
                let cast = |x: String| format!("luma_rt_core->as_class(luma_rt_self, {x}, {class})");
                Ok(fx.temp("uint8_t", &format!("({a} == 0 || {b} == 0) ? (uint8_t)({a} == {b}) : (uint8_t)({} == {})", cast(v(0)), cast(v(1)), a = v(0), b = v(1))))
            }
            // Two of a service's handles (or optional ones), of one type (the checker's rule): the same handle; two
            // nones are the same, as two null views are.
            "same" => {
                let mut sides = Vec::new();
                for (x, xt) in &vals {
                    let (handle, opt) = match xt {
                        Ty::Opt(y) => (&**y, true),
                        y => (y, false),
                    };
                    let Ty::Service { service, name } = handle else { return self.not_yet(fx, e, "same() of these values") };
                    // A handle (uint64_t) or an enum (int32_t), compared as numbers; a struct is not a handle.
                    if !matches!(self.service_c_type(service, name).as_deref(), Some("uint64_t" | "int32_t")) {
                        return self.not_yet(fx, e, "same() of these values");
                    }
                    sides.push(if opt { (format!("{x}.some"), format!("{x}.value")) } else { ("1".to_string(), x.clone()) });
                }
                let ((sa, va), (sb, vb)) = (&sides[0], &sides[1]);
                Ok(fx.temp("uint8_t", &format!("(uint8_t)(({sa}) == ({sb}) && (!({sa}) || {va} == {vb}))")))
            }
            // FNV-1a 32 over the text's UTF-8 bytes (docs/modding/language/language.md, section 16a: a hash that is the same on every platform and run).
            "hash32" => Ok(fx.temp("uint32_t", &format!("lumart_hash32({})", v(0)))),
            // Inside an export, the mod that called it (lumart_caller, kept through chained libraries); else this mod.
            "caller" => Ok(fx.temp("uint64_t", "(uint64_t)(uintptr_t)(lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self)")),
            // An integer's low bits as type T: C's conversion to the unsigned width, then to T.
            "low_bits" => match t {
                Ty::Int(sc) => Ok(fx.temp(scalar_c(*sc), &format!("({})({})(uint64_t){}", scalar_c(*sc), scalar_c(unsigned_of(*sc)), v(0)))),
                _ => self.not_yet(fx, e, "low_bits of this type"),
            },
            other => self.not_yet(fx, e, &format!("{other}()")),
        }
    }

    // ------------------------------------------------------------ containers (docs/modding/language/language.md, sections 3, 6.3; docs/modding/language/compiled-form.md, section 5)

    /// The descriptor of element type `t` (lumart.h `lumart_desc`), written into lm.h once per C type: `&lm_desc_<n>`.
    /// Declared before its functions, which may need it (a struct holding a list of itself).
    pub(super) fn desc(&mut self, t: &Ty) -> Option<String> {
        let ct = self.c_type(t)?;
        if let Some(n) = self.descs.get(&ct) {
            return Some(format!("&{n}"));
        }
        let n = format!("lm_desc_{}", self.descs.len());
        self.descs.insert(ct.clone(), n.clone());
        self.typedefs.push(format!("static const lumart_desc {n};"));
        let a = format!("(*({ct}*)a)");
        let b = format!("(*({ct}*)b)");
        let eq = self.eq_c(&a, &b, t);
        let cmp = self.cmp_c(&a, &b, t);
        let mut body = String::new();
        let (ret, rel) = if counted(&ct) {
            let _ = writeln!(body, "static void {n}_ret(void* p) {{\n    lm_ret_{ct}(({ct}*)p);\n}}");
            let _ = writeln!(body, "static void {n}_rel(void* p) {{\n    lm_rel_{ct}(({ct}*)p);\n}}");
            (format!("{n}_ret"), format!("{n}_rel"))
        } else {
            ("0".into(), "0".into())
        };
        let eqf = match &eq {
            Some(x) => {
                let _ = writeln!(body, "static uint8_t {n}_eq(const void* a, const void* b) {{\n    return (uint8_t)({x});\n}}");
                format!("{n}_eq")
            }
            None => "0".into(),
        };
        let cmpf = match &cmp {
            Some(x) => {
                let _ = writeln!(body, "static int32_t {n}_cmp(const void* a, const void* b) {{\n    return {x};\n}}");
                format!("{n}_cmp")
            }
            None => "0".into(),
        };
        let _ = write!(body, "static const lumart_desc {n} = {{sizeof({ct}), {ret}, {rel}, {eqf}, {cmpf}}};");
        self.typedefs.push(body);
        Some(format!("&{n}"))
    }

    /// The order of two places of type `t` as a C int (-1, 0, 1): numbers, `bool`, text by bytes, game enums by value,
    /// enums without data by case. None for a type without an order.
    fn cmp_c(&mut self, l: &str, r: &str, t: &Ty) -> Option<String> {
        Some(match t {
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Game(GameTy::Enum(_)) => format!("({l} < {r} ? -1 : {l} > {r} ? 1 : 0)"),
            Ty::Text | Ty::Json => format!("lumart_text_cmp({l}, {r})"),
            Ty::Enum(item) if self.enum_plain(*item) => format!("({l}.tag < {r}.tag ? -1 : {l}.tag > {r}.tag ? 1 : 0)"),
            _ => return None,
        })
    }

    /// Element `i` of list `l` (both C) as a place of C type `ct`.
    fn list_elem(ct: &str, l: &str, i: &str) -> String {
        format!("(*({ct}*)lumart_list_at({l}, {i}))")
    }

    /// `[a, b, ...]`: a new list, its room made once.
    fn list_value(&mut self, fx: &mut Fx, e: &Expr, items: &[Expr], t: &Ty) -> Ex {
        let m = fx.module;
        let Ty::List(x) = t else { return self.not_yet(fx, e, "a list literal of this type") };
        if items.is_empty() {
            return Ok("((lumart_list)0)".into());
        }
        let x = (**x).clone();
        let ec = self.c_type(&x).ok_or_else(lost)?;
        let d = self.desc(&x).ok_or_else(lost)?;
        let (file, line) = (c_string(&fx.file), e.at.line);
        let r = fx.fresh("lumart_list");
        fx.emit(format!("lm_rel_lumart_list(&{r});"));
        fx.fail_if(&format!("lumart_list_own(&{r}, {d}, {}u, {file}, {line})", items.len()));
        for it in items {
            let v = self.expr(fx, it)?;
            let from = self.ty(m, it);
            let v = self.coerce(fx, &v, &from, &x, it.at)?;
            let tmp = fx.temp(&ec, &v);
            fx.fail_if(&format!("lumart_list_push(&{r}, {d}, &{tmp}, {file}, {line})"));
        }
        Ok(r)
    }

    /// `{k: v, ...}` (a map) or `{a, b}` (a set): a new block; a key written twice keeps its last value.
    fn braces_value(&mut self, fx: &mut Fx, e: &Expr, entries: &[(Expr, Option<Expr>)], t: &Ty) -> Ex {
        let m = fx.module;
        let (k, v) = match t {
            Ty::Map(k, v) => ((**k).clone(), Some((**v).clone())),
            Ty::Set(k) => ((**k).clone(), None),
            _ => return self.not_yet(fx, e, "braces of this type"),
        };
        if entries.is_empty() {
            return Ok("((lumart_map)0)".into());
        }
        let kc = self.c_type(&k).ok_or_else(lost)?;
        let kd = self.desc(&k).ok_or_else(lost)?;
        let vd = match &v {
            Some(v) => self.desc(v).ok_or_else(lost)?,
            None => "0".into(),
        };
        let (file, line) = (c_string(&fx.file), e.at.line);
        let r = fx.fresh("lumart_map");
        fx.emit(format!("lm_rel_lumart_map(&{r});"));
        fx.fail_if(&format!("lumart_map_own(&{r}, {kd}, {vd}, {}u, {file}, {line})", entries.len()));
        let added = fx.fresh("uint8_t");
        for (key, value) in entries {
            let kv = self.expr(fx, key)?;
            let kf = self.ty(m, key);
            let kv = self.coerce(fx, &kv, &kf, &k, key.at)?;
            let kt = fx.temp(&kc, &kv);
            let vp = match (&v, value) {
                (Some(vt), Some(value)) => {
                    let vc = self.c_type(vt).ok_or_else(lost)?;
                    let x = self.expr(fx, value)?;
                    let vf = self.ty(m, value);
                    let x = self.coerce(fx, &x, &vf, vt, value.at)?;
                    format!("&{}", fx.temp(&vc, &x))
                }
                _ => "0".into(),
            };
            fx.fail_if(&format!("lumart_map_put(&{r}, {kd}, {vd}, &{kt}, {vp}, &{added}, {file}, {line})"));
        }
        Ok(r)
    }

    /// `xs[i]` and `m[k]` read: an index outside the list is S0902, a missing key S0907 (the key shown).
    fn index_value(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, index: &Expr) -> Ex {
        let m = fx.module;
        let bt = match self.ty(m, base) {
            // bytes is a list block of u8 (lumart_bytes): read as one.
            Ty::Bytes => Ty::List(Box::new(Ty::Int(Scalar::U8))),
            t => t,
        };
        let (file, line) = (c_string(&fx.file), e.at.line);
        match &bt {
            Ty::List(x) => {
                let ec = self.c_type(x).ok_or_else(lost)?;
                let b = self.expr(fx, base)?;
                let b = fx.temp("lumart_list", &b);
                let i = self.expr(fx, index)?;
                let i = fx.temp("int64_t", &format!("(int64_t)({i})"));
                fx.fail_if(&format!("(uint64_t){i} >= lumart_list_n({b}) && lumart_index_error({i}, lumart_list_n({b}), {file}, {line})"));
                // The element copied out and the list's count given back at once, so a write after this read finds
                // one owner and copies nothing.
                let r = fx.temp(&ec, &Self::list_elem(&ec, &b, &format!("(uint64_t){i}")));
                fx.emit(format!("lm_rel_lumart_list(&{b});"));
                Ok(r)
            }
            Ty::Map(k, v) => {
                let (k, v) = ((**k).clone(), (**v).clone());
                let b = self.expr(fx, base)?;
                let b = fx.temp("lumart_map", &b);
                let at = self.map_find_or_fail(fx, e, &b, &k, index)?;
                let vc = self.c_type(&v).ok_or_else(lost)?;
                let _ = (file, line);
                let r = fx.temp(&vc, &format!("(*({vc}*)lumart_map_val({b}, {at}))"));
                fx.emit(format!("lm_rel_lumart_map(&{b});"));
                Ok(r)
            }
            other => {
                let what = format!("[ ] on {}", self.ty_text(other));
                self.not_yet(fx, e, &what)
            }
        }
    }

    /// Finds key `key` (of type `k`) in map `b` (a C place): its index, or S0907 with the key shown.
    pub(super) fn map_find_or_fail(&mut self, fx: &mut Fx, e: &Expr, b: &str, k: &Ty, key: &Expr) -> Ex {
        let kt = self.map_key(fx, k, key)?;
        self.map_find_at(fx, e, b, k, &kt)
    }

    /// A map's key read into a temporary of the key's type.
    pub(super) fn map_key(&mut self, fx: &mut Fx, k: &Ty, key: &Expr) -> Ex {
        let m = fx.module;
        let kc = self.c_type(k).ok_or_else(lost)?;
        let kv = self.expr(fx, key)?;
        let kf = self.ty(m, key);
        let kv = self.coerce(fx, &kv, &kf, k, key.at)?;
        Ok(fx.temp(&kc, &kv))
    }

    /// Where key `kt` sits in map `b`; a key not there is S0907 (the key shown).
    pub(super) fn map_find_at(&mut self, fx: &mut Fx, e: &Expr, b: &str, k: &Ty, kt: &str) -> Ex {
        let kd = self.desc(k).ok_or_else(lost)?;
        let (file, line) = (c_string(&fx.file), e.at.line);
        let kt = kt.to_string();
        let at = fx.fresh("uint64_t");
        fx.emit(format!("if (!lumart_map_find({b}, {kd}, &{kt}, &{at})) {{"));
        fx.depth += 1;
        let sb = fx.builder();
        self.value_text(fx, &sb, &kt, k, e, false)?;
        fx.fail_if(&format!("lumart_sb_done(&{sb}, {file}, {line})"));
        fx.fail_if(&format!("lumart_key_error({sb}.p, {file}, {line})"));
        fx.depth -= 1;
        fx.emit("}".into());
        Ok(at)
    }

    /// A method of a list, map or set (language.md). The receiver is read first (its place, for one that writes
    /// it), then the arguments in order.
    fn container_method(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, bt: &Ty, name: &str, args: &[ast::Arg], t: &Ty) -> Ex {
        let writes = matches!((bt, name), (Ty::List(_), "push" | "pop" | "insert" | "remove" | "sort" | "sort_by") | (Ty::Map(..), "remove") | (Ty::Set(_), "add" | "remove"));
        let ct = self.c_type(bt).ok_or_else(lost)?;
        let mark = fx.rechecks.len();
        let b = if writes {
            self.place(fx, base)?
        } else {
            let v = self.expr(fx, base)?;
            fx.temp(&ct, &v)
        };
        let r = self.container_method_on(fx, e, &b, bt, name, args, t, mark);
        fx.rechecks.truncate(mark);
        let r = r?;
        if writes {
            return Ok(r);
        }
        // A read: its result in a temporary of its own, then the receiver's count given back at once (as
        // index_value), so a write after it finds one owner.
        let r = match self.c_type(t) {
            Some(rc) if !r.is_empty() && !r.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') => fx.temp(&rc, &r),
            _ => r,
        };
        fx.emit(format!("lm_rel_{ct}(&{b});"));
        Ok(r)
    }

    /// A method of `bytes` (docs/modding/language/language.md): the receiver read first (its place, for push and write_*, its checks
    /// made again after the arguments), then the arguments in order; a read gives the receiver's count back at once.
    fn bytes_method(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, name: &str, args: &[ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let (file, line) = (c_string(&fx.file), e.at.line);
        let src = c_string(&expr_text(e));
        let int = Ty::Int(Scalar::I32);
        let width = |n: &str| -> Option<(u32, Ty)> {
            Some(match n.rsplit_once('_')?.1 {
                "u16" => (2, Ty::Int(Scalar::U16)),
                "u32" => (4, Ty::Int(Scalar::U32)),
                "u64" => (8, Ty::Int(Scalar::U64)),
                "f32" => (4, Ty::Float(Scalar::F32)),
                "f64" => (8, Ty::Float(Scalar::F64)),
                _ => return None,
            })
        };
        let wants: Vec<Ty> = match name {
            "len" | "text" => Vec::new(),
            "slice" => vec![int.clone(), int.clone()],
            "push" => vec![Ty::Int(Scalar::U8)],
            n if n.starts_with("read_") && width(n).is_some() => vec![int.clone(), Ty::Bool],
            n if n.starts_with("write_") => match width(n) {
                Some((_, vt)) => vec![int.clone(), vt, Ty::Bool],
                None => return self.not_yet(fx, e, &format!("bytes.{name}()")),
            },
            _ => return self.not_yet(fx, e, &format!("bytes.{name}()")),
        };
        let writes = name == "push" || name.starts_with("write_");
        let mark = fx.rechecks.len();
        let b = if writes {
            self.place(fx, base)?
        } else {
            let v = self.expr(fx, base)?;
            fx.temp("lumart_bytes", &v)
        };
        let mut a = Vec::new();
        for (arg, want) in args.iter().zip(wants.iter()) {
            let v = self.expr(fx, &arg.value)?;
            let from = self.ty(m, &arg.value);
            let v = self.coerce(fx, &v, &from, want, arg.value.at)?;
            let ct = self.c_type(want).ok_or_else(lost)?;
            a.push(fx.temp(&ct, &v));
        }
        fx.recheck_since(mark);
        fx.rechecks.truncate(mark);
        let n = format!("lumart_list_n({b})");
        let r = match name {
            "len" => {
                fx.fail_if(&format!("{n} > 2147483647u && lumart_overflow({file}, {line}, \"i32\", {src})"));
                fx.temp("int32_t", &format!("((int32_t){n})"))
            }
            "slice" => {
                let r = fx.fresh("lumart_bytes");
                fx.fail_if(&format!("lumart_bytes_slice(&{r}, {b}, (int64_t){}, (int64_t){}, {file}, {line})", a[0], a[1]));
                r
            }
            "text" => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let r = fx.fresh(&ct);
                fx.fail_if(&format!("lumart_bytes_text(&{r}.value, &{r}.some, {b}, {file}, {line})"));
                r
            }
            "push" => {
                fx.fail_if(&format!("lumart_list_push(&{b}, &lumart_u8, &{}, {file}, {line})", a[0]));
                return Ok(String::new());
            }
            n if n.starts_with("read_") => {
                let (k, vt) = width(n).ok_or_else(lost)?;
                let bits = fx.fresh("uint64_t");
                fx.fail_if(&format!("lumart_bytes_read({b}, (int64_t){}, {k}u, {}, &{bits}, {file}, {line})", a[0], a[1]));
                let vc = self.c_type(&vt).ok_or_else(lost)?;
                let v = match vt {
                    Ty::Float(Scalar::F32) => format!("lumart_f32_of({bits})"),
                    Ty::Float(_) => format!("lumart_f64_of({bits})"),
                    _ => format!("({vc}){bits}"),
                };
                let _ = t;
                fx.temp(&vc, &v)
            }
            n => {
                let (k, vt) = width(n).ok_or_else(lost)?;
                let bits = match vt {
                    Ty::Float(Scalar::F32) => format!("lumart_bits_f32({})", a[1]),
                    Ty::Float(_) => format!("lumart_bits_f64({})", a[1]),
                    _ => format!("(uint64_t){}", a[1]),
                };
                fx.fail_if(&format!("lumart_bytes_write(&{b}, (int64_t){}, {k}u, {}, {bits}, {file}, {line})", a[0], a[2]));
                return Ok(String::new());
            }
        };
        fx.emit(format!("lm_rel_lumart_bytes(&{b});"));
        Ok(r)
    }

    /// [`Gen::container_method`] on receiver `b` (its place for a method that writes it, else a temporary).
    #[allow(clippy::too_many_arguments)]
    fn container_method_on(&mut self, fx: &mut Fx, e: &Expr, b: &str, bt: &Ty, name: &str, args: &[ast::Arg], t: &Ty, mark: usize) -> Ex {
        let m = fx.module;
        let b = b.to_string();
        let (file, line) = (c_string(&fx.file), e.at.line);
        let src = c_string(&expr_text(e));
        // The arguments, each made to the type its parameter takes.
        let mut a = Vec::new();
        let wants: Vec<Ty> = match (bt, name) {
            (Ty::List(x), "push" | "contains") => vec![(**x).clone()],
            (Ty::List(x), "insert") => vec![Ty::Int(Scalar::I32), (**x).clone()],
            (Ty::List(_), "remove" | "get") => vec![Ty::Int(Scalar::I32)],
            (Ty::Map(k, _), "get" | "contains" | "remove") => vec![(**k).clone()],
            (Ty::Set(x), "add" | "remove" | "contains") => vec![(**x).clone()],
            // A closure: written in place by the method (no value made for it).
            (Ty::List(_), "map" | "filter" | "any" | "all" | "sum" | "sort_by") => Vec::new(),
            _ if args.is_empty() => Vec::new(),
            _ => return self.not_yet(fx, e, &format!("{}.{name}()", self.ty_text(bt))),
        };
        for (arg, want) in args.iter().zip(wants.iter()) {
            let v = self.expr(fx, &arg.value)?;
            let from = self.ty(m, &arg.value);
            let v = self.coerce(fx, &v, &from, want, arg.value.at)?;
            let ct = self.c_type(want).ok_or_else(lost)?;
            a.push(fx.temp(&ct, &v));
        }
        // A receiver inside a container: its checks again, the arguments having been read (kept for a closure, which
        // may change the container around the receiver while it runs).
        let nested: Vec<String> = fx.rechecks[mark..].concat();
        fx.recheck_since(mark);
        let len_i32 = |fx: &mut Fx, n: String| -> String {
            fx.fail_if(&format!("{n} > 2147483647u && lumart_overflow({file}, {line}, \"i32\", {src})"));
            format!("((int32_t){n})")
        };
        match bt {
            Ty::List(x) => {
                let x = (**x).clone();
                let ec = self.c_type(&x).ok_or_else(lost)?;
                let d = self.desc(&x).ok_or_else(lost)?;
                let n = format!("lumart_list_n({b})");
                match name {
                    "len" => Ok(len_i32(fx, n)),
                    "push" => {
                        fx.fail_if(&format!("lumart_list_push(&{b}, {d}, &{}, {file}, {line})", a[0]));
                        Ok(String::new())
                    }
                    "insert" => {
                        let i = &a[0];
                        fx.fail_if(&format!("((int64_t){i} < 0 || (uint64_t){i} > {n}) && lumart_index_error({i}, {n}, {file}, {line})"));
                        fx.fail_if(&format!("lumart_list_insert(&{b}, {d}, (uint64_t){i}, &{}, {file}, {line})", a[1]));
                        Ok(String::new())
                    }
                    "remove" => {
                        let i = &a[0];
                        fx.fail_if(&format!("((int64_t){i} < 0 || (uint64_t){i} >= {n}) && lumart_index_error({i}, {n}, {file}, {line})"));
                        let r = fx.fresh(&ec);
                        fx.set(&r, &Self::list_elem(&ec, &b, &format!("(uint64_t){i}")), &ec);
                        fx.fail_if(&format!("lumart_list_remove(&{b}, {d}, (uint64_t){i}, {file}, {line})"));
                        Ok(r)
                    }
                    "pop" | "get" | "first" | "last" => {
                        let ct = self.c_type(t).ok_or_else(lost)?;
                        let i = match name {
                            "get" => format!("(uint64_t){}", a[0]),
                            "first" => "0u".into(),
                            _ => format!("({n} - 1u)"),
                        };
                        let has = match name {
                            "get" => format!("((int64_t){} >= 0 && (uint64_t){} < {n})", a[0], a[0]),
                            _ => format!("({n} != 0)"),
                        };
                        let r = fx.fresh(&ct);
                        fx.emit(format!("if {has} {{"));
                        fx.depth += 1;
                        fx.set(&r, &some_c(t, &ct, &Self::list_elem(&ec, &b, &i)), &ct);
                        if name == "pop" {
                            fx.fail_if(&format!("lumart_list_remove(&{b}, {d}, {i}, {file}, {line})"));
                        }
                        fx.depth -= 1;
                        fx.emit("} else {".into());
                        fx.depth += 1;
                        fx.set(&r, &format!("({ct}){{0}}"), &ct);
                        fx.depth -= 1;
                        fx.emit("}".into());
                        Ok(r)
                    }
                    "contains" => Ok(fx.temp("uint8_t", &format!("(uint8_t)(lumart_list_find({b}, {d}, &{}) >= 0)", a[0]))),
                    "sort" => {
                        fx.fail_if(&format!("lumart_list_sort(&{b}, {d}, lumart_less_by_type, (void*){d}, {file}, {line})"));
                        Ok(String::new())
                    }
                    "indexed" => {
                        let tt = Ty::Tuple(vec![Ty::Int(Scalar::I32), x.clone()]);
                        let tc = self.c_type(&tt).ok_or_else(lost)?;
                        let td = self.desc(&tt).ok_or_else(lost)?;
                        let r = fx.fresh("lumart_list");
                        let i = fx.fresh("uint64_t");
                        let item = fx.fresh(&tc);
                        fx.emit(format!("lm_rel_lumart_list(&{r});"));
                        let _ = len_i32(fx, n.clone());
                        fx.fail_if(&format!("lumart_list_own(&{r}, {td}, {n}, {file}, {line})"));
                        fx.emit(format!("for ({i} = 0; {i} < {n}; {i}++) {{"));
                        fx.depth += 1;
                        fx.set(&item, &format!("({tc}){{(int32_t){i}, {}}}", Self::list_elem(&ec, &b, &i)), &tc);
                        fx.fail_if(&format!("lumart_list_push(&{r}, {td}, &{item}, {file}, {line})"));
                        fx.depth -= 1;
                        fx.emit("}".into());
                        Ok(r)
                    }
                    "map" | "filter" | "any" | "all" | "sum" | "sort_by" => {
                        let Some(arg) = args.first() else { return Err(lost()) };
                        let ExprKind::Closure { params, body, .. } = &strip(&arg.value).kind else {
                            return self.not_yet(fx, e, &format!("list.{name}() given a function value (a closure written in the call is compiled)"));
                        };
                        let i = fx.fresh("uint64_t");
                        let item = Self::list_elem(&ec, &b, &i);
                        match name {
                            "map" => {
                                let Ty::List(u) = t else { return Err(lost()) };
                                let u = (**u).clone();
                                let ud = self.desc(&u).ok_or_else(lost)?;
                                let r = fx.fresh("lumart_list");
                                fx.emit(format!("lm_rel_lumart_list(&{r});"));
                                fx.fail_if(&format!("lumart_list_own(&{r}, {ud}, {n}, {file}, {line})"));
                                fx.emit(format!("for ({i} = 0; {i} < {n}; {i}++) {{"));
                                fx.depth += 1;
                                let v = self.inline_closure(fx, params, body, &[(item, x.clone())], &u)?;
                                fx.fail_if(&format!("lumart_list_push(&{r}, {ud}, &{v}, {file}, {line})"));
                                fx.depth -= 1;
                                fx.emit("}".into());
                                Ok(r)
                            }
                            "filter" => {
                                let r = fx.fresh("lumart_list");
                                fx.emit(format!("lm_rel_lumart_list(&{r});"));
                                fx.emit(format!("for ({i} = 0; {i} < {n}; {i}++) {{"));
                                fx.depth += 1;
                                let ok = self.inline_closure(fx, params, body, &[(item, x.clone())], &Ty::Bool)?;
                                fx.fail_if(&format!("{ok} && lumart_list_push(&{r}, {d}, lumart_list_at({b}, {i}), {file}, {line})"));
                                fx.depth -= 1;
                                fx.emit("}".into());
                                Ok(r)
                            }
                            "any" | "all" => {
                                // `any` stops at the first true, `all` at the first false.
                                let r = fx.fresh("uint8_t");
                                let (start, go) = if name == "any" { ("0", format!("!{r}")) } else { ("1", r.clone()) };
                                fx.emit(format!("{r} = {start};"));
                                fx.emit(format!("for ({i} = 0; {i} < {n} && {go}; {i}++) {{"));
                                fx.depth += 1;
                                let ok = self.inline_closure(fx, params, body, &[(item, x.clone())], &Ty::Bool)?;
                                fx.emit(format!("{r} = {ok};"));
                                fx.depth -= 1;
                                fx.emit("}".into());
                                Ok(r)
                            }
                            "sum" => {
                                let nc = self.c_type(t).ok_or_else(lost)?;
                                let r = fx.fresh(&nc);
                                fx.emit(format!("{r} = 0;"));
                                fx.emit(format!("for ({i} = 0; {i} < {n}; {i}++) {{"));
                                fx.depth += 1;
                                let v = self.inline_closure(fx, params, body, &[(item, x.clone())], t)?;
                                let acc = self.binary_values(fx, BinOp::Add, &r, t, &v, t, t, &expr_text(e), e.at)?;
                                fx.set(&r, &acc, &nc);
                                fx.depth -= 1;
                                fx.emit("}".into());
                                Ok(r)
                            }
                            _ => {
                                // sort_by: a list inside a container is sorted as a copy and stored back (its checks
                                // again), since the closure may change the container holding it.
                                let target = if nested.is_empty() { b.clone() } else { fx.temp("lumart_list", &b) };
                                let s = fx.fresh("lumart_sorter");
                                fx.fail_if(&format!("lumart_sorter_begin(&{s}, &{target}, {d}, 0u, {file}, {line})"));
                                let y_first = fx.fresh("uint8_t");
                                fx.emit(format!("while (lumart_sorter_next(&{s}, {y_first})) {{"));
                                fx.depth += 1;
                                let (y, xv) = (format!("(*({ec}*){s}.y)"), format!("(*({ec}*){s}.x)"));
                                let before = self.inline_closure(fx, params, body, &[(y, x.clone()), (xv, x.clone())], &Ty::Bool)?;
                                fx.emit(format!("{y_first} = {before};"));
                                fx.depth -= 1;
                                fx.emit("}".into());
                                if !nested.is_empty() {
                                    fx.lines.extend(nested);
                                    fx.set(&b, &target, "lumart_list");
                                }
                                Ok(String::new())
                            }
                        }
                    }
                    _ => self.not_yet(fx, e, &format!("list.{name}()")),
                }
            }
            Ty::Map(k, v) | Ty::Set(k @ v) => {
                let is_set = matches!(bt, Ty::Set(_));
                let (k, v) = ((**k).clone(), (**v).clone());
                let kc = self.c_type(&k).ok_or_else(lost)?;
                let kd = self.desc(&k).ok_or_else(lost)?;
                let (vc, vd) = if is_set { (String::new(), "0".to_string()) } else { (self.c_type(&v).ok_or_else(lost)?, self.desc(&v).ok_or_else(lost)?) };
                let n = format!("lumart_map_n({b})");
                match name {
                    "len" => Ok(len_i32(fx, n)),
                    "contains" => {
                        let at = fx.fresh("uint64_t");
                        Ok(fx.temp("uint8_t", &format!("lumart_map_find({b}, {kd}, &{}, &{at})", a[0])))
                    }
                    "add" => {
                        let added = fx.fresh("uint8_t");
                        fx.fail_if(&format!("lumart_map_put(&{b}, {kd}, 0, &{}, 0, &{added}, {file}, {line})", a[0]));
                        Ok(added)
                    }
                    "get" | "remove" => {
                        let at = fx.fresh("uint64_t");
                        let found = fx.temp("uint8_t", &format!("lumart_map_find({b}, {kd}, &{}, &{at})", a[0]));
                        let (r, ct) = if is_set { (fx.fresh("uint8_t"), "uint8_t".to_string()) } else {
                            let ct = self.c_type(t).ok_or_else(lost)?;
                            (fx.fresh(&ct), ct)
                        };
                        fx.emit(format!("if ({found}) {{"));
                        fx.depth += 1;
                        if is_set {
                            fx.emit(format!("{r} = 1;"));
                        } else {
                            fx.set(&r, &some_c(t, &ct, &format!("(*({vc}*)lumart_map_val({b}, {at}))")), &ct);
                        }
                        if name == "remove" {
                            fx.fail_if(&format!("lumart_map_remove(&{b}, {kd}, {vd}, {at}, {file}, {line})"));
                        }
                        fx.depth -= 1;
                        fx.emit("} else {".into());
                        fx.depth += 1;
                        fx.set(&r, &if is_set { "0".to_string() } else { format!("({ct}){{0}}") }, &ct);
                        fx.depth -= 1;
                        fx.emit("}".into());
                        Ok(r)
                    }
                    "keys" | "values" => {
                        let (ec, ed, at) = if name == "keys" || is_set { (kc.clone(), kd.clone(), "lumart_map_key") } else { (vc.clone(), vd.clone(), "lumart_map_val") };
                        let r = fx.fresh("lumart_list");
                        let i = fx.fresh("uint64_t");
                        fx.emit(format!("lm_rel_lumart_list(&{r});"));
                        fx.fail_if(&format!("lumart_list_own(&{r}, {ed}, {n}, {file}, {line})"));
                        fx.emit(format!("for ({i} = 0; {i} < {n}; {i}++) {{"));
                        fx.depth += 1;
                        fx.fail_if(&format!("lumart_list_push(&{r}, {ed}, ({ec}*){at}({b}, {i}), {file}, {line})"));
                        fx.depth -= 1;
                        fx.emit("}".into());
                        Ok(r)
                    }
                    _ => self.not_yet(fx, e, &format!("{}.{name}()", self.ty_text(bt))),
                }
            }
            _ => Err(lost()),
        }
    }

    /// A list, set or map as text: `[a, b]`, `{k: v}`; as json an array, or an object whose keys are strings.
    fn container_text(&mut self, fx: &mut Fx, b: &str, v: &str, t: &Ty, e: &Expr, json: bool) -> Result<(), NotWritten> {
        let ct = self.c_type(t).ok_or_else(lost)?;
        let tmp = fx.temp(&ct, v);
        let i = fx.fresh("uint64_t");
        let (open, close, n) = match t {
            Ty::List(_) => ("[", "]", format!("lumart_list_n({tmp})")),
            Ty::Set(_) => ("[", "]", format!("lumart_map_n({tmp})")),
            _ => ("{", "}", format!("lumart_map_n({tmp})")),
        };
        fx.emit(format!("lumart_sb_str(&{b}, \"{open}\");"));
        fx.emit(format!("for ({i} = 0; {i} < {n}; {i}++) {{"));
        fx.depth += 1;
        fx.emit(format!("if ({i} != 0) lumart_sb_str(&{b}, \", \");"));
        match t {
            Ty::List(x) => {
                let ec = self.c_type(x).ok_or_else(lost)?;
                self.value_text(fx, b, &Self::list_elem(&ec, &tmp, &i), &x.clone(), e, json)?;
            }
            Ty::Set(x) => {
                let ec = self.c_type(x).ok_or_else(lost)?;
                self.value_text(fx, b, &format!("(*({ec}*)lumart_map_key({tmp}, {i}))"), &x.clone(), e, json)?;
            }
            Ty::Map(k, v) => {
                let (k, v) = ((**k).clone(), (**v).clone());
                let kc = self.c_type(&k).ok_or_else(lost)?;
                let vc = self.c_type(&v).ok_or_else(lost)?;
                let key = format!("(*({kc}*)lumart_map_key({tmp}, {i}))");
                // A json object's key is a string: text as itself, other keys as they show, quoted.
                // A game enum's key as json is its name, quoted by value_text itself.
                let quote = json && !matches!(k, Ty::Text | Ty::Game(GameTy::Enum(_)));
                if quote {
                    fx.emit(format!("lumart_sb_str(&{b}, \"\\\"\");"));
                }
                self.value_text(fx, b, &key, &k, e, json && !quote)?;
                if quote {
                    fx.emit(format!("lumart_sb_str(&{b}, \"\\\"\");"));
                }
                fx.emit(format!("lumart_sb_str(&{b}, \": \");"));
                self.value_text(fx, b, &format!("(*({vc}*)lumart_map_val({tmp}, {i}))"), &v, e, json)?;
            }
            _ => return Err(lost()),
        }
        fx.depth -= 1;
        fx.emit("}".into());
        fx.emit(format!("lumart_sb_str(&{b}, \"{close}\");"));
        fx.emit(format!("lm_rel_{ct}(&{tmp});"));
        Ok(())
    }

    // ------------------------------------------------------------ text and json

    /// A string as a value: without interpolations a literal block of the module (counted LUMART_FOREVER: no count
    /// is ever taken or given back); with them a new text from a builder.
    pub(super) fn text_value(&mut self, fx: &mut Fx, e: &Expr, t: &Ty) -> Ex {
        let ExprKind::Str(lit) = &e.kind else { return Err(lost()) };
        // `b"..."`: its bytes (any byte by \xHH), a new block made from a static one.
        if *t == Ty::Bytes && lit.prefix == StrPrefix::Bytes && lit.parts.iter().all(|p| matches!(p, StrPart::Text(_))) {
            let mut bytes = Vec::new();
            for p in &lit.parts {
                if let StrPart::Text(raw) = p {
                    bytes.extend(unescape(raw));
                }
            }
            let s = self.text_literal(fx.module, &bytes);
            let r = fx.fresh("lumart_bytes");
            fx.fail_if(&format!("lumart_bytes_of_text(&{r}, {s}, {}, {})", c_string(&fx.file), e.at.line));
            return Ok(r);
        }
        if *t != Ty::Text {
            let what = format!("a string as {}", self.ty_text(t));
            return self.not_yet(fx, e, &what);
        }
        if lit.prefix != StrPrefix::None && lit.prefix != StrPrefix::Raw {
            return self.not_yet(fx, e, "bytes as text");
        }
        if lit.parts.iter().all(|p| matches!(p, StrPart::Text(_))) {
            let mut bytes = Vec::new();
            for p in &lit.parts {
                if let StrPart::Text(raw) = p {
                    bytes.extend(if lit.prefix == StrPrefix::Raw { raw.as_bytes().to_vec() } else { unescape(raw) });
                }
            }
            return Ok(self.text_literal(fx.module, &bytes));
        }
        // Made in the scratch and copied into its own block once whole: one allocation, of the text's size.
        let (b, mark) = fx.scratch_builder();
        self.text_into(fx, &b, e)?;
        let r = fx.fresh("lumart_text");
        fx.fail_if(&format!("lumart_text_from_sb(&{r}, &{b}, {}, {})", c_string(&fx.file), e.at.line));
        fx.emit(format!("lumart_scratch_release({mark});"));
        Ok(r)
    }

    /// `text "Key"(name: value, ..)`: the mod's line `Key` in the player's language. A mod's lines are cooked
    /// into the game's messages when it is imported (nebula-import cook/message_merge.rs), so the row is the game's
    /// message of that id as text with its tags in bracket words, the key itself when the game has none. The runtime
    /// reads it (lumart_text_message) and never calls the game before its messages exist: made earlier, the text is a
    /// row read on its first use after they do. Each named argument's text then takes the place of every `{name}`.
    fn text_row(&mut self, fx: &mut Fx, e: &Expr, path: &ast::StrLit, args: &[ast::Arg]) -> Ex {
        let mut key = Vec::new();
        for part in &path.parts {
            match part {
                StrPart::Text(raw) => key.extend(if path.prefix == StrPrefix::Raw { raw.as_bytes().to_vec() } else { unescape(raw) }),
                _ => return self.not_yet(fx, e, "a text key made at run time"),
            }
        }
        let file = c_string(&fx.file);
        let line = e.at.line;
        let k = fx.temp("const char*", &c_bytes(&key));
        let mut text = fx.fresh("lumart_text");
        fx.fail_if(&format!("lumart_text_message(&{text}, {k}, {}, {file}, {line})", key.len()));
        for a in args {
            let Some(name) = &a.name else { return self.not_yet(fx, e, "a text argument without its placeholder's name") };
            let (b, mark) = fx.scratch_builder();
            self.value_text_into(fx, &b, &a.value, false)?;
            fx.fail_if(&format!("lumart_sb_done(&{b}, {file}, {line})"));
            let filled = fx.fresh("lumart_text");
            fx.fail_if(&format!(
                "lumart_text_fill(&{filled}, {text}, {}, {}, {b}.p, {b}.len, {file}, {line})",
                c_bytes(name.name.as_bytes()),
                name.name.len()
            ));
            fx.emit(format!("lumart_scratch_release({mark});"));
            text = filled;
        }
        Ok(text)
    }

    /// A builtin method (language.md) on the value of `base`: the receiver read first, then the arguments in order.
    fn call_method(&mut self, fx: &mut Fx, e: &Expr, base: &Expr, name: &str, args: &[ast::Arg], t: &Ty) -> Ex {
        let m = fx.module;
        let bt = match (self.ty(m, base), &self.pre_read) {
            (Ty::Opt(x), Some((id, _))) if *id == base.id => *x,
            (bt, _) => bt,
        };
        let file = c_string(&fx.file);
        let line = e.at.line;
        let src = c_string(&expr_text(e));
        if matches!(bt, Ty::List(_) | Ty::Map(..) | Ty::Set(_)) {
            return self.container_method(fx, e, base, &bt, name, args, t);
        }
        match &bt {
            Ty::Bytes => self.bytes_method(fx, e, base, name, args, t),
            Ty::Json => self.json_method(fx, e, base, name, args, t),
            // `h.get()`: the object as the handle's class, none (null) once it is gone.
            // `t.stop()` (11.1).
            Ty::Task if name == "stop" => {
                let h = self.expr(fx, base)?;
                let h = fx.temp("lumart_task", &h);
                fx.emit(format!("lumart_task_stop({h});"));
                Ok(String::new())
            }
            // `p.stop()` (11.1a): its task stopped by its maker's code; a `wait p` then ends without a value.
            Ty::Pending(_) if name == "stop" => {
                let p = self.expr(fx, base)?;
                let p = fx.temp("lumart_pending", &p);
                fx.emit(format!("if ({p}.ops != 0) {p}.ops->stop({p}.t);"));
                Ok(String::new())
            }
            Ty::Ref(of) if name == "get" => {
                // A mod's class narrows by its own number (define_class's; as_class takes it, luma.h 246).
                let cls = match &**of {
                    Ty::Game(GameTy::View { record, .. }) => {
                        let record = record.clone();
                        self.class_slot(&record)
                    }
                    Ty::Class(item) => {
                        let ItemKind::Class(c) = &self.item(*item).kind else { return Err(lost()) };
                        let name = c.name.name.clone();
                        format!("{}_n", self.item_c(*item, &name))
                    }
                    _ => return self.not_yet(fx, e, "get() on a handle of this type"),
                };
                let h = self.expr(fx, base)?;
                let p = fx.temp("void*", &format!("lumart_ref_get({h}, {cls})"));
                // A view's option is the pointer itself; a mod class's is the pair.
                let ct = self.c_type(t).ok_or_else(lost)?;
                if ct == "void*" {
                    return Ok(p);
                }
                Ok(fx.temp(&ct, &format!("{p} != 0 ? ({ct}){{1, {p}}} : ({ct}){{0}}")))
            }
            // C4: a row (0 to 2) or column (0 to 3) of a matrix; another index S0902.
            Ty::Value(Value::Mat34) if name == "row" || name == "column" => {
                let mv = self.expr(fx, base)?;
                let mv = fx.temp("lm_mat34", &mv);
                let x = &args[0].value;
                let iv = self.expr(fx, x)?;
                let from = self.ty(m, x);
                let iv = self.coerce(fx, &iv, &from, &Ty::Int(Scalar::I32), x.at)?;
                let i = fx.temp("int32_t", &iv);
                let n = if name == "row" { 3 } else { 4 };
                fx.fail_if(&format!("({i} < 0 || {i} >= {n}) && lumart_index_error({i}, {n}u, {file}, {line})"));
                Ok(if name == "row" {
                    fx.temp("lm_vec4", &format!("(lm_vec4){{{mv}.m[{i}][0], {mv}.m[{i}][1], {mv}.m[{i}][2], {mv}.m[{i}][3]}}"))
                } else {
                    fx.temp("lm_vec3", &format!("(lm_vec3){{{mv}.m[0][{i}], {mv}.m[1][{i}], {mv}.m[2][{i}]}}"))
                })
            }
            // C2: a content value's name is the value itself (content values are texts here).
            Ty::Text if name == "name" => {
                let s = self.expr(fx, base)?;
                Ok(fx.temp("lumart_text", &s))
            }
            Ty::Text => {
                if !matches!(name, "len" | "starts_with" | "ends_with" | "find" | "slice" | "lower" | "upper" | "parse_int" | "parse_float" | "chars" | "split" | "bytes") {
                    return self.not_yet(fx, e, &format!("text.{name}()"));
                }
                let s = self.expr(fx, base)?;
                let s = fx.temp("lumart_text", &s);
                let mut a = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let want = if name == "slice" { Ty::Int(Scalar::I32) } else { Ty::Text };
                    let v = self.expr(fx, &arg.value)?;
                    let from = self.ty(m, &arg.value);
                    let v = self.coerce(fx, &v, &from, &want, arg.value.at)?;
                    let ct = self.c_type(&want).ok_or_else(lost)?;
                    let _ = i;
                    a.push(fx.temp(&ct, &v));
                }
                match name {
                    "len" => {
                        let r = fx.fresh("int32_t");
                        fx.fail_if(&format!("lumart_text_len(&{r}, {s}, {file}, {line}, {src})"));
                        Ok(r)
                    }
                    "starts_with" => Ok(fx.temp("uint8_t", &format!("lumart_text_starts({s}, {})", a[0]))),
                    "lower" | "upper" => {
                        let r = fx.fresh("lumart_text");
                        fx.fail_if(&format!("lumart_text_case(&{r}, {s}, {}, {file}, {line})", u8::from(name == "upper")));
                        Ok(r)
                    }
                    // A number or none (no script error): the value through an out-parameter, then the option.
                    "parse_int" | "parse_float" => {
                        let ct = self.c_type(t).ok_or_else(lost)?;
                        let (vc, f) = if name == "parse_int" { ("int64_t", "lumart_text_parse_i64") } else { ("double", "lumart_text_parse_f64") };
                        let v = fx.fresh(vc);
                        let ok = fx.temp("uint8_t", &format!("{f}({s}, &{v})"));
                        Ok(fx.temp(&ct, &format!("{ok} ? ({ct}){{1, {v}}} : ({ct}){{0}}")))
                    }
                    "ends_with" => Ok(fx.temp("uint8_t", &format!("lumart_text_ends({s}, {})", a[0]))),
                    "bytes" => {
                        let r = fx.fresh("lumart_bytes");
                        fx.fail_if(&format!("lumart_bytes_of_text(&{r}, {s}, {file}, {line})"));
                        Ok(r)
                    }
                    "chars" | "split" => {
                        let td = self.desc(&Ty::Text).ok_or_else(lost)?;
                        let r = fx.fresh("lumart_list");
                        if name == "chars" {
                            fx.fail_if(&format!("lumart_text_chars(&{r}, {s}, {td}, {file}, {line})"));
                        } else {
                            fx.fail_if(&format!("lumart_text_split(&{r}, {s}, {}, {td}, {file}, {line})", a[0]));
                        }
                        Ok(r)
                    }
                    "find" => {
                        // The byte offset of the first match, none when there is none (an offset past i32 is S0904).
                        let ct = self.c_type(t).ok_or_else(lost)?;
                        let k = fx.temp("int64_t", &format!("lumart_text_find({s}, {})", a[0]));
                        fx.fail_if(&format!("{k} > 2147483647 && lumart_overflow({file}, {line}, \"i32\", {src})"));
                        Ok(fx.temp(&ct, &format!("({k} >= 0) ? ({ct}){{1, (int32_t){k}}} : ({ct}){{0}}")))
                    }
                    _ => {
                        let r = fx.fresh("lumart_text");
                        fx.fail_if(&format!("lumart_text_slice(&{r}, {s}, (int64_t){}, (int64_t){}, {file}, {line})", a[0], a[1]));
                        Ok(r)
                    }
                }
            }
            other => {
                let what = format!("{}.{name}()", self.ty_text(other));
                self.not_yet(fx, e, &what)
            }
        }
    }

    /// Whether `l` and `r` (C places of type `t`, read more than once) are equal, as a C expression of 0 or 1; None
    /// for a type the back end cannot compare yet.
    pub(super) fn eq_c(&mut self, l: &str, r: &str, t: &Ty) -> Option<String> {
        Some(match t {
            Ty::List(x) => format!("lumart_list_eq({l}, {r}, {})", self.desc(x)?),
            Ty::Set(x) => format!("lumart_map_eq({l}, {r}, {}, 0)", self.desc(x)?),
            Ty::Map(k, v) => format!("lumart_map_eq({l}, {r}, {}, {})", self.desc(k)?, self.desc(v)?),
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Game(GameTy::Enum(_)) => format!("({l} == {r})"),
            Ty::Text | Ty::Json => format!("lumart_text_eq({l}, {r})"),
            Ty::Ref(_) => format!("lumart_ref_eq({l}, {r})"),
            Ty::Bytes => format!("lumart_bytes_eq({l}, {r})"),
            Ty::Value(Value::Mat34) => {
                let all: Vec<String> = (0..3).flat_map(|i| (0..4).map(move |j| (i, j))).map(|(i, j)| format!("{l}.m[{i}][{j}] == {r}.m[{i}][{j}]")).collect();
                format!("({})", all.join(" && "))
            }
            Ty::Value(v) if !components(*v).is_empty() => {
                let all: Vec<String> = components(*v).iter().map(|c| format!("{l}.{c} == {r}.{c}")).collect();
                format!("({})", all.join(" && "))
            }
            Ty::Opt(x) if is_view(x) => format!("({l} == {r})"),
            Ty::Opt(x) => {
                let inner = self.eq_c(&format!("{l}.value"), &format!("{r}.value"), x)?;
                format!("({l}.some == {r}.some && (!{l}.some || {inner}))")
            }
            Ty::Tuple(xs) => {
                let mut all = Vec::new();
                for (i, x) in xs.iter().enumerate() {
                    all.push(self.eq_c(&format!("{l}._{i}"), &format!("{r}._{i}"), x)?);
                }
                if all.is_empty() { "1".into() } else { format!("({})", all.join(" && ")) }
            }
            Ty::Struct(item) => {
                let item = *item;
                let ItemKind::Struct(s) = &self.item(item).kind else { return None };
                let names: Vec<String> = s.fields.iter().map(|f| f.name.name.clone()).collect();
                let mut all = Vec::new();
                for n in names {
                    let ft = self.declared_ty(item, &format!(".{n}"));
                    all.push(self.eq_c(&format!("{l}.{n}"), &format!("{r}.{n}"), &ft)?);
                }
                if all.is_empty() { "1".into() } else { format!("({})", all.join(" && ")) }
            }
            // The same case, and that case's values equal.
            Ty::Enum(item) => {
                let item = *item;
                let ItemKind::Enum(d) = &self.item(item).kind else { return None };
                let cases: Vec<(String, Vec<String>)> = d.cases.iter().map(|c| (c.name.name.clone(), c.fields.iter().map(|f| f.name.name.clone()).collect())).collect();
                let mut all = vec![format!("{l}.tag == {r}.tag")];
                for (k, (case, fields)) in cases.iter().enumerate() {
                    if fields.is_empty() {
                        continue;
                    }
                    let mut parts = Vec::new();
                    for f in fields {
                        let ft = self.declared_ty(item, &format!("{case}.{f}"));
                        parts.push(self.eq_c(&format!("{l}.u.{case}.{f}"), &format!("{r}.u.{case}.{f}"), &ft)?);
                    }
                    all.push(format!("({l}.tag != {k}u || ({}))", parts.join(" && ")));
                }
                format!("({})", all.join(" && "))
            }
            _ => return None,
        })
    }

    /// Writes a text value (a string literal with its interpolations) into builder `b`.
    pub(super) fn text_into(&mut self, fx: &mut Fx, b: &str, e: &Expr) -> Result<(), NotWritten> {
        let ExprKind::Str(lit) = &e.kind else { return self.not_yet(fx, e, "a text that is no string literal") };
        if lit.prefix != StrPrefix::None && lit.prefix != StrPrefix::Raw {
            return self.not_yet(fx, e, "bytes as text");
        }
        for part in &lit.parts {
            match part {
                StrPart::Text(raw) => {
                    let bytes = if lit.prefix == StrPrefix::Raw { raw.as_bytes().to_vec() } else { unescape(raw) };
                    if bytes.is_empty() {
                        continue;
                    }
                    fx.emit(format!("lumart_sb_bytes(&{b}, {}, {});", c_bytes(&bytes), bytes.len()));
                }
                StrPart::Expr(x) => self.value_text_into(fx, b, x, false)?,
            }
        }
        Ok(())
    }

    /// A value as text (`json`: as JSON; else as interpolation writes it).
    pub(super) fn value_text_into(&mut self, fx: &mut Fx, b: &str, e: &Expr, json: bool) -> Result<(), NotWritten> {
        let m = fx.module;
        let t = self.ty(m, e);
        if let (ExprKind::Str(_), Ty::Text) = (&e.kind, &t) {
            if !json {
                return self.text_into(fx, b, e);
            }
            // A literal text as a JSON string: made in a builder of its own, then quoted.
            let inner = fx.builder();
            self.text_into(fx, &inner, e)?;
            fx.fail_if(&format!("lumart_sb_done(&{inner}, {}, {})", c_string(&fx.file), e.at.line));
            fx.emit(format!("lumart_sb_json_str(&{b}, {inner}.p, {inner}.len);"));
            return Ok(());
        }
        let v = self.expr(fx, e)?;
        self.value_text(fx, b, &v, &t, e, json)
    }

    pub(super) fn value_text(&mut self, fx: &mut Fx, b: &str, v: &str, t: &Ty, e: &Expr, json: bool) -> Result<(), NotWritten> {
        let inline = std::mem::replace(&mut self.writer_inline, false);
        let (file, line, src) = if fx.site { ("file".to_string(), "line".to_string(), "src".to_string()) } else { (c_string(&fx.file), e.at.line.to_string(), c_string(&expr_text(e))) };
        // A struct or an enum with values is written by a function of its own (one per type and form): a type that
        // holds a list of itself is written by calls, not in place without end.
        if !inline && matches!(t, Ty::Struct(_)) || matches!(t, Ty::Enum(i) if !inline && !self.enum_plain(*i)) {
            let w = self.text_writer(fx, t, e, json)?;
            let ct = self.c_type(t).ok_or_else(lost)?;
            let tmp = fx.temp(&ct, v);
            fx.fail_if(&format!("{w}(&{b}, &{tmp}, {file}, {line}, {src})"));
            return Ok(());
        }
        match t {
            Ty::Bool => fx.emit(format!("lumart_sb_bool(&{b}, {v});")),
            Ty::Int(s) if signed(*s) => fx.emit(format!("lumart_sb_i64(&{b}, (int64_t){v});")),
            Ty::Int(_) => fx.emit(format!("lumart_sb_u64(&{b}, (uint64_t){v});")),
            // As json a game enum is its value's name, the first declared; a value of no name its number.
            Ty::Game(GameTy::Enum(en)) if json => {
                let cases = self.game_enum_values(en);
                self.named_value_text(fx, b, v, cases, true);
            }
            Ty::Game(GameTy::Enum(_)) => fx.emit(format!("lumart_sb_i64(&{b}, (int64_t){v});")),
            // A service's enum and `svc.Answer` by its case's name (`NotFound`), as json a string; a value of no
            // case its number (a newer host's answer).
            Ty::Service { service, name } if service == "svc" && name == "Answer" => {
                let cases = self.game.services.answers.clone();
                self.named_value_text(fx, b, v, cases, json);
            }
            Ty::Service { service, name } if self.game.services.faces.get(service).is_some_and(|f| f.enums.contains_key(name)) => {
                let cases = self.game.services.faces[service].enums[name].clone();
                self.named_value_text(fx, b, v, cases, json);
            }
            // A json.Value is its JSON text, as json and as text.
            Ty::Json => {
                let tmp = fx.temp("lumart_text", v);
                fx.emit(format!("lumart_sb_bytes(&{b}, lumart_text_p({tmp}), lumart_text_n({tmp}));"));
            }
            Ty::Text => {
                let tmp = fx.temp("lumart_text", v);
                if json {
                    fx.emit(format!("lumart_sb_json_str(&{b}, lumart_text_p({tmp}), lumart_text_n({tmp}));"));
                } else {
                    fx.emit(format!("lumart_sb_bytes(&{b}, lumart_text_p({tmp}), lumart_text_n({tmp}));"));
                }
            }
            Ty::Float(Scalar::F64) => {
                let tmp = fx.temp("double", v);
                if json {
                    fx.fail_if(&format!("lumart_finite_f64({tmp}, {file}, {line}, {src})"));
                }
                fx.emit(format!("lumart_sb_f64(&{b}, {tmp});"));
            }
            Ty::Float(_) => {
                let tmp = fx.temp("float", v);
                if json {
                    fx.fail_if(&format!("lumart_finite_f32(&{tmp}, 1, {file}, {line}, {src})"));
                }
                fx.emit(format!("lumart_sb_f32(&{b}, {tmp});"));
            }
            Ty::Value(val) if !components(*val).is_empty() => {
                let n = components(*val).len();
                let tmp = fx.temp(value_c(*val), v);
                let first = components(*val)[0];
                if json {
                    fx.fail_if(&format!("lumart_finite_f32(&{tmp}.{first}, {n}, {file}, {line}, {src})"));
                }
                fx.emit(format!("lumart_sb_vec(&{b}, &{tmp}.{first}, {n});"));
            }
            // The matrix: its three rows of four.
            Ty::Value(Value::Mat34) => {
                let tmp = fx.temp("lm_mat34", v);
                if json {
                    fx.fail_if(&format!("lumart_finite_f32(&{tmp}.m[0][0], 12, {file}, {line}, {src})"));
                }
                fx.emit(format!("lumart_sb_str(&{b}, \"[\");"));
                for i in 0..3 {
                    if i > 0 {
                        fx.emit(format!("lumart_sb_str(&{b}, \", \");"));
                    }
                    fx.emit(format!("lumart_sb_vec(&{b}, &{tmp}.m[{i}][0], 4);"));
                }
                fx.emit(format!("lumart_sb_str(&{b}, \"]\");"));
            }
            // An option: its value, or `null` / `none`.
            Ty::Opt(x) if !is_view(x) => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let tmp = fx.temp(&ct, v);
                fx.emit(format!("if ({tmp}.some) {{"));
                fx.depth += 1;
                let inner = format!("{tmp}.value");
                self.value_text(fx, b, &inner, &x.clone(), e, json)?;
                fx.depth -= 1;
                fx.emit("} else {".into());
                fx.emit(format!("    lumart_sb_str(&{b}, \"{}\");", if json { "null" } else { "none" }));
                fx.emit("}".into());
            }
            // A tuple as a JSON array.
            Ty::Tuple(xs) => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                let tmp = fx.temp(&ct, v);
                fx.emit(format!("lumart_sb_str(&{b}, \"{}\");", if json { "[" } else { "(" }));
                for (i, x) in xs.clone().iter().enumerate() {
                    if i > 0 {
                        fx.emit(format!("lumart_sb_str(&{b}, \", \");"));
                    }
                    self.value_text(fx, b, &format!("{tmp}._{i}"), x, e, json)?;
                }
                fx.emit(format!("lumart_sb_str(&{b}, \"{}\");", if json { "]" } else { ")" }));
            }
            Ty::List(_) | Ty::Set(_) | Ty::Map(..) => self.container_text(fx, b, v, t, e, json)?,
            Ty::Bytes => self.container_text(fx, b, v, &Ty::List(Box::new(Ty::Int(Scalar::U8))), e, json)?,
            // A struct as a JSON object of its fields, in declared order.
            Ty::Struct(item) => {
                let item = *item;
                let ct = self.c_type(t).ok_or_else(lost)?;
                let tmp = fx.temp(&ct, v);
                let ItemKind::Struct(s) = &self.item(item).kind else { return Err(lost()) };
                let fields = s.fields.clone();
                fx.emit(format!("lumart_sb_str(&{b}, \"{{\");"));
                for (i, f) in fields.iter().enumerate() {
                    let sep = if i > 0 { ", " } else { "" };
                    let key = if json { format!("{sep}\\\"{}\\\": ", f.name.name) } else { format!("{sep}{}: ", f.name.name) };
                    fx.emit(format!("lumart_sb_str(&{b}, \"{key}\");"));
                    let ft = self.declared_ty(item, &format!(".{}", f.name.name));
                    self.value_text(fx, b, &format!("{tmp}.{}", f.name.name), &ft, e, json)?;
                }
                fx.emit(format!("lumart_sb_str(&{b}, \"}}\");"));
            }
            // An enum with values: as json a case with values is `{"Case": {fields by name}}`, one without its name
            //; as text `Case(a: 1)` or `Case {a: 1}` as declared, or its name.
            Ty::Enum(item) if !self.enum_plain(*item) => {
                let item = *item;
                let ct = self.c_type(t).ok_or_else(lost)?;
                let tmp = fx.temp(&ct, v);
                let ItemKind::Enum(d) = &self.item(item).kind else { return Err(lost()) };
                let cases = d.cases.clone();
                for (k, c) in cases.iter().enumerate() {
                    let n = &c.name.name;
                    fx.emit(format!("if ({tmp}.tag == {k}u) {{"));
                    fx.depth += 1;
                    if c.fields.is_empty() {
                        let shown = if json { format!("\\\"{n}\\\"") } else { n.clone() };
                        fx.emit(format!("lumart_sb_str(&{b}, \"{shown}\");"));
                    } else {
                        let open = if json { format!("{{\\\"{n}\\\": {{") } else if c.braces { format!("{n} {{") } else { format!("{n}(") };
                        fx.emit(format!("lumart_sb_str(&{b}, \"{open}\");"));
                        for (i, f) in c.fields.iter().enumerate() {
                            let sep = if i > 0 { ", " } else { "" };
                            let key = if json { format!("{sep}\\\"{}\\\": ", f.name.name) } else { format!("{sep}{}: ", f.name.name) };
                            fx.emit(format!("lumart_sb_str(&{b}, \"{key}\");"));
                            let ft = self.declared_ty(item, &format!("{n}.{}", f.name.name));
                            self.value_text(fx, b, &format!("{tmp}.u.{n}.{}", f.name.name), &ft, e, json)?;
                        }
                        let close = if json { "}}" } else if c.braces { "}" } else { ")" };
                        fx.emit(format!("lumart_sb_str(&{b}, \"{close}\");"));
                    }
                    fx.depth -= 1;
                    fx.emit("}".into());
                }
            }
            // A plain enum by its case's name, as a JSON string.
            Ty::Enum(item) if self.enum_plain(*item) => {
                let item = *item;
                let ct = self.c_type(t).ok_or_else(lost)?;
                let tmp = fx.temp(&ct, v);
                let ItemKind::Enum(d) = &self.item(item).kind else { return Err(lost()) };
                let names: Vec<String> = d.cases.iter().map(|c| c.name.name.clone()).collect();
                for (i, n) in names.iter().enumerate() {
                    let shown = if json { format!("\\\"{n}\\\"") } else { n.clone() };
                    fx.emit(format!("if ({tmp}.tag == {i}u) lumart_sb_str(&{b}, \"{shown}\");"));
                }
            }
            other => {
                let what = format!("{} as text", self.ty_text(other));
                return self.not_yet(fx, e, &what);
            }
        }
        Ok(())
    }

    /// An `int32_t` value `v` by the name of its first case of that value in `cases`, quoted when `json`; a value of
    /// no case its number.
    fn named_value_text(&mut self, fx: &mut Fx, b: &str, v: &str, cases: Vec<(String, i64)>, json: bool) {
        let tmp = fx.temp("int32_t", v);
        let mut firsts: Vec<(String, i64)> = Vec::new();
        for (name, n) in cases {
            if !firsts.iter().any(|(_, m)| *m == n) {
                firsts.push((name, n));
            }
        }
        let mut chain = String::new();
        for (name, n) in firsts {
            let shown = if json { format!("\"{name}\"") } else { name };
            let _ = write!(chain, "if ({tmp} == (int32_t){n}) lumart_sb_str(&{b}, {}); else ", c_string(&shown));
        }
        fx.emit(format!("{chain}lumart_sb_i64(&{b}, (int64_t){tmp});"));
    }

    pub(super) fn json_into(&mut self, fx: &mut Fx, b: &str, e: &Expr) -> Result<(), NotWritten> {
        self.value_text_into(fx, b, e, true)
    }
}

/// The record of a view or an optional view.
fn view_record(t: &Ty) -> Option<String> {
    match t {
        Ty::Game(GameTy::View { record, .. }) => Some(record.clone()),
        Ty::Opt(x) => view_record(x),
        _ => None,
    }
}

/// The type of a game scalar, value or enum as the script holds it.
fn game_scalar_ty(g: &GameTy) -> Option<Ty> {
    Some(match g {
        GameTy::Scalar(Scalar::Bool) => Ty::Bool,
        GameTy::Scalar(s @ (Scalar::F32 | Scalar::F64)) => Ty::Float(*s),
        GameTy::Scalar(s) => Ty::Int(*s),
        GameTy::Value(v) => Ty::Value(*v),
        GameTy::Enum(e) => Ty::Game(GameTy::Enum(e.clone())),
        _ => return None,
    })
}

fn unsigned_of(s: Scalar) -> Scalar {
    match s {
        Scalar::I8 => Scalar::U8,
        Scalar::I16 => Scalar::U16,
        Scalar::I32 => Scalar::U32,
        Scalar::I64 => Scalar::U64,
        other => other,
    }
}

fn clamp01(v: &str) -> String {
    format!("({v} < 0.0f ? 0.0f : {v} > 1.0f ? 1.0f : {v})")
}

/// A colour's channels stay in 0..1 (docs/modding/language/language.md, section 3); other values as they are.
fn colour_clamp(v: Value, parts: Vec<String>) -> Vec<String> {
    if v != Value::Color {
        return parts;
    }
    parts.into_iter().map(|p| clamp01(&format!("({p})"))).collect()
}

/// `some(v)` for a `T?` in C: an option of a view is the pointer itself, any other the tagged pair.
fn some_c(t: &Ty, ct: &str, v: &str) -> String {
    if matches!(t, Ty::Opt(x) if is_view(x)) {
        v.to_string()
    } else {
        format!("({ct}){{1, {v}}}")
    }
}

/// The C type a game result `T &` of a number or a vector is read through (the game's layout is the script's, as
/// place_at reads fields); None for any other (a colour is four bytes in the game).
fn place_result_c(of: &GameTy) -> Option<&'static str> {
    match of {
        GameTy::Scalar(s) => Some(scalar_c(*s)),
        GameTy::Value(Value::Color) => None,
        GameTy::Value(v) if *v == Value::Mat34 || !components(*v).is_empty() => Some(value_c(*v)),
        _ => None,
    }
}

fn strip(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren(x) => strip(x),
        _ => e,
    }
}

/// A string literal with nothing interpolated: static bytes in the library's image.
fn text_literal_arg(e: &Expr) -> bool {
    matches!(&strip(e).kind, ExprKind::Str(l) if l.parts.iter().all(|p| matches!(p, StrPart::Text(_))))
}

/// A number literal's text without `_` and without a `f32`/`f64`/integer suffix: (digits, radix).
pub(super) fn number_text(text: &str) -> (String, u32) {
    let t: String = text.chars().filter(|c| *c != '_').collect();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return (h.to_string(), 16);
    }
    if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return (b.to_string(), 2);
    }
    let t = t.trim_end_matches("f32").trim_end_matches("f64").to_string();
    (t, 10)
}

pub(super) fn expr_name(e: &ExprKind) -> &'static str {
    match e {
        ExprKind::Number(_) => "a number",
        ExprKind::Color(_) => "a colour literal",
        ExprKind::Str(_) => "a string",
        ExprKind::True | ExprKind::False => "a bool",
        ExprKind::None => "none",
        ExprKind::SelfValue => "self",
        ExprKind::Result => "result",
        ExprKind::Proceed => "proceed()",
        ExprKind::Ref(_) => "ref(...)",
        ExprKind::If { .. } => "if as a value",
        ExprKind::Match { .. } => "match",
        ExprKind::Closure { .. } => "a closure",
        ExprKind::Paren(_) => "( )",
        ExprKind::Tuple(_) => "a tuple",
        ExprKind::List(_) => "a list",
        ExprKind::Braces(_) => "a map or set",
        ExprKind::Content { .. } => "a content name",
        ExprKind::Game(_) => "a game name in backquotes",
        ExprKind::Name { .. } => "a name",
        ExprKind::StructLit { .. } => "a struct literal",
        ExprKind::Call { .. } => "a call",
        ExprKind::Index { .. } => "[ ]",
        ExprKind::Member { optional: true, .. } => "?.",
        ExprKind::Member { .. } => "a member",
        ExprKind::Unwrap(_) => "!",
        ExprKind::Neg(_) => "-",
        ExprKind::Not(_) => "not",
        ExprKind::Binary { .. } => "an operator",
        ExprKind::Wait { .. } | ExprKind::WaitUntil(_) => "wait",
        ExprKind::Start(_) => "start",
        ExprKind::As { .. } => "as",
    }
}

pub(super) fn binop_text(op: BinOp) -> &'static str {
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

/// An expression's source, short, for a run-time report's argument (S0904 `i32 overflow in a + b`).
/// How a call is given its object or `self`: an expression to read, or a value read already (`a?.f()`).
pub(super) enum Recv<'e> {
    Expr(&'e Expr),
    Value(String),
}

pub(super) fn expr_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Name { name, .. } => name.name.clone(),
        ExprKind::Number(t) => t.clone(),
        ExprKind::Paren(x) => format!("({})", expr_text(x)),
        ExprKind::Binary { op, left, right } => format!("{} {} {}", expr_text(left), binop_text(*op), expr_text(right)),
        ExprKind::Neg(x) => format!("-{}", expr_text(x)),
        ExprKind::Member { base, name, optional: false, .. } => format!("{}.{}", expr_text(base), name.name),
        ExprKind::Member { base, name, optional: true, .. } => format!("{}?.{}", expr_text(base), name.name),
        ExprKind::Index { base, index } => format!("{}[{}]", expr_text(base), expr_text(index)),
        ExprKind::True => "true".into(),
        ExprKind::False => "false".into(),
        ExprKind::None => "none".into(),
        ExprKind::Not(x) => format!("not {}", expr_text(x)),
        ExprKind::Result => "result".into(),
        ExprKind::SelfValue => "self".into(),
        ExprKind::As { value, .. } => format!("{} as ...", expr_text(value)),
        ExprKind::Call { callee, .. } => format!("{}(...)", expr_text(callee)),
        ExprKind::Unwrap(x) => format!("{}!", expr_text(x)),
        ExprKind::Game(g) => g.text.clone(),
        _ => "...".into(),
    }
}

