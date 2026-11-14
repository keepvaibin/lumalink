//! Function values (docs/modding/language/language.md, section 5.4): a closure written where a value goes, a function of the mod used as a value, and a
//! call of a value of function type.
//!
//! A value of type `fn(P...) -> R` is a `lumart_fn` (lumart.h): a counted block whose head holds its C function and how
//! its captures are given back. Its C function is `int32_t f(lumart_fn self, P..., R* lm_ret)`, the status as every
//! script function's. A closure's block carries a copy of each local of the body around it that the body reads
//! (captured values are read, never written: 5.4), taken where the closure is made; one that reads none, and a function
//! used as a value, is a static block counted LUMART_FOREVER. Nothing runs at run time but the C written here.

use super::{c_string, counted, lost, Ex, Fx, Gen};
use crate::ast::{self, Block, Expr};
use crate::check::{LocalId, Resolved, Ty};
use std::fmt::Write as _;

impl<'a> Gen<'a> {
    /// The C function type of a value of type `fn(ps) -> r`, as a cast: `int32_t (*)(lumart_fn, P..., R*)`.
    pub(super) fn fn_cast(&mut self, ps: &[Ty], r: &Ty) -> Option<String> {
        let mut cs = vec!["lumart_fn".to_string()];
        for p in ps {
            cs.push(self.c_type(p)?);
        }
        if *r != Ty::Unit {
            cs.push(format!("{}*", self.c_type(r)?));
        }
        Some(format!("int32_t (*)({})", cs.join(", ")))
    }

    /// A closure written where a value goes: its C function and, when it captures, its block's type, maker and drop,
    /// written into the module's code; here, the value made.
    pub(super) fn closure_value(&mut self, fx: &mut Fx, e: &Expr, params: &[ast::Param], body: &Block, t: &Ty) -> Ex {
        let m = fx.module;
        let Ty::Fn(_, r) = t else { return self.not_yet(fx, e, "a closure of this type") };
        let r = (**r).clone();
        let n = self.closure_n;
        self.closure_n += 1;
        let name = format!("lm_cl_{m}_{n}");
        let env = format!("lm_env_{m}_{n}");
        // The closure's own parameters.
        let mut head = vec!["lumart_fn lm_fn".to_string()];
        let mut inner = Fx::new(&fx.file, m);
        let Ty::Fn(pts, _) = t else { return Err(lost()) };
        for (i, p) in params.iter().enumerate() {
            let Some(pn) = &p.name else {
                // `_`: a parameter no name reads.
                let ct = self.c_type(pts.get(i).ok_or_else(lost)?).ok_or_else(lost)?;
                head.push(format!("{ct} lm_p{i}"));
                continue;
            };
            let Some(Resolved::Local(id)) = self.checked.named.get(&(m, pn.at)) else { return Err(lost()) };
            let id = *id;
            let lt = self.checked.locals[id as usize].ty.clone();
            let ct = self.c_type(&lt).ok_or_else(lost)?;
            head.push(format!("{ct} l{id}"));
            inner.declared.insert(id);
        }
        if r != Ty::Unit {
            let ct = self.c_type(&r).ok_or_else(lost)?;
            head.push(format!("{ct}* lm_ret"));
            inner.ret = Some((ct, r.clone()));
        }
        // Every local around it, read through the block (only those the body names are kept).
        let mut outer: Vec<LocalId> = fx.declared.iter().copied().chain(fx.places.keys().copied()).collect();
        outer.sort_unstable();
        outer.dedup();
        for id in &outer {
            if !inner.declared.contains(id) {
                inner.places.insert(*id, format!("lm_env->c{id}"));
                inner.declared.insert(*id);
            }
        }
        self.block(&mut inner, body);
        self.out_defers(&mut inner);
        let head = format!("int32_t {name}({})", head.join(", "));
        let text = inner.function(&head, e.at);
        // The captures: the locals whose block place the body's C reads.
        let mut caps: Vec<(LocalId, String)> = Vec::new();
        for id in &outer {
            let key = format!("lm_env->c{id}");
            let read = text.match_indices(&key).any(|(i, _)| !text[i + key.len()..].starts_with(|c: char| c.is_ascii_digit()));
            if read {
                let lt = self.checked.locals[*id as usize].ty.clone();
                let ct = self.c_type(&lt).ok_or_else(lost)?;
                caps.push((*id, ct));
            }
        }
        let mut code = String::new();
        self.protos.push(format!("{head};"));
        if caps.is_empty() {
            // Declared at the module's head: the code reading it comes before the closures' code.
            self.literal_defs.push(format!("static lumart_fn_block {name}_b;"));
            let _ = writeln!(code, "{text}static lumart_fn_block {name}_b = {{LUMART_FOREVER, 0, 0, (const void*){name}}};\n");
            return Ok(self.closures_push(code, format!("((lumart_fn)&{name}_b)")));
        }
        // The block: its head, then each capture.
        let _ = writeln!(code, "typedef struct {env} {{\n    lumart_fn_block h;");
        for (id, ct) in &caps {
            let _ = writeln!(code, "    {ct} c{id};");
        }
        let _ = writeln!(code, "}} {env};\n");
        let _ = writeln!(code, "static void {name}_drop(lumart_fn_block* b) {{\n    {env}* lm_env = ({env}*)b;");
        for (id, ct) in &caps {
            if counted(ct) {
                let _ = writeln!(code, "    lm_rel_{ct}(&lm_env->c{id});");
            }
        }
        let _ = writeln!(code, "    lumart_free(b, b->size);\n}}\n");
        // The body reads its captures through `lm_fn`.
        let body_text = text.replacen(&format!("{head} {{\n"), &format!("{head} {{\n    {env}* lm_env = ({env}*)lm_fn;\n"), 1);
        code.push_str(&body_text);
        // The maker: a new block counted once, each capture copied in with its count taken.
        let args: Vec<String> = caps.iter().map(|(id, ct)| format!("{ct} c{id}")).collect();
        let mk = format!("lumart_fn {name}_mk({})", args.join(", "));
        self.protos.push(format!("{mk};"));
        let _ = writeln!(code, "{mk} {{\n    {env}* b = ({env}*)lumart_alloc(sizeof({env}));");
        let _ = writeln!(code, "    if (b == 0) return 0;\n    __builtin_memset(b, 0, sizeof *b);");
        let _ = writeln!(code, "    b->h.rc = 1;\n    b->h.size = (uint32_t)sizeof({env});\n    b->h.drop = {name}_drop;\n    b->h.code = (const void*){name};");
        for (id, ct) in &caps {
            let _ = writeln!(code, "    {}", super::assign_line(&format!("b->c{id}"), &format!("c{id}"), ct));
        }
        let _ = writeln!(code, "    return &b->h;\n}}\n");
        self.closures.push_str(&code);
        // Made here: a temporary that owns the new block's one count (zero when taken: no copy, no count).
        let vals: Vec<String> = caps.iter().map(|(id, _)| fx.local(*id)).collect();
        let tmp = fx.take("lumart_fn");
        fx.emit(format!("{tmp} = {name}_mk({});", vals.join(", ")));
        let src = c_string("closure");
        fx.fail_if(&format!("{tmp} == 0 && lumart_raise({}, {}, 911, 1, (const char* const[]){{{src}}})", c_string(&fx.file), e.at.line));
        Ok(tmp)
    }

    fn closures_push(&mut self, code: String, value: String) -> String {
        self.closures.push_str(&code);
        value
    }

    /// A function of the mod used as a value: a static block whose C function calls it.
    pub(super) fn fn_item_value(&mut self, fx: &mut Fx, e: &Expr, c_name: &str, t: &Ty) -> Ex {
        let Ty::Fn(ps, r) = t else { return self.not_yet(fx, e, "this function as a value") };
        let key = format!("{c_name}_fv{}", fx.module);
        if self.fn_values.insert(key.clone()) {
            let mut head = vec!["lumart_fn lm_fn".to_string()];
            let mut pass = Vec::new();
            for (i, p) in ps.iter().enumerate() {
                let ct = self.c_type(p).ok_or_else(lost)?;
                head.push(format!("{ct} p{i}"));
                pass.push(format!("p{i}"));
            }
            if **r != Ty::Unit {
                let ct = self.c_type(r).ok_or_else(lost)?;
                head.push(format!("{ct}* lm_ret"));
                pass.push("lm_ret".into());
            }
            let head = format!("int32_t {key}({})", head.join(", "));
            self.protos.push(format!("{head};"));
            self.literal_defs.push(format!("static lumart_fn_block {key}_b;"));
            let _ = writeln!(
                self.closures,
                "{head} {{\n    (void)lm_fn;\n    return {c_name}({});\n}}\n\nstatic lumart_fn_block {key}_b = {{LUMART_FOREVER, 0, 0, (const void*){key}}};\n",
                pass.join(", ")
            );
        }
        Ok(format!("((lumart_fn)&{key}_b)"))
    }

    /// `f(args)` for a value `f` of type `fn(ps) -> r`: its C function called with the value itself first; its status
    /// passed on. A value never set (zero) is S0906.
    pub(super) fn call_fn_value(&mut self, fx: &mut Fx, e: &Expr, callee: &Expr, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        let Ty::Fn(ps, r) = self.ty(m, callee) else { return self.not_yet(fx, e, "a call of this value") };
        let cast = self.fn_cast(&ps, &r).ok_or_else(lost)?;
        let v = self.expr(fx, callee)?;
        let f = fx.temp("lumart_fn", &v);
        let mut cs = vec![f.clone()];
        for (a, p) in args.iter().zip(ps.iter()) {
            let at = self.ty(m, &a.value);
            let x = self.expr(fx, &a.value)?;
            let x = self.coerce(fx, &x, &at, p, a.at)?;
            cs.push(x);
        }
        let out = if *r != Ty::Unit {
            let ct = self.c_type(&r).ok_or_else(lost)?;
            let o = fx.fresh(&ct);
            cs.push(format!("&{o}"));
            Some(o)
        } else {
            None
        };
        let src = c_string(&super::expr_text(callee));
        fx.fail_if(&format!("{f} == 0 && lumart_raise({}, {}, 906, 1, (const char* const[]){{{src}}})", c_string(&fx.file), e.at.line));
        fx.fail_if(&format!("(({cast})({f}->code))({}) != 0", cs.join(", ")));
        Ok(out.unwrap_or_default())
    }
}
