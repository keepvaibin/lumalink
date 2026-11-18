//! C in the same mod (docs/modding/language/language.md, section 13, docs/modding/language/compiled-form.md, section 9): `use c "x.h"` includes the mod's own header in `lm.h` (so a
//! declaration that disagrees with the script's is clang's error); `extern c fn` is a prototype and a direct call;
//! `export c fn` is the script's function plus a C function of that name. `lum/lm_exports.h` is the mod's C's view of
//! its script: the types that cross (lumart.h's), its `c struct`s, a getter for each handle type that crosses, and its
//! exports.
//!
//! Across, by value: integers, floats, `bool` (C `_Bool`), `vec2` to `vec4`, `quat`, `mat34` and `color` (lumart.h's
//! `lm_vec3` and the rest), a `c struct` (the C struct of its name as written, its fields any of these but text and
//! bytes), and `ref<C>` (a `lumart_ref`: the handle as it is, which C may keep; `lm_ref_get_<C>` gives the object, or
//! null once it is gone). `text` and `bytes` cross as (pointer, length) for the call's duration (an extern's
//! parameters; an export's text parameters, copied into a text of the script's own). A fault in the C is the guard's,
//! not a script error.

use super::{c_string, scalar_c, value_c, Ex, Fx, Gen, Ny};
use crate::ast::{self, Expr, Ident, ItemKind, Param};
use crate::check::{ItemRef, Ty};
use crate::game::GameTy;
use std::fmt::Write as _;

/// How a value of a type crosses to C.
enum Side {
    /// As it is, by value: a number, a vector, `quat`, `mat34`, `color`, a `c struct` or a handle; its C type.
    Plain(String),
    Bool,
    Text,
    Bytes,
}

/// A parameter as C declares it: one or two C parameters.
fn c_param(s: &Side, name: &str) -> String {
    match s {
        Side::Plain(ct) => format!("{ct} {name}"),
        Side::Bool => format!("_Bool {name}"),
        Side::Text => format!("const char* {name}, uint64_t {name}_len"),
        Side::Bytes => format!("const uint8_t* {name}, uint64_t {name}_len"),
    }
}

impl<'a> Gen<'a> {
    fn param_name(p: &Param, i: usize) -> String {
        p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone())
    }

    /// Whether struct `item` is a `c struct`.
    pub(super) fn is_c_struct(&self, item: ItemRef) -> bool {
        matches!(&self.item(item).kind, ItemKind::Struct(s) if s.c)
    }

    fn side(&mut self, t: &Ty) -> Option<Side> {
        Some(match t {
            Ty::Int(s) | Ty::Float(s) => Side::Plain(scalar_c(*s).into()),
            Ty::Value(v) => Side::Plain(value_c(*v).into()),
            Ty::Struct(i) if self.is_c_struct(*i) => Side::Plain(self.struct_type(*i)?),
            Ty::Ref(of) => Side::Plain(self.ref_getter(of)?),
            Ty::Bool => Side::Bool,
            Ty::Text => Side::Text,
            Ty::Bytes => Side::Bytes,
            _ => return None,
        })
    }

    /// A `c struct` field's C type: what crosses by value (`bool` as C's); None for any other type.
    pub(super) fn c_field(&mut self, t: &Ty) -> Option<String> {
        match self.side(t)? {
            Side::Plain(ct) => Some(ct),
            Side::Bool => Some("bool".into()),
            Side::Text | Side::Bytes => None,
        }
    }

    /// `ref<C>` crossing to C: `lumart_ref`, and `lm_ref_get_<C>` in lm_exports.h, the object as C (its view) or null.
    fn ref_getter(&mut self, of: &Ty) -> Option<String> {
        let (name, cls) = match of {
            Ty::Game(GameTy::View { record, .. }) => {
                let record = record.clone();
                let slot = self.class_slot(&record);
                (record, slot)
            }
            // A mod's class: its own number (define_class's).
            Ty::Class(item) => {
                let ItemKind::Class(c) = &self.item(*item).kind else { return None };
                let name = c.name.name.clone();
                let number = format!("{}_n", self.item_c(*item, &name));
                (name, number)
            }
            _ => return None,
        };
        let c_name: String = name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect();
        let getter = format!(
            "/* A ref<{name}>'s object, or null once it is gone; on the game thread only. */\nextern int32_t {cls};\nstatic inline void* lm_ref_get_{c_name}(lumart_ref r) {{\n    return lumart_ref_get(r, {cls});\n}}"
        );
        self.ref_getters.insert(c_name, getter);
        Some("lumart_ref".into())
    }

    /// The C result type of an extern or export (None: it gives none); Err when it does not cross.
    fn c_result(&mut self, item: ItemRef, has: bool) -> Result<Option<(Side, Ty)>, Ny> {
        if !has {
            return Ok(None);
        }
        let t = self.declared_ty(item, "->");
        match self.side(&t) {
            Some(Side::Text) | Some(Side::Bytes) => Err(ny!("CResult", "type" => self.ty_text(&t))),
            Some(s) => Ok(Some((s, t))),
            None if t == Ty::Unit => Ok(None),
            None => Err(ny!("CResult", "type" => self.ty_text(&t))),
        }
    }

    /// `use c "x.h"`: included by lm.h, so the script's declarations are checked against the mod's own.
    pub(super) fn use_c(&mut self, header: &ast::StrLit) {
        let written: String = header.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect();
        self.c_headers.insert(written);
    }

    /// `extern c fn name(params) [-> type]`: its prototype in lm.h, under the name as written.
    pub(super) fn extern_fn(&mut self, m: usize, me: ItemRef, name: &Ident, params: &[Param], ret: bool) {
        let mut ps = Vec::new();
        for (i, p) in params.iter().enumerate() {
            // The C gets a copy: a var's write-back would be lost without a word.
            if p.var {
                self.say(m, name.at, ny!("ExternVar"));
                return;
            }
            let pn = Self::param_name(p, i);
            let t = self.declared_ty(me, &format!("({pn})"));
            let Some(s) = self.side(&t) else {
                let what = ny!("CParam", "type" => self.ty_text(&t));
                self.say(m, name.at, what);
                return;
            };
            ps.push(c_param(&s, &format!("p{i}")));
        }
        let r = match self.c_result(me, ret) {
            Ok(Some((Side::Plain(ct), _))) => ct,
            Ok(Some((Side::Bool, _))) => "_Bool".into(),
            Ok(_) => "void".into(),
            Err(what) => {
                self.say(m, name.at, what);
                return;
            }
        };
        let ps = if ps.is_empty() { "void".to_string() } else { ps.join(", ") };
        // At the script's line (M-124): the mod's header is included before it, so a declaration there that does not
        // agree is the C compiler's "conflicting types" at the `extern c fn` line, not at a line of lm.h.
        let file = format!("{}.lum", self.sources[m].path);
        self.c_protos.insert(format!("#line {} {}\n{r} {}({ps});", name.at.line, c_string(&file), name.name));
    }

    /// A call of an `extern c fn`: the arguments in order (text and bytes as pointer and length, valid for the call),
    /// the C function called directly.
    pub(super) fn call_extern(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, name: &str, params: &[Param], ret: bool, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        if args.len() != params.len() || args.iter().any(|a| a.name.is_some()) {
            return self.not_yet(fx, e, ny!("CCallNamed"));
        }
        let mut list = Vec::new();
        for (i, (a, p)) in args.iter().zip(params).enumerate() {
            let pt = self.declared_ty(item, &format!("({})", Self::param_name(p, i)));
            let v = self.expr(fx, &a.value)?;
            let from = self.ty(m, &a.value);
            let v = self.coerce(fx, &v, &from, &pt, a.value.at)?;
            let Some(s) = self.side(&pt) else { return self.not_yet(fx, e, ny!("CArg")) };
            list.push(match s {
                Side::Plain(ct) => fx.temp(&ct, &v),
                Side::Bool => format!("(_Bool){}", fx.temp("uint8_t", &v)),
                Side::Text => {
                    let t = fx.temp("lumart_text", &v);
                    format!("lumart_text_p({t}), lumart_text_n({t})")
                }
                Side::Bytes => {
                    let t = fx.temp("lumart_bytes", &v);
                    format!("({t} != 0 ? (const uint8_t*)lumart_bytes_p({t}) : (const uint8_t*)0), lumart_list_n({t})")
                }
            });
        }
        let call = format!("{name}({})", list.join(", "));
        match self.c_result(item, ret) {
            Ok(Some((Side::Plain(ct), _))) => Ok(fx.temp(&ct, &call)),
            Ok(Some((Side::Bool, _))) => Ok(fx.temp("uint8_t", &format!("(uint8_t)({call} != 0)"))),
            Ok(_) => {
                fx.emit(format!("{call};"));
                Ok(String::new())
            }
            Err(what) => self.not_yet(fx, e, what),
        }
    }

    /// `export c fn name(params) [-> type] { body }`: the body as the script's function `lm_<m>__name`, and the C
    /// function `name` the mod's C calls: it copies text arguments into texts of the script's own, calls the body,
    /// and gives its result (zero when the body ended in a script error, which lumart has reported, or when called off
    /// the game thread, S0901).
    pub(super) fn export_fn(&mut self, m: usize, me: ItemRef, f: &ast::FnDecl, file: &str) -> Option<String> {
        let internal = self.item_c(me, &f.name.name);
        let mut out = self.function(m, me, "", &internal, f, file)?;
        let mut c_ps = Vec::new();
        let mut pre = String::new();
        // The script's own copies of the text arguments, given back after the call.
        let mut texts: Vec<String> = Vec::new();
        let mut call = Vec::new();
        let (src, line) = (c_string(file), f.name.at.line);
        let r = match self.c_result(me, f.ret.is_some()) {
            Ok(r) => r,
            Err(what) => {
                self.say(m, f.name.at, what);
                return None;
            }
        };
        // The result type, and its zero: 0 for a number or bool, all fields zero for a vector, a struct or a handle.
        let (ret_c, zero) = match &r {
            Some((Side::Plain(ct), Ty::Int(_) | Ty::Float(_))) => (ct.clone(), "0".to_string()),
            Some((Side::Plain(ct), _)) => (ct.clone(), format!("({ct}){{0}}")),
            Some((Side::Bool, _)) => ("bool".to_string(), "0".to_string()),
            _ => ("void".to_string(), String::new()),
        };
        let back = if zero.is_empty() { "return;".to_string() } else { format!("return {zero};") };
        for (i, p) in f.params.iter().enumerate() {
            let pn = Self::param_name(p, i);
            let t = self.declared_ty(me, &format!("({pn})"));
            if p.var {
                self.say(m, f.name.at, ny!("ExportVar"));
                return None;
            }
            match self.side(&t) {
                Some(Side::Plain(ct)) => {
                    c_ps.push(format!("{ct} p{i}"));
                    call.push(format!("p{i}"));
                }
                Some(Side::Bool) => {
                    c_ps.push(format!("bool p{i}"));
                    call.push(format!("(uint8_t)(p{i} != 0)"));
                }
                Some(Side::Text) => {
                    c_ps.push(format!("const char* p{i}, uint64_t p{i}_len"));
                    // On a failed copy (S0911, reported) the copies made so far are given back.
                    let fail: String = texts.iter().map(|t| format!("lm_rel_lumart_text(&{t}); ")).collect();
                    let _ = writeln!(pre, "    lumart_text a{i} = 0;\n    if (lumart_text_make(&a{i}, p{i}, p{i}_len, {src}, {line}) != 0) {{ {fail}{back} }}");
                    texts.push(format!("a{i}"));
                    call.push(format!("a{i}"));
                }
                _ => {
                    let what = ny!("ExportParam", "type" => self.ty_text(&t));
                    self.say(m, f.name.at, what);
                    return None;
                }
            }
        }
        let ps = if c_ps.is_empty() { "void".to_string() } else { c_ps.join(", ") };
        let head = format!("{ret_c} {}({ps})", f.name.name);
        let _ = writeln!(out, "{head} {{");
        // Script code runs on the game thread only (lumart's pools and tables have no locks): a call from a thread the
        // mod's C made is S0901 and gives zero (M-140).
        let _ = writeln!(out, "    if (!lumart_on_game_thread({src}, {line}, {})) {{ {back} }}", c_string(&f.name.name));
        if let Some((_, t)) = &r {
            let ct = self.c_type(t)?;
            let _ = writeln!(out, "    {ct} r = {zero};");
            call.push("&r".into());
        }
        out.push_str(&pre);
        let _ = writeln!(out, "    (void){internal}({});", call.join(", "));
        for t in &texts {
            let _ = writeln!(out, "    lm_rel_lumart_text(&{t});");
        }
        match &r {
            Some((Side::Bool, _)) => out.push_str("    return (_Bool)(r != 0);\n"),
            Some(_) => out.push_str("    return r;\n"),
            None => {}
        }
        out.push_str("}\n\n");
        self.exports.push(format!("{head};"));
        Some(out)
    }
}
