//! A mod's functions for other mods, and its calls of another mod's (docs/modding/language/compiled-form.md).
//!
//! Exported: every `pub fn` at a module's top level whose parameters and result cross (below), as
//! `int32_t lmx_<module>__<fn>(const LumaMod* caller, const char* file, uint32_t line, params..., result* out)` in
//! `LumaModInfo.exports`, named `<module>.<fn>` (a module path's `/` written `.`), with its `lumi1` signature (item 3).
//! A script error while it runs is reported for the caller at the call's line (lumart_caller), and the export
//! returns 2.
//!
//! A call of a function of a needed library mod goes through a kind 6 import (`LUMA_MOD_FUNCTION`, name
//! `<mod id>/<module>.<fn>`); a result other than 0 ends the caller's run as its own errors do (the report was made).
//!
//! What crosses (item 5): a bool, number, vector, colour, matrix, game enum, view of a game object or of the mod's
//! class's object, and an option of one of these, as the value; `text`, as the caller's text, which the export copies
//! into its own before the call (each mod's memory is its own); a struct of the library's own, as its size and its
//! address; an enum of the library's own, as its case number and the address of its case's fields. Every other value
//! made of these, text, bytes, handles, lists, maps, sets, options, tuples, the library's structs and enums and
//! function values crosses deep: as its size and its address, copied by the side that receives it into its own memory
//! with `lm_in_<n>` (a function value is shared: its count taken, its drop its maker's; a handle is made again in the
//! receiver's table from its object, read with the sender's lumart_ref_get). A result crosses as a plain value into
//! the caller's, or deep: the export copies it with the caller's own `lm_in_<n>`, which the caller passes. Tasks and
//! game struct copies do not cross: such a function is not exported, its interface line says so, and a caller's call
//! of it is S0800 at the call. Defaults a caller fills in are literals (a number, `true`, `false`, `none`, text, an
//! empty list or map, vectors and colours of literals, the library's enum cases of literals).

use super::{c_string, counted, lost, module_c, Ex, Fx, Gen};
use std::collections::HashSet;
use crate::ast::{self, Expr, ExprKind, ItemKind};
use crate::check::{ItemRef, Owner, Ty};
use crate::game::GameTy;
use std::fmt::Write as _;

/// How a parameter's or result's type goes across.
#[derive(Clone)]
pub(super) enum Cross {
    /// By value, as its C type.
    Plain(String),
    /// The caller's text, copied by the export.
    Text,
    /// A struct of the library's own: its size, then its address.
    Struct(String),
    /// An enum of the library's own: its case number, then its case's fields' address (null for a case with none).
    Enum(String, ItemRef),
    /// A value holding counted parts: its size, then its address; the receiver copies it with its own `lm_in_<n>`
    /// (C type, the receiver's copy function).
    Deep(String, String),
}

impl Gen<'_> {
    /// How `t` crosses, or None when it does not yet.
    pub(super) fn cross(&mut self, t: &Ty) -> Option<Cross> {
        match t {
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Value(_) | Ty::Class(_) => Some(Cross::Plain(self.c_type(t)?)),
            Ty::Game(GameTy::Enum(_)) | Ty::Game(GameTy::View { .. }) => Some(Cross::Plain(self.c_type(t)?)),
            Ty::Opt(x) => match self.cross(x)? {
                Cross::Plain(_) => Some(Cross::Plain(self.c_type(t)?)),
                _ => self.deep(t),
            },
            Ty::Text => Some(Cross::Text),
            // A library task's result to come (11.1a): its handle and the library's operations, read by the caller.
            Ty::Pending(_) => Some(Cross::Plain("lumart_pending".into())),
            Ty::Struct(_) => {
                let ct = self.c_type(t)?;
                if counted(&ct) || self.holds_ref(t, &mut HashSet::new()) {
                    return self.deep(t);
                }
                Some(Cross::Struct(ct))
            }
            Ty::Enum(item) => {
                let ct = self.c_type(t)?;
                if counted(&ct) || self.holds_ref(t, &mut HashSet::new()) {
                    return self.deep(t);
                }
                Some(Cross::Enum(ct, *item))
            }
            _ => self.deep(t),
        }
    }

    /// `t` crossing deep, when every part of it can be copied.
    fn deep(&mut self, t: &Ty) -> Option<Cross> {
        if !self.deep_ok(t, &mut HashSet::new()) {
            return None;
        }
        let ct = self.c_type(t)?;
        let f = self.in_fn(t)?;
        Some(Cross::Deep(ct, f))
    }

    /// Whether `t` holds a handle (`ref<T>`): a slot of the maker's own table, so it crosses as its object, made again
    /// in the receiver's table (`in_fn` with the maker's lumart_ref_get).
    pub(super) fn holds_ref(&mut self, t: &Ty, seen: &mut HashSet<String>) -> bool {
        match t {
            Ty::Ref(_) => true,
            Ty::List(x) | Ty::Set(x) | Ty::Opt(x) => self.holds_ref(x, seen),
            Ty::Map(k, v) => self.holds_ref(k, seen) || self.holds_ref(v, seen),
            Ty::Tuple(xs) => xs.iter().any(|x| self.holds_ref(x, seen)),
            Ty::Struct(item) | Ty::Enum(item) => {
                if !seen.insert(format!("{item:?}")) {
                    return false;
                }
                let parts = self.parts_of(t);
                parts.iter().any(|(_, pt)| self.holds_ref(pt, seen))
            }
            Ty::Service { .. } => {
                let parts = self.parts_of(t);
                parts.iter().any(|(_, pt)| self.holds_ref(pt, seen))
            }
            _ => false,
        }
    }

    /// Whether `in_fn` copies `t`: values holding nothing counted, text, bytes, handles, function values, and
    /// containers, options, tuples, structs and enums of such.
    fn deep_ok(&mut self, t: &Ty, seen: &mut HashSet<String>) -> bool {
        match t {
            Ty::Text | Ty::Json | Ty::Fn(..) | Ty::Bytes | Ty::Ref(_) => true,
            Ty::List(x) | Ty::Set(x) | Ty::Opt(x) => self.deep_ok(x, seen),
            Ty::Map(k, v) => self.deep_ok(k, seen) && self.deep_ok(v, seen),
            Ty::Tuple(xs) => xs.iter().all(|x| self.deep_ok(x, seen)),
            Ty::Struct(item) | Ty::Enum(item) => {
                // A struct holding a list of itself: what is being looked at already is taken as copied.
                if !seen.insert(format!("{item:?}")) {
                    return true;
                }
                let parts = self.parts_of(t);
                parts.iter().all(|(_, pt)| self.deep_ok(pt, seen))
            }
            Ty::Task | Ty::Pending(_) | Ty::Game(GameTy::Struct(_)) => false,
            Ty::Service { .. } => {
                let parts = self.parts_of(t);
                if parts.is_empty() {
                    return self.c_type(t).is_some_and(|ct| !counted(&ct));
                }
                parts.iter().all(|(_, pt)| self.deep_ok(pt, seen))
            }
            _ => self.c_type(t).is_some_and(|ct| !counted(&ct)),
        }
    }

    /// A struct's fields or an enum's cases' fields: (C member path from the value, type).
    fn parts_of(&self, t: &Ty) -> Vec<(String, Ty)> {
        match t {
            Ty::Struct(item) => {
                let ItemKind::Struct(s) = &self.item(*item).kind else { return Vec::new() };
                s.fields.iter().map(|f| (f.name.name.clone(), self.declared_ty(*item, &format!(".{}", f.name.name)))).collect()
            }
            Ty::Enum(item) => {
                let ItemKind::Enum(d) = &self.item(*item).kind else { return Vec::new() };
                let mut out = Vec::new();
                for c in &d.cases {
                    for f in &c.fields {
                        out.push((format!("u.{}.{}", c.name.name, f.name.name), self.declared_ty(*item, &format!("{}.{}", c.name.name, f.name.name))));
                    }
                }
                out
            }
            // A service's struct holding text or an actor (service.rs text_struct): its fields, an actor kept as a handle.
            Ty::Service { service, name } => match self.text_struct_fields(service, name) {
                Some(fields) => fields
                    .iter()
                    .map(|(f, st)| {
                        let ft = if *st == crate::svc::STy::Actor { Ty::Ref(Box::new(Ty::Unit)) } else { super::service::service_sty(service, st) };
                        (f.clone(), ft)
                    })
                    .collect(),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// `lm_in_<n>(dst, src, file, line, get)`: a value of type `t` another mod made, copied into zeroed `dst` in this
    /// mod's own memory (texts and containers made again, a function value shared, a handle made again in this mod's
    /// table from its object, which `get`, the maker's lumart_ref_get, gives); 0, or 1 after a report (no memory left;
    /// `dst` then holds what was copied, given back by its owner as any value).
    pub(super) fn in_fn(&mut self, t: &Ty) -> Option<String> {
        let key = format!("{t:?}");
        if let Some(n) = self.in_fns.get(&key) {
            return Some(n.clone());
        }
        let ct = self.c_type(t)?;
        let n = format!("lm_in_{}", self.in_fns.len());
        self.in_fns.insert(key, n.clone());
        let head = format!("static inline int32_t {n}(void* dv, const void* sv, const char* f, uint32_t l, const void* g)");
        self.typedefs.push(format!("{head};"));
        let mut b = format!("{head} {{\n    {ct}* d = ({ct}*)dv;\n    const {ct}* s = (const {ct}*)sv;\n    (void)f;\n    (void)l;\n    (void)g;\n");
        let part = |g: &mut Self, b: &mut String, path: &str, pt: &Ty| -> Option<()> {
            let pc = g.c_type(pt)?;
            if counted(&pc) || g.holds_ref(pt, &mut HashSet::new()) {
                let pf = g.in_fn(pt)?;
                let _ = writeln!(b, "        if ({pf}(&d->{path}, &s->{path}, f, l, g) != 0) return 1;");
            } else {
                let _ = writeln!(b, "        d->{path} = s->{path};");
            }
            Some(())
        };
        if !counted(&ct) && !self.holds_ref(t, &mut HashSet::new()) {
            b.push_str("    *d = *s;\n    return 0;\n");
        } else {
            match t {
                // The maker's handle: its object (none when it went), made again in this mod's table. The class slot
                // is declared here: lm.h writes these functions before its slots.
                Ty::Ref(_) => {
                    let nameobj = self.class_slot("NameObj");
                    let _ = write!(
                        b,
                        "    extern int32_t {nameobj};\n    void* v = (s->slot != 0 && g != 0) ? ((void* (*)(lumart_ref, int32_t))g)(*s, {nameobj}) : 0;\n    return v != 0 ? lumart_ref_make(d, v, {nameobj}, f, l) : 0;\n"
                    );
                }
                // A text row made before the game's messages exist stays a row (lumart_text_cross); JSON is copied.
                Ty::Text => b.push_str("    return lumart_text_cross(d, *s, f, l);\n"),
                Ty::Json => b.push_str("    return lumart_text_make(d, lumart_text_p(*s), lumart_text_n(*s), f, l);\n"),
                Ty::Fn(..) => b.push_str("    *d = *s;\n    lm_ret_lumart_fn(d);\n    return 0;\n"),
                // Bytes (a waited request's data, a library's): a block of this mod's own, the bytes copied.
                Ty::Bytes => b.push_str(
                    "    const uint64_t n = lumart_list_n(*s);\n    if (n == 0) return 0;\n    if (lumart_list_own(d, &lumart_u8, n, f, l) != 0) return 1;\n    __builtin_memcpy(lumart_bytes_p(*d), lumart_bytes_p(*s), n);\n    (*d)->len = n;\n    return 0;\n",
                ),
                Ty::List(x) => {
                    let xc = self.c_type(x)?;
                    let xd = self.desc(x)?;
                    let xf = self.in_fn(x)?;
                    let rel = if counted(&xc) { format!("        lm_rel_{xc}(&e);\n") } else { String::new() };
                    let _ = write!(
                        b,
                        "    for (uint64_t i = 0; i < lumart_list_n(*s); i++) {{\n        {xc} e;\n        __builtin_memset(&e, 0, sizeof e);\n        int32_t r = {xf}(&e, lumart_list_at(*s, i), f, l, g);\n        if (r == 0) r = lumart_list_push(d, {xd}, &e, f, l);\n{rel}        if (r != 0) return 1;\n    }}\n    return 0;\n"
                    );
                }
                Ty::Map(k, v) | Ty::Set(k @ v) => {
                    let set = matches!(t, Ty::Set(_));
                    let (kc, kd, kf) = (self.c_type(k)?, self.desc(k)?, self.in_fn(k)?);
                    let (vc, vd, vf) = if set { (String::new(), "0".to_string(), String::new()) } else { (self.c_type(v)?, self.desc(v)?, self.in_fn(v)?) };
                    let _ = write!(b, "    for (uint64_t i = 0; i < lumart_map_n(*s); i++) {{\n        {kc} k;\n        __builtin_memset(&k, 0, sizeof k);\n        int32_t r = {kf}(&k, lumart_map_key(*s, i), f, l, g);\n");
                    if set {
                        b.push_str("        uint8_t added = 0;\n        if (r == 0) r = lumart_map_put(d, ");
                        let _ = write!(b, "{kd}, 0, &k, 0, &added, f, l);\n");
                    } else {
                        let _ = write!(b, "        {vc} v;\n        __builtin_memset(&v, 0, sizeof v);\n        if (r == 0) r = {vf}(&v, lumart_map_val(*s, i), f, l, g);\n        uint8_t added = 0;\n        if (r == 0) r = lumart_map_put(d, {kd}, {vd}, &k, &v, &added, f, l);\n");
                        if counted(&vc) {
                            let _ = writeln!(b, "        lm_rel_{vc}(&v);");
                        }
                    }
                    if counted(&kc) {
                        let _ = writeln!(b, "        lm_rel_{kc}(&k);");
                    }
                    b.push_str("        (void)added;\n        if (r != 0) return 1;\n    }\n    return 0;\n");
                }
                Ty::Opt(x) => {
                    let xf = self.in_fn(x)?;
                    let _ = write!(b, "    d->some = s->some;\n    if (!s->some) return 0;\n    return {xf}(&d->value, &s->value, f, l, g);\n");
                }
                Ty::Tuple(xs) => {
                    b.push_str("    {\n");
                    for (i, x) in xs.clone().iter().enumerate() {
                        part(self, &mut b, &format!("_{i}"), x)?;
                    }
                    b.push_str("    }\n    return 0;\n");
                }
                Ty::Struct(_) | Ty::Service { .. } => {
                    b.push_str("    {\n");
                    for (path, pt) in self.parts_of(t) {
                        part(self, &mut b, &path, &pt)?;
                    }
                    b.push_str("    }\n    return 0;\n");
                }
                Ty::Enum(item) => {
                    let ItemKind::Enum(dcl) = &self.item(*item).kind else { return None };
                    let cases: Vec<(String, Vec<String>)> = dcl.cases.iter().map(|c| (c.name.name.clone(), c.fields.iter().map(|f| f.name.name.clone()).collect())).collect();
                    let item = *item;
                    b.push_str("    d->tag = s->tag;\n");
                    for (k, (cn, fields)) in cases.iter().enumerate() {
                        if fields.is_empty() {
                            continue;
                        }
                        let _ = writeln!(b, "    if (s->tag == {k}u) {{");
                        for fname in fields {
                            let pt = self.declared_ty(item, &format!("{cn}.{fname}"));
                            part(self, &mut b, &format!("u.{cn}.{fname}"), &pt)?;
                        }
                        b.push_str("    }\n");
                    }
                    b.push_str("    return 0;\n");
                }
                _ => return None,
            }
        }
        b.push('}');
        self.typedefs.push(b);
        Some(n)
    }

    /// A result's C type when it crosses: written by the export into the caller's value through `out`, so any value
    /// that holds no text or container does (an option of the library's enum, a tuple of numbers); text does not yet.
    pub(super) fn cross_result(&mut self, t: &Ty) -> Option<String> {
        match self.cross(t) {
            Some(Cross::Plain(ct) | Cross::Struct(ct) | Cross::Enum(ct, _)) => Some(ct),
            // A value holding nothing counted is written into the caller's as it is.
            Some(Cross::Deep(ct, _)) if !counted(&ct) && !self.holds_ref(t, &mut HashSet::new()) => Some(ct),
            Some(Cross::Text | Cross::Deep(..)) => None,
            None => match t {
                Ty::Opt(_) | Ty::Tuple(_) if !self.holds_ref(t, &mut HashSet::new()) => self.c_type(t).filter(|ct| !counted(ct)),
                _ => None,
            },
        }
    }

    /// A result holding counted parts: (its C type, the caller's copy function), copied by the export into the
    /// caller's memory with the caller's function.
    fn deep_result(&mut self, t: &Ty) -> Option<(String, String)> {
        match t {
            Ty::Text => Some(("lumart_text".into(), self.in_fn(t)?)),
            _ => match self.deep(t)? {
                Cross::Deep(ct, f) => Some((ct, f)),
                _ => None,
            },
        }
    }

    /// The C parameters a crossing value takes, named from `p`.
    fn cross_params(c: &Cross, p: &str) -> String {
        match c {
            Cross::Plain(ct) => format!("{ct} {p}"),
            Cross::Text => format!("lumart_text {p}"),
            Cross::Struct(ct) => format!("uint32_t {p}_n, const {ct}* {p}"),
            Cross::Enum(..) => format!("uint32_t {p}_case, const void* {p}"),
            Cross::Deep(..) => format!("uint32_t {p}_n, const void* {p}"),
        }
    }

    /// A type's full LumaScript name for a signature's text (item 3): a mod's own type is `<mod id>.<module>.<Name>`.
    pub(super) fn canon(&self, t: &Ty) -> String {
        match t {
            Ty::List(x) => format!("list<{}>", self.canon(x)),
            Ty::Pending(x) => format!("pending<{}>", self.canon(x)),
            Ty::Map(k, v) => format!("map<{},{}>", self.canon(k), self.canon(v)),
            Ty::Set(x) => format!("set<{}>", self.canon(x)),
            Ty::Opt(x) => format!("{}?", self.canon(x)),
            Ty::Ref(x) => format!("ref<{}>", self.canon(x)),
            Ty::Tuple(xs) => format!("({})", xs.iter().map(|x| self.canon(x)).collect::<Vec<_>>().join(",")),
            Ty::Fn(ps, r) => format!("fn({})->{}", ps.iter().map(|x| self.canon(x)).collect::<Vec<_>>().join(","), self.canon(r)),
            Ty::Unit => "()".into(),
            Ty::Struct(i) | Ty::Enum(i) | Ty::Class(i) => {
                let (id, path) = match i.owner {
                    Owner::Mod => (self.mod_id.as_str(), self.sources[i.module].path.as_str()),
                    Owner::Library(l) => (self.libraries[l].id.as_str(), self.libraries[l].modules[i.module].path.as_str()),
                };
                format!("{id}.{}.{}", path.replace('/', "."), self.ty_text(t))
            }
            _ => self.ty_text(t),
        }
    }

    /// A function's export name (`<module>.<fn>`) and the canonical text its signature hashes.
    fn export_text(&self, path: &str, fname: &str, params: &[Ty], result: &Ty) -> (String, String) {
        let name = format!("{}.{fname}", path.replace('/', "."));
        let text = format!("{name}({})->{}", params.iter().map(|t| self.canon(t)).collect::<Vec<_>>().join(","), self.canon(result));
        (name, text)
    }

    /// The parameter types (`var` refused) and result of function `f` of `item`, as declared; `key` is `.<fn>` for a
    /// struct's function (its `self` the struct, by value), empty for a module's.
    fn fn_types(&self, item: ItemRef, key: &str, f: &ast::FnDecl) -> Option<(Vec<Ty>, Ty)> {
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            if p.var {
                return None;
            }
            if p.is_self {
                params.push(Ty::Struct(item));
                continue;
            }
            let pname = p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone());
            params.push(self.declared_ty(item, &format!("{key}({pname})")));
        }
        let result = self.checked.types.get(&(item, format!("{key}->"))).cloned().unwrap_or(Ty::Unit);
        Some((params, result))
    }

    /// The name a function goes by across mods: `f`, or `S.f` for struct S's.
    fn cross_name(&self, item: ItemRef, key: &str, f: &ast::FnDecl) -> String {
        match &self.item(item).kind {
            ItemKind::Struct(st) if !key.is_empty() => format!("{}.{}", st.name.name, f.name.name),
            _ => f.name.name.clone(),
        }
    }

    /// Module `m`'s `pub fn f` exported, when its types cross: its wrapper written and its row kept. Its signature
    /// goes to the interface file; a function that does not cross gets none there.
    /// `task`: a `pub task f(...) -> T` (11.1a), whose export starts it and gives the caller its pending.
    /// `key`: `.<fn>` for a pub struct's function, exported as `<module>.<Struct>.<fn>` with `self` first.
    pub(super) fn export(&mut self, m: usize, item: ItemRef, key: &str, f: &ast::FnDecl, c_name: &str, task: bool) {
        let Some((params, result)) = self.fn_types(item, key, f) else { return };
        let result = if task { Ty::Pending(Box::new(result)) } else { result };
        let fname = self.cross_name(item, key, f);
        let pnames: Vec<String> = f.params.iter().enumerate().map(|(i, p)| p.name.as_ref().map_or_else(|| i.to_string(), |n| n.name.clone())).collect();
        self.export_as(m, &fname, &params, &pnames, &result, c_name, task);
    }

    /// A struct field's default the library makes itself ([`made_by_library`]), exported as
    /// `<module>.<S>.<field>.default` (no parameters, the default its result): `default_c` is its C function.
    pub(super) fn export_default(&mut self, m: usize, st: &str, field: &str, t: &Ty, default_c: &str) {
        self.export_as(m, &format!("{st}.{field}.default"), &[], &[], t, default_c, false);
    }

    /// The export of C function `c_name` as `<module>.<fname>` with these parameter types (named `pnames` in reports)
    /// and result.
    #[allow(clippy::too_many_arguments)]
    fn export_as(&mut self, m: usize, fname: &str, params: &[Ty], pnames: &[String], result: &Ty, c_name: &str, task: bool) {
        let mut crossing = Vec::new();
        for t in params {
            match self.cross(t) {
                Some(c) => crossing.push(c),
                None => return,
            }
        }
        let mut deep_out = None;
        let out = match result {
            Ty::Unit => None,
            t => match self.cross_result(t) {
                Some(ct) => Some(ct),
                None => match self.deep_result(t) {
                    Some((ct, _)) => {
                        deep_out = Some(ct);
                        None
                    }
                    None => return,
                },
            },
        };
        let path = self.sources[m].path.clone();
        let (name, text) = self.export_text(&path, fname, params, result);
        let sig = crate::lumi::signature(&text);
        let wrapper = format!("lmx_{}__{}", module_c(&path), fname.replace('.', "__"));
        // The caller, its call's place, and its lumart_ref_get (a handle it passes is made again here from its object).
        let mut head = vec!["const LumaMod* lm_caller".to_string(), "const char* lm_file".into(), "uint32_t lm_line".into(), "const void* lm_peer".into()];
        for (i, c) in crossing.iter().enumerate() {
            head.push(Self::cross_params(c, &format!("p{i}")));
        }
        if let Some(ct) = &out {
            head.push(format!("{ct}* lm_out"));
        }
        if deep_out.is_some() {
            head.push("void* lm_out".into());
            head.push("int32_t (*lm_in)(void*, const void*, const char*, uint32_t, const void*)".into());
        }
        let mut body = String::new();
        let mut call = Vec::new();
        let mut after = String::new();
        let _ = writeln!(body, "    lumart_as lm_was = lumart_caller;\n    int32_t lm_r = 1;");
        for (i, c) in crossing.iter().enumerate() {
            match c {
                Cross::Plain(_) => call.push(format!("p{i}")),
                Cross::Text => {
                    let _ = writeln!(body, "    lumart_text a{i} = 0;");
                    let _ = writeln!(after, "    lm_rel_lumart_text(&a{i});");
                    call.push(format!("a{i}"));
                }
                Cross::Struct(ct) | Cross::Enum(ct, _) => {
                    let _ = writeln!(body, "    {ct} a{i};\n    __builtin_memset(&a{i}, 0, sizeof a{i});");
                    call.push(format!("a{i}"));
                }
                Cross::Deep(ct, _) => {
                    let _ = writeln!(body, "    {ct} a{i};\n    __builtin_memset(&a{i}, 0, sizeof a{i});");
                    // An option or tuple of nothing counted (`draw.Texture?`) has no release.
                    if counted(ct) {
                        let _ = writeln!(after, "    lm_rel_{ct}(&a{i});");
                    }
                    call.push(format!("a{i}"));
                }
            }
        }
        if let Some(ct) = &deep_out {
            let _ = writeln!(body, "    {ct} lm_res;\n    __builtin_memset(&lm_res, 0, sizeof lm_res);");
            if counted(ct) {
                let _ = writeln!(after, "    lm_rel_{ct}(&lm_res);");
            }
        }
        let _ = writeln!(body, "    lumart_caller.mod = lm_caller;\n    lumart_caller.file = lm_file;\n    lumart_caller.line = lm_line;");
        let fn_name = format!("{}.{name}", self.mod_id);
        for (i, c) in crossing.iter().enumerate() {
            match c {
                Cross::Plain(_) => {}
                Cross::Text => {
                    let _ = writeln!(body, "    if (lumart_text_cross(&a{i}, p{i}, lm_file, lm_line) != 0) goto lm_done;");
                }
                // Fields only ever added at the end (item 5): what the caller has, the rest left zero.
                Cross::Struct(_) => {
                    let _ = writeln!(body, "    __builtin_memcpy(&a{i}, p{i}, p{i}_n < sizeof a{i} ? p{i}_n : sizeof a{i});");
                }
                // The caller's value copied into this mod's memory; a type of another layout (another version of a
                // struct) is refused.
                Cross::Deep(_, inf) => {
                    let pname = pnames[i].clone();
                    let _ = writeln!(
                        body,
                        "    if (p{i}_n != (uint32_t)sizeof a{i}) {{\n        (void)lumart_wrong_call(lm_file, lm_line, {}, {}, \"a value of another version of its type\", \"the version this mod was built with\");\n        goto lm_done;\n    }}\n    if ({inf}(&a{i}, p{i}, lm_file, lm_line, lm_peer) != 0) goto lm_done;",
                        c_string(&fn_name),
                        c_string(&pname)
                    );
                }
                Cross::Enum(_, e) => {
                    let ItemKind::Enum(decl) = &self.item(*e).kind else { return };
                    let n = decl.cases.len();
                    let pname = pnames[i].clone();
                    let ename = decl.name.name.clone();
                    let _ = writeln!(
                        body,
                        "    if (p{i}_case >= {n}u) {{\n        (void)lumart_wrong_call(lm_file, lm_line, {}, {}, \"a case this version does not have\", {});\n        goto lm_done;\n    }}\n    a{i}.tag = p{i}_case;",
                        c_string(&fn_name),
                        c_string(&pname),
                        c_string(&format!("a case of {ename}"))
                    );
                    for (k, case) in decl.cases.iter().enumerate() {
                        if !case.fields.is_empty() {
                            let cn = &case.name.name;
                            let _ = writeln!(body, "    if (p{i}_case == {k}u) __builtin_memcpy(&a{i}.u.{cn}, p{i}, sizeof a{i}.u.{cn});");
                        }
                    }
                }
            }
        }
        if task {
            body.push_str("    lumart_task lm_t = {0, 0};\n");
            call.push("&lm_t".into());
        } else if out.is_some() {
            call.push("lm_out".into());
        }
        if deep_out.is_some() {
            call.push("&lm_res".into());
        }
        let _ = writeln!(body, "    lm_r = {c_name}({});", call.join(", "));
        // A task started: the pending is its handle with this mod's operations.
        if task {
            // Without a result: its end by itself is kept as `()` for the caller's pending.
            let watch = if *result == Ty::Pending(Box::new(Ty::Unit)) { "        lumart_task_watch(lm_t);\n" } else { "" };
            let _ = writeln!(body, "    if (lm_r == 0) {{\n{watch}        lm_out->t = lm_t;\n        lm_out->ops = &lumart_pending_own;\n    }}");
        }
        // A deep result: copied into the caller's memory by the caller's own copy function.
        if deep_out.is_some() {
            body.push_str("    if (lm_r == 0) lm_r = lm_in(lm_out, &lm_res, lm_file, lm_line, (const void*)lumart_ref_get);\n");
        }
        body.push_str("    goto lm_done;\nlm_done:\n");
        body.push_str(&after);
        body.push_str("    lumart_caller = lm_was;\n    return lm_r != 0 ? 2 : 0;\n");
        let head = format!("int32_t {wrapper}({})", head.join(", "));
        let _ = write!(self.export_code, "{head} {{\n{body}}}\n\n");
        self.protos.push(format!("{head};"));
        self.lib_exports.push((name, sig.clone(), wrapper));
        self.lumi_sigs.insert((m, fname.to_string()), sig);
    }

    /// A default a caller fills in for a library's parameter or field, written from its declared type (the library's
    /// expressions have no types in this mod): none, true, false, a number (or its negation), a text, an empty list,
    /// map or set, `vec2`..`vec4` and `color` of numbers, and a case of the library's enum (its fields such defaults).
    pub(super) fn library_default(&mut self, fx: &mut Fx, e: &Expr, d: &Expr, t: &Ty) -> Ex {
        let ct = self.c_type(t).ok_or_else(lost)?;
        match (&d.kind, t) {
            (ExprKind::Str(lit), Ty::Text) if !lit.interpolates() => return self.text_value(fx, d, t),
            (ExprKind::List(items), Ty::List(_)) if items.is_empty() => return Ok("((lumart_list)0)".into()),
            (ExprKind::Braces(items), Ty::Map(..) | Ty::Set(_)) if items.is_empty() => return Ok("((lumart_map)0)".into()),
            (ExprKind::Neg(inner), Ty::Int(_) | Ty::Float(_)) if matches!(inner.kind, ExprKind::Number(_)) => {
                let v = self.library_default(fx, e, inner, t)?;
                return Ok(format!("(({ct})-{v})"));
            }
            (ExprKind::Call { callee, args }, Ty::Value(v)) if matches!(&callee.kind, ExprKind::Name { .. }) => {
                let width = match v {
                    crate::game::Value::Vec2 => 2,
                    crate::game::Value::Vec3 => 3,
                    crate::game::Value::Vec4 | crate::game::Value::Color => 4,
                    _ => 0,
                };
                if width != 0 && args.len() == width && args.iter().all(|a| a.name.is_none()) {
                    let mut parts = Vec::new();
                    for a in args {
                        parts.push(self.library_default(fx, e, &a.value, &Ty::Float(crate::game::Scalar::F32))?);
                    }
                    return Ok(format!("(({ct}){{{}}})", parts.join(", ")));
                }
            }
            (ExprKind::Member { name, .. } | ExprKind::Name { name, .. }, Ty::Enum(item)) => {
                if let Some(tag) = self.case_tag(*item, &name.name) {
                    let ItemKind::Enum(decl) = &self.item(*item).kind else { return Err(lost()) };
                    if decl.cases[tag].fields.iter().all(|f| f.default.is_some()) {
                        return self.library_case(fx, e, *item, tag, &[]);
                    }
                }
            }
            (ExprKind::Call { callee, args }, Ty::Enum(item)) => {
                if let ExprKind::Member { name, .. } | ExprKind::Name { name, .. } = &callee.kind {
                    if let Some(tag) = self.case_tag(*item, &name.name) {
                        return self.library_case(fx, e, *item, tag, args);
                    }
                }
            }
            _ => {}
        }
        match &d.kind {
            ExprKind::None if matches!(t, Ty::Opt(x) if super::is_view(x)) => Ok("0".into()),
            ExprKind::None => Ok(format!("({ct}){{0}}")),
            ExprKind::True => Ok("1".into()),
            ExprKind::False => Ok("0".into()),
            ExprKind::Number(n) if matches!(t, Ty::Int(_) | Ty::Float(_)) => {
                let n: String = n.chars().filter(|c| *c != '_').collect();
                let n = if let Some(hex) = n.strip_prefix("0x").or_else(|| n.strip_prefix("0X")) {
                    format!("0x{}", hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect::<String>())
                } else {
                    n.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')).collect()
                };
                Ok(format!("(({ct})({n}))"))
            }
            _ => self.not_yet(fx, e, "a call leaving out a parameter of another mod whose default is not a literal"),
        }
    }

    /// A case of a library's enum written in a default: its tag, and each field from the arguments (positional, then
    /// by name) or its own default, each one a default as `library_default` writes them.
    fn library_case(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, tag: usize, args: &[ast::Arg]) -> Ex {
        let ct = self.c_type(&Ty::Enum(item)).ok_or_else(lost)?;
        let ItemKind::Enum(decl) = &self.item(item).kind else { return Err(lost()) };
        let case = decl.cases[tag].name.name.clone();
        let fields = decl.cases[tag].fields.clone();
        let mut given: Vec<Option<Expr>> = vec![None; fields.len()];
        let mut next = 0;
        for a in args {
            let k = match &a.name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(n) => fields.iter().position(|f| f.name.name == n.name).ok_or_else(lost)?,
            };
            *given.get_mut(k).ok_or_else(lost)? = Some(a.value.clone());
        }
        let r = fx.fresh(&ct);
        if super::counted(&ct) {
            fx.emit(format!("lm_rel_{ct}(&{r});"));
        }
        fx.emit(format!("{r}.tag = {tag}u;"));
        for (f, g) in fields.iter().zip(given) {
            let Some(x) = g.or_else(|| f.default.clone()) else { return self.not_yet(fx, e, "a value missing its field") };
            let ft = self.declared_ty(item, &format!("{case}.{}", f.name.name));
            let v = self.library_default(fx, e, &x, &ft)?;
            let fc = self.field_c(item, &format!("{case}.{}", f.name.name))?;
            fx.set(&format!("{r}.u.{case}.{}", f.name.name), &v, &fc);
        }
        Ok(r)
    }

    /// A call of a library's function through its kind 6 import.
    /// `task`: a library's `pub task f(...) -> T`, whose call gives `pending<T>` (11.1a).
    /// `key` and `recv`: a library struct's function (`.<fn>`) and the value it is called on (its `self`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_library(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, key: &str, recv: Option<&Expr>, f: &ast::FnDecl, args: &[ast::Arg], task: bool) -> Ex {
        let Some((ptys, result)) = self.fn_types(item, key, f) else { return self.not_yet(fx, e, "a call of another mod's function with a `var` parameter") };
        let result = if task { Ty::Pending(Box::new(result)) } else { result };
        let fname = self.cross_name(item, key, f);
        self.call_library_as(fx, e, item, &fname, &ptys, &result, &f.params, recv, args)
    }

    /// A library struct field's default the library makes ([`made_by_library`]): its `<S>.<field>.default` export
    /// called, the value its result.
    pub(super) fn library_made_default(&mut self, fx: &mut Fx, e: &Expr, owner: ItemRef, field: &str, t: &Ty) -> Ex {
        let ItemKind::Struct(st) = &self.item(owner).kind else { return Err(lost()) };
        let fname = format!("{}.{field}.default", st.name.name);
        self.call_library_as(fx, e, owner, &fname, &[], t, &[], None, &[])
    }

    /// A call through the kind 6 import of library function `<module of item>.<fname>`, of these parameter types,
    /// declarations (their defaults) and result.
    #[allow(clippy::too_many_arguments)]
    fn call_library_as(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, fname: &str, ptys: &[Ty], result: &Ty, decl_params: &[ast::Param], recv: Option<&Expr>, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        let Owner::Library(l) = item.owner else { return Err(lost()) };
        let mut crossing = Vec::new();
        for t in ptys {
            match self.cross(t) {
                Some(c) => crossing.push(c),
                None => {
                    let what = format!("a call of another mod's function with a parameter of type {}", self.ty_text(t));
                    return self.not_yet(fx, e, &what);
                }
            }
        }
        let mut deep_r = None;
        let rct = match result {
            Ty::Unit => None,
            t => match self.cross_result(t) {
                Some(ct) => Some(ct),
                None => match self.deep_result(t) {
                    Some(d) => {
                        deep_r = Some(d);
                        None
                    }
                    None => {
                        let what = format!("a call of another mod's function giving {}", self.ty_text(t));
                        return self.not_yet(fx, e, &what);
                    }
                },
            },
        };
        let path = self.libraries[l].modules[item.module].path.clone();
        let (name, text) = self.export_text(&path, fname, ptys, result);
        let sig = crate::lumi::signature(&text);
        let full = format!("{}/{name}", self.libraries[l].id);
        let n = self.mod_fns.len();
        let slot = self.mod_fns.entry((full, sig)).or_insert_with(|| format!("lm_mf_{n}")).clone();
        // The arguments: positional in order, then by name, then each missing one's default.
        let params: Vec<&ast::Param> = decl_params.iter().collect();
        let mut given: Vec<Option<&Expr>> = vec![None; params.len()];
        // A struct's function: `self` is the value it is called on, the arguments the parameters after it.
        let mut next = 0;
        if params.first().is_some_and(|p| p.is_self) {
            given[0] = Some(recv.ok_or_else(lost)?);
            next = 1;
        }
        for a in args {
            let k = match &a.name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(nm) => params.iter().position(|p| p.name.as_ref().is_some_and(|pn| pn.name == nm.name)).ok_or_else(lost)?,
            };
            *given.get_mut(k).ok_or_else(lost)? = Some(&a.value);
        }
        // The caller and its place: this mod's own, or, inside an export of this mod, the first caller's and its
        // call's place passed through, so a library a library calls reports at the mod's line too (lumakit rule 8).
        let mut list = vec![
            "(lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self)".to_string(),
            format!("(lumart_caller.mod != 0 ? lumart_caller.file : {})", c_string(&fx.file)),
            format!("(lumart_caller.mod != 0 ? lumart_caller.line : {}u)", e.at.line),
        ];
        list.push("(const void*)lumart_ref_get".to_string());
        let mut types = vec!["const LumaMod*".to_string(), "const char*".into(), "uint32_t".into(), "const void*".into()];
        let mut held = Vec::new();
        for (k, p) in params.iter().enumerate() {
            let pt = ptys[k].clone();
            let ct = self.c_type(&pt).ok_or_else(lost)?;
            let v = match given[k] {
                Some(x) => {
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    self.coerce(fx, &v, &from, &pt, x.at)?
                }
                None => match &p.default {
                    Some(d) => self.library_default(fx, e, d, &pt)?,
                    None => return self.not_yet(fx, e, "a call missing an argument"),
                },
            };
            let t = fx.temp(&ct, &v);
            if counted(&ct) {
                held.push((t.clone(), ct.clone()));
            }
            match &crossing[k] {
                Cross::Plain(c) => {
                    types.push(c.clone());
                    list.push(t);
                }
                Cross::Text => {
                    types.push("lumart_text".into());
                    list.push(t);
                }
                Cross::Struct(c) => {
                    types.push("uint32_t".into());
                    types.push(format!("const {c}*"));
                    list.push(format!("(uint32_t)sizeof {t}"));
                    list.push(format!("&{t}"));
                }
                Cross::Deep(..) => {
                    types.push("uint32_t".into());
                    types.push("const void*".into());
                    list.push(format!("(uint32_t)sizeof {t}"));
                    list.push(format!("(const void*)&{t}"));
                }
                Cross::Enum(_, en) => {
                    let ItemKind::Enum(decl) = &self.item(*en).kind else { return Err(lost()) };
                    let has_fields = decl.cases.iter().any(|c| !c.fields.is_empty());
                    types.push("uint32_t".into());
                    types.push("const void*".into());
                    list.push(format!("{t}.tag"));
                    list.push(if has_fields { format!("(const void*)&{t}.u") } else { "(const void*)0".into() });
                }
            }
        }
        let r = match (&rct, &deep_r) {
            (Some(ct), _) => {
                let r = fx.fresh(ct);
                types.push(format!("{ct}*"));
                list.push(format!("&{r}"));
                Some(r)
            }
            // Copied into this mod's memory by this mod's own copy function, which the export calls.
            (None, Some((ct, inf))) => {
                let r = fx.fresh(ct);
                types.push("void*".into());
                types.push("int32_t (*)(void*, const void*, const char*, uint32_t, const void*)".into());
                list.push(format!("(void*)&{r}"));
                list.push(inf.clone());
                Some(r)
            }
            (None, None) => None,
        };
        fx.fail_if(&format!("((int32_t (*)({}))(const void*){slot})({}) != 0", types.join(", "), list.join(", ")));
        for (t, ct) in held {
            fx.emit(format!("lm_rel_{ct}(&{t});"));
        }
        Ok(r.unwrap_or_default())
    }
}

/// A struct field's default its library makes and gives through an export (`<S>.<field>.default`) rather than a caller
/// writing it: a function value (it stays the library's own) or a list with elements. The same test on both sides,
/// from the declared type.
pub(super) fn made_by_library(d: &Expr, t: &Ty) -> bool {
    match (&d.kind, t) {
        (ExprKind::Paren(x), _) => made_by_library(x, t),
        (ExprKind::None, _) => false,
        (_, Ty::Fn(..)) => true,
        (_, Ty::Opt(x)) if matches!(**x, Ty::Fn(..)) => true,
        (ExprKind::List(items), Ty::List(_)) => !items.is_empty(),
        _ => false,
    }
}
