//! A mod's class derived from a game class (docs/modding/language/language.md, section 5.5, docs/modding/language/classes.md): `define_class` at the mod's
//! start with its overrides; its own fields in data the kernel keeps beside each object (`class_data`), first values
//! written by `made` when the factory makes one; each `override fn` a wrap the kernel runs for the class's own objects
//! only, `base.<name>(...)` the base's own version (the kernel's proceed).
//!
//! A field may hold text or a container: `gone` gives its count back before the kernel frees the data.
//!
//! Not written yet (S0800, each named): a first value that needs code to run (`made` may run off the game thread), a
//! class's own `fn` and its hooks.

use super::{assign_line, c_string, counted, Body, Ex, Fx, Gen, HookFx};
use crate::ast::{self, ClassMember, Expr, ExprKind, HookKind, ItemKind};
use crate::check::{ItemRef, Resolved};
use crate::check::Ty;
use crate::game::{GameTy, Scalar, Value};
use std::fmt::Write as _;

impl Gen<'_> {
    /// A class field's first value as one C expression with no statements (`made` may run off the game thread, so it
    /// runs no script code): a value the plain path writes with none, else a constant folded here (a negated number,
    /// a vector of such numbers, `none` as the option zeroed, in parentheses). Ok(None): it needs code.
    fn first_value(&mut self, m: usize, file: &str, v: &Expr, t: &Ty) -> Result<Option<String>, super::NotWritten> {
        let plain = {
            let mut fx = Fx::new(file, m);
            let x = self.expr(&mut fx, v)?;
            let x = self.coerce(&mut fx, &x, &self.ty(m, v), t, v.at)?;
            (fx.lines.is_empty() && fx.decls.is_empty()).then_some(x)
        };
        Ok(match plain {
            Some(x) => Some(x),
            None => self.folded_first(m, file, v, t),
        })
    }

    fn folded_first(&mut self, m: usize, file: &str, v: &Expr, t: &Ty) -> Option<String> {
        match &v.kind {
            ExprKind::Paren(x) => self.folded_first(m, file, x, t),
            ExprKind::None if matches!(t, Ty::Opt(_)) => {
                let ct = self.c_type(t)?;
                Some(format!("(({ct}){{0}})"))
            }
            ExprKind::Neg(x) if matches!(t, Ty::Int(_) | Ty::Float(_)) && matches!(x.kind, ExprKind::Number(_)) => {
                let inner = self.first_value(m, file, x, t).ok()??;
                Some(format!("(-{inner})"))
            }
            ExprKind::Call { callee, args } => {
                let n = match t {
                    Ty::Value(Value::Vec2) => 2,
                    Ty::Value(Value::Vec3) => 3,
                    Ty::Value(Value::Vec4) => 4,
                    _ => return None,
                };
                let std = matches!(self.resolved(m, v).or_else(|| self.resolved(m, callee)), Some(Resolved::Std(s)) if *s == ["vec2", "vec3", "vec4"][n - 2]);
                if !std || args.len() != n || args.iter().any(|a| a.name.is_some() || a.var) {
                    return None;
                }
                let f32_ty = Ty::Float(Scalar::F32);
                let mut parts = Vec::new();
                for a in args {
                    parts.push(self.first_value(m, file, &a.value, &f32_ty).ok()??);
                }
                Some(format!("((lm_vec{n}){{{}}})", parts.join(", ")))
            }
            _ => None,
        }
    }

    /// One class item: its C (data struct and `made` in lm.h's globals, the override bodies and wrappers, its define
    /// function, called from the mod's start).
    pub(super) fn class_item(&mut self, m: usize, me: ItemRef, c: &ast::ClassDecl, file: &str) -> Option<String> {
        let base = c.base.text.clone();
        let cname = self.item_c(me, &c.name.name);
        let data = format!("{cname}_d");
        let number = format!("{cname}_n");
        // Fields: their C types and first values (a C expression with no statements, so `made` runs no script code).
        let mut fields: Vec<(String, String, Option<String>)> = Vec::new();
        let mut ok = true;
        for member in &c.members {
            match member {
                ClassMember::Binding(b) => {
                    let ast::PatternKind::Path { .. } = &b.pattern.kind else {
                        self.say(m, b.pattern.at, "a class field bound by a pattern");
                        ok = false;
                        continue;
                    };
                    let Some(name) = pattern_name(&b.pattern) else {
                        ok = false;
                        continue;
                    };
                    let t = self.declared_ty(me, &format!(".{name}"));
                    let Some(ct) = self.c_type(&t) else {
                        self.say(m, b.pattern.at, "a class field of this type");
                        ok = false;
                        continue;
                    };
                    let first = match &b.value {
                        None => None,
                        Some(v) => match self.first_value(m, file, v, &t) {
                            Ok(Some(x)) => Some(x),
                            Ok(None) => {
                                self.say(m, v.at, "a class field whose first value needs code to run");
                                ok = false;
                                None
                            }
                            Err(_) => {
                                ok = false;
                                None
                            }
                        },
                    };
                    fields.push((name, ct, first));
                }
                ClassMember::Fn(f) => {
                    self.say(m, f.name.at, "a class's own fn");
                    ok = false;
                }
                ClassMember::Hook(h) => {
                    self.say(m, h.at, "a hook in a class");
                    ok = false;
                }
                ClassMember::Override(_) => {}
            }
        }
        if !ok {
            return None;
        }
        let mut out = String::new();
        // Its machines (`states M for <class>`, docs/modding/language/language.md, section 11.2): each kept in the data, run from `control`.
        let machines = self.machines_of(me);
        let control = match self.game.type_by_name(&base) {
            Ok(GameTy::View { record, .. }) => self.game.virtual_of(&record, "control", Some(0)),
            _ => None,
        };
        if !machines.is_empty() && control.is_none() {
            self.say(m, c.name.at, "states for a class whose base has no control() to run them from");
            return None;
        }
        let mut machine_code = String::new();
        let mut machine_gone = Vec::new();
        for mach in &machines {
            let (code, gone) = self.machine(me, &data, mach)?;
            machine_code.push_str(&code);
            machine_gone.extend(gone);
        }
        let has_data = !fields.is_empty() || !machines.is_empty();
        if has_data {
            let mut d = format!("typedef struct {data} {{\n");
            for (name, ct, _) in &fields {
                let _ = writeln!(d, "    {ct} {name};");
            }
            for mach in &machines {
                let _ = writeln!(d, "    {}_m {};", mach.c, mach.field);
            }
            let _ = write!(d, "}} {data};");
            self.globals.push(d);
        }
        self.globals.push(format!("extern int32_t {number}; /* class {}'s number (define_class), -1 before */", c.name.name));
        let _ = writeln!(out, "int32_t {number} = -1;\n");
        // `made`: the first values, in the data the kernel zeroed.
        let made = if fields.iter().any(|(_, _, f)| f.is_some()) {
            let made = format!("{cname}_made");
            let _ = writeln!(out, "static void {made}(void* object, void* data, void* user) {{\n    (void)object;\n    (void)user;\n    {data}* d = ({data}*)data;");
            for (name, ct, first) in &fields {
                if let Some(v) = first {
                    let _ = writeln!(out, "    {}", assign_line(&format!("d->{name}"), v, ct));
                }
            }
            out.push_str("}\n\n");
            made
        } else {
            "0".to_string()
        };
        // `gone` (docs/modding/language/classes.md): before the kernel frees an object's data, what it holds is given
        // back: a waiting state's frame, then each counted field. No script code runs (the object is gone): a
        // waiting state's task is not resumed, its defers do not run.
        let counted_fields: Vec<&(String, String, Option<String>)> = fields.iter().filter(|(_, ct, _)| counted(ct)).collect();
        let gone = if counted_fields.is_empty() && machine_gone.is_empty() {
            None
        } else {
            let gone = format!("{cname}_gone");
            let mut g = format!("static void {gone}(void* object, void* data, void* user) {{\n    (void)object;\n    (void)user;\n    {data}* d = ({data}*)data;\n");
            for f in &machine_gone {
                let _ = writeln!(g, "    {f}(d);");
            }
            for (name, ct, _) in counted_fields {
                let _ = writeln!(g, "    lm_rel_{ct}(&d->{name});");
            }
            g.push_str("}\n");
            Some((gone, g))
        };
        // The machines' run, at the start of each `control` of one of the class's objects (before its override's
        // body, or the base's own when the class has none).
        let run_machines = format!("{cname}_machines");
        if !machines.is_empty() {
            out.push_str(&machine_code);
            let _ = writeln!(out, "static void {run_machines}(LumaCall* call) {{\n    {data}* lm_d = ({data}*)luma_rt_core->class_data(luma_rt_self, call->args[0], {number});\n    if (lm_d == 0) return;");
            for mach in &machines {
                let _ = writeln!(out, "    (void){}_run(call, lm_d);", mach.c);
            }
            out.push_str("}\n\n");
        }
        if let Some((_, g)) = &gone {
            out.push_str(g);
            out.push('\n');
        }
        // The overrides: a body (a wrap's) and the hook the kernel calls.
        let mut table = Vec::new();
        let own_control = c.members.iter().any(|mb| matches!(mb, ClassMember::Override(f) if f.name.name == "control"));
        if let (false, false, Some((name, signature))) = (machines.is_empty(), own_control, &control) {
            let hook = format!("{cname}__machines_ov");
            let _ = writeln!(
                out,
                "static int32_t {hook}(LumaCall* call, void* user) {{\n    (void)user;\n    if (lumart_on_game_thread({}, {}, {})) {run_machines}(call);\n    luma_rt_core->proceed(call);\n    return 0;\n}}\n",
                c_string(file),
                c.name.at.line,
                c_string(name)
            );
            table.push(format!("    {{{}, {}, {hook}, 0}},", c_string(name), c_string(signature)));
        }
        for member in &c.members {
            let ClassMember::Override(f) = member else { continue };
            let key = format!(".{}", f.name.name);
            let Some(target) = self.checked.hooks.get(&(me, key.clone())).cloned() else { continue };
            let body = format!("{cname}__{}_ob", f.name.name);
            let hook = format!("{cname}__{}_ov", f.name.name);
            let text = self.override_body(m, me, f, &target.signature, &body, has_data.then_some((data.as_str(), number.as_str())), file)?;
            out.push_str(&text);
            let first = Body { c_name: body.clone(), file: file.to_string(), line: f.name.at.line };
            let machines_first = if f.name.name == "control" && !machines.is_empty() { format!("    {run_machines}(call);\n") } else { String::new() };
            let _ = writeln!(
                out,
                "static int32_t {hook}_next(LumaCall* call) {{\n    luma_rt_core->proceed(call);\n    return 0;\n}}\n\nstatic int32_t {hook}(LumaCall* call, void* user) {{\n    (void)user;\n    if (!lumart_on_game_thread({}, {}, {})) {{\n        luma_rt_core->proceed(call);\n        return 0;\n    }}\n{machines_first}    uint8_t proceeded = 0;\n    /* A script error before base's own ran: the base's runs (a wrap's rule, section 8). */\n    if ({body}(call, &proceeded, {hook}_next) != 0 && !proceeded) luma_rt_core->proceed(call);\n    return 0;\n}}\n",
                c_string(&first.file),
                first.line,
                c_string(&target.name)
            );
            table.push(format!("    {{{}, {}, {hook}, 0}},", c_string(&target.name), c_string(&target.signature)));
        }
        let table_name = format!("{cname}_ovt");
        if !table.is_empty() {
            let _ = writeln!(out, "static const LumaOverride {table_name}[{}] = {{\n{}\n}};\n", table.len(), table.join("\n"));
        }
        let define = format!("{cname}_define");
        let opt = |g: &Option<ast::GameName>| g.as_ref().map_or_else(|| "0".to_string(), |n| c_string(&n.text));
        let (size, align) = if has_data { (format!("(uint32_t)sizeof({data})"), format!("(uint32_t)_Alignof({data})")) } else { ("0".into(), "0".into()) };
        // Named, so a luma.h before `gone` says which member it lacks; a class with nothing to give back leaves it out
        // and builds against either.
        let gone_init = gone.as_ref().map_or_else(String::new, |(g, _)| format!(", .gone = {g}"));
        let _ = writeln!(
            out,
            "int32_t {define}(void) {{\n    static const LumaClassDef def = {{sizeof(LumaClassDef), {}, {}, {}, {}, {size}, {align}, {made}, 0, {}, {}{gone_init}}};\n    {number} = luma_rt_core->define_class(luma_rt_self, &def);\n    return {number} < 0;\n}}\n",
            c_string(&c.name.name),
            c_string(&base),
            opt(&c.maker),
            opt(&c.replaces),
            if table.is_empty() { "0".to_string() } else { table_name },
            table.len()
        );
        self.protos.push(format!("int32_t {define}(void);"));
        self.class_defines.push(define);
        Some(out)
    }

    /// An override's body: a wrap's body function (`call`, `lm_proceeded`, `lm_next`), its parameters the call's
    /// arguments (docs/modding/language/compiled-form.md, section 4), `self` the object, its fields in the class's data (`lm_d`), `return v` the call's result.
    #[allow(clippy::too_many_arguments)]
    fn override_body(&mut self, m: usize, me: ItemRef, f: &ast::FnDecl, signature: &str, c_name: &str, data: Option<(&str, &str)>, file: &str) -> Option<String> {
        let Ok(fnty) = self.game.hook_signature(signature) else {
            self.say(m, f.name.at, "an override whose signature does not cross");
            return None;
        };
        let sig = crate::ctype::parse_signature(signature).ok()?;
        let result_c = match &fnty.result {
            GameTy::Unit => None,
            GameTy::Scalar(s) => Some(super::scalar_c(*s).to_string()),
            GameTy::Value(v) => Some(super::value_c(*v).to_string()),
            GameTy::Enum(_) => Some("int32_t".to_string()),
            GameTy::View { .. } | GameTy::Opt(_) => Some("void*".to_string()),
            other => {
                self.say(m, f.name.at, &format!("an override of a function returning {other:?}"));
                return None;
            }
        };
        let mut fx = Fx::new(file, m);
        fx.hook = Some(HookFx { kind: HookKind::Wrap, result_c: result_c.clone(), texts: Vec::new(), outs: Vec::new(), name: String::new(), line: 0 });
        let mut k = 0;
        for p in &f.params {
            if p.is_self {
                continue;
            }
            let i = k;
            k += 1;
            let (Some(g), Some(c)) = (fnty.params.get(i), sig.params.get(fnty.c_index(i))) else { continue };
            // A pointer to a number, vector or enum: its own local (`T?`), read and written through the pointer.
            if let Some(Resolved::Local(id)) = p.name.as_ref().and_then(|n| self.checked.named.get(&(m, n.at))) {
                if self.bind_place_param(&mut fx, *id, g, c, fnty.c_index(i) + 1)? {
                    fx.override_params.push((Some(*id), format!("l{id}"), g.clone()));
                    continue;
                }
            }
            let Some(place) = self.hook_param(g, c, fnty.c_index(i) + 1) else {
                self.say(m, p.at, &format!("an override parameter of type {g:?}"));
                return None;
            };
            let id = match p.name.as_ref().and_then(|n| self.checked.named.get(&(m, n.at))) {
                Some(Resolved::Local(id)) => {
                    fx.places.insert(*id, place.clone());
                    fx.declared.insert(*id);
                    Some(*id)
                }
                _ => None,
            };
            fx.override_params.push((id, place, g.clone()));
        }
        // `return v`: written to the call's result (the script's type into the row's C type).
        if let Some(rc) = &result_c {
            let t = self.declared_ty(me, &format!(".{}->", f.name.name));
            let ct = self.c_type(&t)?;
            fx.decl(format!("{ct} lm_rv;"));
            fx.decl(format!("{ct}* lm_ret;"));
            fx.emit("lm_ret = &lm_rv;".into());
            fx.ret = Some((ct, t));
            fx.override_rc = Some(rc.clone());
        }
        if let Some((d, number)) = data {
            fx.decl(format!("{d}* lm_d;"));
            fx.emit(format!("lm_d = ({d}*)luma_rt_core->class_data(luma_rt_self, call->args[0], {number});"));
            fx.fail_if("lm_d == 0");
            fx.class_data = Some(me);
        }
        self.block(&mut fx, &f.body);
        let head = format!("static int32_t {c_name}(LumaCall* call, uint8_t* lm_proceeded, int32_t (*lm_next)(LumaCall*))");
        self.out_defers(&mut fx);
        fx.place_args_tail();
        Some(fx.function(&head, f.name.at))
    }

    /// `base.<name>(...)` in an override: the base class's own version (the kernel's proceed) with these arguments. A
    /// value parameter's slot is set to its argument for the call and given back after, so the override's own names
    /// keep their values; a reference parameter is passed on as the override has it.
    pub(super) fn base_call(&mut self, fx: &mut Fx, e: &Expr, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        let params = fx.override_params.clone();
        let mut sets = Vec::new();
        for (a, (id, place, g)) in args.iter().zip(params.iter()) {
            let own = matches!((self.resolved(m, &a.value), id), (Some(Resolved::Local(x)), Some(id)) if x == id);
            if own {
                continue;
            }
            if matches!(g, GameTy::Opt(x) if matches!(**x, GameTy::Place { .. })) {
                return self.not_yet(fx, &a.value, "base.f(...) given another value for a pointer parameter to a number or vector");
            }
            if !matches!(g, GameTy::Scalar(_) | GameTy::Value(_) | GameTy::Enum(_) | GameTy::View { .. } | GameTy::Opt(_)) {
                return self.not_yet(fx, &a.value, "base.f(...) given another object for a reference parameter");
            }
            let from = self.ty(m, &a.value);
            let to = match id {
                Some(id) => self.checked.locals[*id as usize].ty.clone(),
                None => from.clone(),
            };
            let x = self.expr(fx, &a.value)?;
            let x = self.coerce(fx, &x, &from, &to, a.value.at)?;
            let ct = self.c_type(&to).ok_or_else(super::lost)?;
            let v = fx.temp(&ct, &x);
            sets.push((place.clone(), v, ct));
        }
        let mut saved = Vec::new();
        for (place, v, ct) in &sets {
            saved.push(fx.temp(ct, place));
            fx.emit(format!("{place} = {v};"));
        }
        fx.place_args_back();
        fx.emit("*lm_proceeded = 1;".into());
        fx.emit("(void)lm_next(call);".into());
        fx.place_args_in();
        for ((place, _, _), s) in sets.iter().zip(saved) {
            fx.emit(format!("{place} = {s};"));
        }
        match fx.hook.as_ref().and_then(|h| h.result_c.clone()) {
            Some(rc) => {
                let t = self.ty(m, e);
                let ct = self.c_type(&t).ok_or_else(super::lost)?;
                Ok(fx.temp(&ct, &format!("LUMA_RESULT(call, {rc})")))
            }
            None => Ok(String::new()),
        }
    }

    /// A mod class's own field as a C place, or None when `e` is not one: `self`'s in its class's override (`lm_d`),
    /// another object's through `class_data` (a script error when the object is not of the class).
    pub(super) fn class_field_place(&mut self, fx: &mut Fx, e: &Expr) -> Option<Ex> {
        let m = fx.module;
        let r = match &e.kind {
            ExprKind::Name { name, .. } => self.resolved(m, e).or_else(|| self.checked.named.get(&(m, name.at))),
            ExprKind::Member { optional: false, .. } => self.resolved(m, e),
            _ => None,
        };
        let Some(Resolved::Member { owner, name }) = r else { return None };
        let (owner, name) = (*owner, name.clone());
        let ItemKind::Class(cd) = &self.item(owner).kind else { return None };
        let cname = self.item_c(owner, &cd.name.name);
        let own = match &e.kind {
            ExprKind::Name { .. } => true,
            ExprKind::Member { base, .. } => matches!(base.kind, ExprKind::SelfValue),
            _ => false,
        };
        if own && fx.class_data == Some(owner) {
            return Some(Ok(format!("lm_d->{name}")));
        }
        let ExprKind::Member { base, .. } = &e.kind else { return Some(self.not_yet(fx, e, "a class field outside its class's overrides")) };
        let b = match self.expr(fx, base) {
            Ok(b) => b,
            Err(x) => return Some(Err(x)),
        };
        let o = fx.temp("void*", &b);
        let d = fx.temp("void*", &format!("luma_rt_core->class_data(luma_rt_self, {o}, {cname}_n)"));
        fx.fail_if(&format!("{d} == 0"));
        Some(Ok(format!("(({cname}_d*){d})->{name}")))
    }

}

/// The name a field's `var name` / `let name` binds.
fn pattern_name(p: &ast::Pattern) -> Option<String> {
    match &p.kind {
        ast::PatternKind::Path { path, .. } if path.len() == 1 => Some(path[0].name.clone()),
        _ => None,
    }
}
