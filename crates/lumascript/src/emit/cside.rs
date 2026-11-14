//! C in the same mod (docs/modding/language/language.md, section 13, docs/modding/language/compiled-form.md, section 9): `use c "x.h"` includes the mod's own header in `lm.h` (so a
//! declaration that disagrees with the script's is clang's error); `extern c fn` is a prototype and a direct call;
//! `export c fn` is the script's function plus a C function of that name, declared in `lum/lm_exports.h` for the
//! mod's C. Across: integers, floats, `bool` (C `_Bool`), `text` and `bytes` as (pointer, length) for the call's
//! duration (an extern's parameters; an export's text parameters, copied into a text of the script's own). A fault in
//! the C is the guard's, not a script error.

use super::{c_string, scalar_c, Ex, Fx, Gen};
use crate::ast::{self, Expr, Ident, Param};
use crate::check::{ItemRef, Ty};
use std::fmt::Write as _;

/// How a value of a type crosses to C.
enum Side {
    /// A number: its C type.
    Scalar(&'static str),
    Bool,
    Text,
    Bytes,
}

fn side(t: &Ty) -> Option<Side> {
    Some(match t {
        Ty::Int(s) | Ty::Float(s) => Side::Scalar(scalar_c(*s)),
        Ty::Bool => Side::Bool,
        Ty::Text => Side::Text,
        Ty::Bytes => Side::Bytes,
        _ => return None,
    })
}

/// A parameter as C declares it: one or two C parameters.
fn c_param(s: &Side, name: &str) -> String {
    match s {
        Side::Scalar(ct) => format!("{ct} {name}"),
        Side::Bool => format!("_Bool {name}"),
        Side::Text => format!("const char* {name}, uint64_t {name}_len"),
        Side::Bytes => format!("const uint8_t* {name}, uint64_t {name}_len"),
    }
}

impl<'a> Gen<'a> {
    fn param_name(p: &Param, i: usize) -> String {
        p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone())
    }

    /// The C result type of an extern or export (None: it gives none); Err when it does not cross.
    fn c_result(&self, item: ItemRef, has: bool) -> Result<Option<(Side, Ty)>, String> {
        if !has {
            return Ok(None);
        }
        let t = self.declared_ty(item, "->");
        match side(&t) {
            Some(Side::Text) | Some(Side::Bytes) => Err(format!("a C result of type {}", self.ty_text(&t))),
            Some(s) => Ok(Some((s, t))),
            None if t == Ty::Unit => Ok(None),
            None => Err(format!("a C result of type {}", self.ty_text(&t))),
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
                self.say(m, name.at, "an extern C function's var parameter");
                return;
            }
            let pn = Self::param_name(p, i);
            let t = self.declared_ty(me, &format!("({pn})"));
            let Some(s) = side(&t) else {
                let what = format!("a C parameter of type {}", self.ty_text(&t));
                self.say(m, name.at, &what);
                return;
            };
            ps.push(c_param(&s, &format!("p{i}")));
        }
        let r = match self.c_result(me, ret) {
            Ok(Some((Side::Scalar(ct), _))) => ct.to_string(),
            Ok(Some((Side::Bool, _))) => "_Bool".into(),
            Ok(_) => "void".into(),
            Err(what) => {
                self.say(m, name.at, &what);
                return;
            }
        };
        let ps = if ps.is_empty() { "void".to_string() } else { ps.join(", ") };
        self.c_protos.insert(format!("{r} {}({ps});", name.name));
    }

    /// A call of an `extern c fn`: the arguments in order (text and bytes as pointer and length, valid for the call),
    /// the C function called directly.
    pub(super) fn call_extern(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, name: &str, params: &[Param], ret: bool, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        if args.len() != params.len() || args.iter().any(|a| a.name.is_some()) {
            return self.not_yet(fx, e, "a C call with named or missing arguments");
        }
        let mut list = Vec::new();
        for (i, (a, p)) in args.iter().zip(params).enumerate() {
            let pt = self.declared_ty(item, &format!("({})", Self::param_name(p, i)));
            let v = self.expr(fx, &a.value)?;
            let from = self.ty(m, &a.value);
            let v = self.coerce(fx, &v, &from, &pt, a.value.at)?;
            let Some(s) = side(&pt) else { return self.not_yet(fx, e, "a C argument of this type") };
            list.push(match s {
                Side::Scalar(ct) => fx.temp(ct, &v),
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
            Ok(Some((Side::Scalar(ct), _))) => Ok(fx.temp(ct, &call)),
            Ok(Some((Side::Bool, _))) => Ok(fx.temp("uint8_t", &format!("(uint8_t)({call} != 0)"))),
            Ok(_) => {
                fx.emit(format!("{call};"));
                Ok(String::new())
            }
            Err(what) => self.not_yet(fx, e, &what),
        }
    }

    /// `export c fn name(params) [-> type] { body }`: the body as the script's function `lm_<m>__name`, and the C
    /// function `name` the mod's C calls: it copies text arguments into texts of the script's own, calls the body,
    /// and gives its result (zero when the body ended in a script error, which lumart has reported).
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
                self.say(m, f.name.at, &what);
                return None;
            }
        };
        let (ret_c, zero) = match &r {
            Some((Side::Scalar(ct), _)) => (ct.to_string(), "0"),
            Some((Side::Bool, _)) => ("bool".to_string(), "0"),
            _ => ("void".to_string(), ""),
        };
        let back = if zero.is_empty() { "return;".to_string() } else { format!("return {zero};") };
        for (i, p) in f.params.iter().enumerate() {
            let pn = Self::param_name(p, i);
            let t = self.declared_ty(me, &format!("({pn})"));
            if p.var {
                self.say(m, f.name.at, "an exported C function's var parameter");
                return None;
            }
            match side(&t) {
                Some(Side::Scalar(ct)) => {
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
                    let what = format!("an exported C function's parameter of type {}", self.ty_text(&t));
                    self.say(m, f.name.at, &what);
                    return None;
                }
            }
        }
        let ps = if c_ps.is_empty() { "void".to_string() } else { c_ps.join(", ") };
        let head = format!("{ret_c} {}({ps})", f.name.name);
        let _ = writeln!(out, "{head} {{");
        if let Some((_, t)) = &r {
            let ct = self.c_type(t)?;
            let _ = writeln!(out, "    {ct} r = 0;");
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
