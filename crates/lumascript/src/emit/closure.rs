//! Function values (docs/modding/language/language.md, section 5.4): a closure written where a value goes, a function of the mod used as a value, and a
//! call of a value of function type.
//!
//! A value of type `fn(P...) -> R` is a `lumart_fn` (lumart.h): a counted block whose head holds its C function and how
//! its captures are given back. Its C function is `int32_t f(lumart_fn self, P..., R* lm_ret)`, the status as every
//! script function's. A closure's block carries a copy of each local of the body around it that the body reads
//! (captured values are read, never written: 5.4), taken where the closure is made; one that reads none, and a function
//! used as a value, is a static block counted LUMART_FOREVER. Nothing runs at run time but the C written here.
//!
//! A block names the mod that made it (`owner`) and an entry for its type (`entry`). A call of a value this mod made
//! is a plain C call; a call of another mod's value goes through LumaCore.call_mod to its maker's entry
//! (lumart_call_other), so a fault in that code turns its maker off and the caller goes on with an empty result
//! (docs/modding/concepts/when-a-mod-fails.md, "A fault in another mod's code").

use super::{c_string, counted, lost, Ex, Fx, Gen};
use crate::ast::{self, Block, Expr, Pos};
use crate::check::{LocalId, Resolved, Ty};
use std::fmt::Write as _;

/// How a list method's function argument is run for each item (`list.map(f)`, `filter`, `any`, `all`, `sum`,
/// `sort_by`).
pub(super) enum Apply<'x> {
    /// A closure written as the argument: its body in place.
    Inline(&'x [ast::Param], &'x Block),
    /// Any other function value: worked out once into `f`, called for each item.
    Value { f: String, ps: Vec<Ty>, r: Ty, src: String, at: Pos },
}

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

    /// The entry another mod calls this mod's function values of type `fn(ps) -> r` by (one per C type, in this mod's
    /// library, so the kernel finds this mod as the code's giver): `uint64_t e(self, argv, out, 0)`, each argument read
    /// through argv's pointer, the status returned. Its code runs as this mod's own: its reports are this mod's.
    pub(super) fn fn_entry(&mut self, ps: &[Ty], r: &Ty) -> Option<String> {
        let cast = self.fn_cast(ps, r)?;
        if let Some(name) = self.fn_entries.get(&cast) {
            return Some(name.clone());
        }
        let name = format!("lm_fe_{}", self.fn_entries.len());
        let mut pass = vec!["f".to_string()];
        for (i, p) in ps.iter().enumerate() {
            let ct = self.c_type(p)?;
            pass.push(format!("*({ct}*)av[{i}]"));
        }
        if *r != Ty::Unit {
            let ct = self.c_type(r)?;
            pass.push(format!("({ct}*)(uintptr_t)o"));
        }
        // External: lm.h declares it for every module, and the first module that needs it defines it.
        let head = format!("uint64_t {name}(uint64_t s, uint64_t a, uint64_t o, uint64_t u)");
        self.protos.push(format!("{head};"));
        let _ = writeln!(
            self.closures,
            "{head} {{\n    (void)a;\n    (void)o;\n    (void)u;\n    lumart_fn f = (lumart_fn)(uintptr_t)s;\n    void* const* av = (void* const*)(uintptr_t)a;\n    (void)av;\n    lumart_as_link lm_link;\n    lumart_as_enter(&lm_link, 0, 0, 0);\n    const int32_t st = (({cast})(f->code))({});\n    lumart_as_leave(&lm_link);\n    return (uint64_t)(uint32_t)st;\n}}\n",
            pass.join(", ")
        );
        self.fn_entries.insert(cast, name.clone());
        Some(name)
    }

    /// C that calls the function value `f` of type `fn(tys) -> r` with `args` (C expressions of the parameters' C
    /// types) and `out` (the result's place, `&o`, or "0" for unit): `plain` is the statement for this mod's own value
    /// (the caller's own call, with its status handling); another mod's goes through lumart_call_other, its status
    /// dropped (a failure there is its maker's, said in its log). Lines without indentation.
    pub(super) fn fn_value_call(&mut self, f: &str, tys: &[Ty], args: &[String], out: &str, plain: &str) -> Option<Vec<String>> {
        let mut lines = vec![format!("if ({f}->owner == (const void*)&luma_rt_self) {{"), format!("    {plain}"), "} else {".to_string()];
        let mut ptrs = Vec::new();
        for (i, (t, a)) in tys.iter().zip(args).enumerate() {
            let ct = self.c_type(t)?;
            lines.push(format!("    {ct} lm_xa{i} = {a};"));
            ptrs.push(format!("(void*)&lm_xa{i}"));
        }
        if ptrs.is_empty() {
            ptrs.push("0".into());
        }
        lines.push(format!("    void* const lm_xv[] = {{{}}};", ptrs.join(", ")));
        lines.push(format!("    (void)lumart_call_other({f}, lm_xv, (void*){out});"));
        lines.push("}".into());
        Some(lines)
    }

    /// A closure written where a value goes: its C function and, when it captures, its block's type, maker and drop,
    /// written into the module's code; here, the value made. `own` is a local function's name (`fn f(..) { }` in a
    /// body): inside, `f` is the closure being run (`lm_fn`), so it may call itself, never a copy made before it was.
    pub(super) fn closure_value(&mut self, fx: &mut Fx, at: Pos, params: &[ast::Param], body: &Block, t: &Ty, own: Option<LocalId>) -> Ex {
        let m = fx.module;
        let Ty::Fn(_, r) = t else { return self.say_at(fx, at, ny!("ClosureType")) };
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
        if let Some(id) = own {
            inner.places.insert(id, "lm_fn".into());
            inner.declared.insert(id);
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
        let text = inner.function(&head, at);
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
        let entry = self.fn_entry(pts, &r).ok_or_else(lost)?;
        if caps.is_empty() {
            // Declared at the module's head: the code reading it comes before the closures' code.
            self.literal_defs.push(format!("static lumart_fn_block {name}_b;"));
            let _ = writeln!(code, "{text}static lumart_fn_block {name}_b = {{LUMART_FOREVER, 0, 0, (const void*){name}, (const void*)&luma_rt_self, {entry}}};\n");
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
        let _ = writeln!(code, "    b->h.rc = 1;\n    b->h.size = (uint32_t)sizeof({env});\n    b->h.drop = {name}_drop;\n    b->h.code = (const void*){name};\n    b->h.owner = (const void*)&luma_rt_self;\n    b->h.entry = {entry};");
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
        fx.fail_if(&format!("{tmp} == 0 && lumart_raise({}, {}, 911, 1, (const char* const[]){{{src}}})", c_string(&fx.file), at.line));
        Ok(tmp)
    }

    fn closures_push(&mut self, code: String, value: String) -> String {
        self.closures.push_str(&code);
        value
    }

    /// A function of the mod used as a value: a static block whose C function calls it.
    pub(super) fn fn_item_value(&mut self, fx: &mut Fx, e: &Expr, c_name: &str, t: &Ty) -> Ex {
        let Ty::Fn(ps, r) = t else { return self.not_yet(fx, e, ny!("FnAsValue")) };
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
            let entry = self.fn_entry(ps, r).ok_or_else(lost)?;
            let _ = writeln!(
                self.closures,
                "{head} {{\n    (void)lm_fn;\n    return {c_name}({});\n}}\n\nstatic lumart_fn_block {key}_b = {{LUMART_FOREVER, 0, 0, (const void*){key}, (const void*)&luma_rt_self, {entry}}};\n",
                pass.join(", ")
            );
        }
        Ok(format!("((lumart_fn)&{key}_b)"))
    }

    /// `f(args)` for a value `f` of type `fn(ps) -> r`: its C function called with the value itself first; its status
    /// passed on. A value never set (zero) is S0906. Another mod's value is called through its maker's entry
    /// (fn_value_call): when that does not run to its end, the result is empty and the caller goes on.
    pub(super) fn call_fn_value(&mut self, fx: &mut Fx, e: &Expr, callee: &Expr, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        let Ty::Fn(ps, r) = self.ty(m, callee) else { return self.not_yet(fx, e, ny!("CallValue")) };
        let v = self.expr(fx, callee)?;
        let f = fx.temp("lumart_fn", &v);
        let mut xs = Vec::new();
        for a in args {
            let at = self.ty(m, &a.value);
            let x = self.expr(fx, &a.value)?;
            xs.push((x, at, a.at));
        }
        self.call_value_with(fx, &f, &ps, &r, &xs, &super::expr_text(callee), e.at)
    }

    /// One call of the function value `f` (a `lumart_fn` already worked out) with these C values, each of its type,
    /// given to the parameters' types: a plain C call for this mod's value, LumaCore.call_mod for another's. Its
    /// result (empty for none); a null value is the script error naming `src`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_value_with(&mut self, fx: &mut Fx, f: &str, ps: &[Ty], r: &Ty, args: &[(String, Ty, Pos)], src: &str, at: Pos) -> Ex {
        let cast = self.fn_cast(ps, r).ok_or_else(lost)?;
        let mut cs = vec![f.to_string()];
        let mut xs = Vec::new();
        for ((x, from, xat), p) in args.iter().zip(ps.iter()) {
            let x = self.coerce(fx, x, from, p, *xat)?;
            cs.push(x.clone());
            xs.push(x);
        }
        let out = if *r != Ty::Unit {
            let ct = self.c_type(r).ok_or_else(lost)?;
            let o = fx.fresh(&ct);
            cs.push(format!("&{o}"));
            Some(o)
        } else {
            None
        };
        let src = c_string(src);
        fx.fail_if(&format!("{f} == 0 && lumart_raise({}, {}, 906, 1, (const char* const[]){{{src}}})", c_string(&fx.file), at.line));
        let place = out.as_ref().map_or_else(|| "0".to_string(), |o| format!("&{o}"));
        let plain = format!("if ((({cast})({f}->code))({}) != 0) {{ lm_st = 1; goto lm_out; }}", cs.join(", "));
        let lines = self.fn_value_call(f, ps, &xs, &place, &plain).ok_or_else(lost)?;
        fx.emit("{".into());
        for line in lines {
            fx.emit(format!("    {line}"));
        }
        fx.emit("}".into());
        Ok(out.unwrap_or_default())
    }

    /// A list method's function argument for each item: a closure written there, run in place, or a function value,
    /// called (its result given to the type the method wants).
    pub(super) fn apply(&mut self, fx: &mut Fx, how: &Apply, args: &[(String, Ty)], ret: &Ty) -> Ex {
        match how {
            Apply::Inline(params, body) => self.inline_closure(fx, params, body, args, ret),
            Apply::Value { f, ps, r, src, at } => {
                let xs: Vec<(String, Ty, Pos)> = args.iter().map(|(x, t)| (x.clone(), t.clone(), *at)).collect();
                let v = self.call_value_with(fx, f, ps, r, &xs, src, *at)?;
                if *ret == Ty::Unit {
                    return Ok(v);
                }
                let v = self.coerce(fx, &v, r, ret, *at)?;
                let ct = self.c_type(ret).ok_or_else(lost)?;
                Ok(fx.temp(&ct, &v))
            }
        }
    }
}
