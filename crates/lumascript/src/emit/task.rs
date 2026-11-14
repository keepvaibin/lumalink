//! Tasks and `wait` (docs/modding/language/language.md, section 11.1): no scheduler in the game.
//!
//! A task's body is written as a function's is, its locals and temporaries at the top; the function is its step
//! function, run by lumart from the mod's frame handler. Its frame (`lm_task_<n>_frame`: lumart's head, then every local,
//! temporary and text builder) keeps them between frames: the step function copies them in at its start and out at
//! `lm_park`, where a `wait` goes after setting the step to resume at, so no stack is kept and the C stays as a plain
//! function's (a `wait` is the first thing its statement reads, so nothing of the statement is held across it).
//! Each `wait` K is the label `lm_wK`, reached from the top by a `switch` of gotos (gotos, so that no `case` falls
//! inside a C block of the body), where its condition is read again on each frame.

use super::{assign_line, c_string, Fx, Gen, NotWritten};
use crate::ast::{self, Expr, ExprKind, Pos};
use crate::check::{ItemRef, Resolved, Ty};
use crate::game::Scalar;
use std::fmt::Write as _;

type Ex = Result<String, NotWritten>;

impl<'a> Gen<'a> {
    /// The C name of a task (docs/modding/language/compiled-form.md, section 2): `lm_task_<n>`, n counting the mod's tasks in written order (modules in order).
    pub(super) fn task_c(&self, item: ItemRef) -> String {
        let mut n = 0;
        for (m, s) in self.sources.iter().enumerate().take(item.module + 1) {
            for (i, it) in s.module.items.iter().enumerate() {
                if m == item.module && i == item.item {
                    return format!("lm_task_{n}");
                }
                if matches!(it.kind, ast::ItemKind::Task { .. }) {
                    n += 1;
                }
            }
        }
        format!("lm_task_{n}")
    }

    /// `task name(params) { body }`: its frame `lm_task_<n>_frame`, its step function `lm_task_<n>_step`, and
    /// `lm_task_<n>(params, &handle)`, which `start` calls.
    pub(super) fn task(&mut self, m: usize, me: ItemRef, f: &ast::FnDecl, file: &str) -> Option<String> {
        let c_name = self.task_c(me);
        let Some((head, params, _, _)) = self.fn_head(me, "", &c_name, f) else {
            self.say(m, f.name.at, "a task with a parameter of this type");
            return None;
        };
        // A result (11.1a) is kept by the task, not written by its start function: fn_head's result parameter goes.
        let head = match self.checked.types.get(&(me, "->".to_string())).cloned().and_then(|t| self.c_type(&t)) {
            Some(rc) => {
                let only = format!("({rc}* lm_ret)");
                if head.ends_with(&only) {
                    format!("{}(void)", &head[..head.len() - only.len()])
                } else {
                    head.replacen(&format!(", {rc}* lm_ret)"), ")", 1)
                }
            }
            None => head,
        };
        let head = match head.strip_suffix("(void)") {
            Some(h) => format!("{h}(lumart_task* lm_ret)"),
            None => format!("{}, lumart_task* lm_ret)", &head[..head.len() - 1]),
        };
        let frame = format!("{c_name}_frame");
        let step = format!("{c_name}_step");
        let mut fx = Fx::new(file, m);
        fx.task = true;
        // A result (11.1a): kept through `lm_resv`, a field of the frame.
        if let Some(t) = self.checked.types.get(&(me, "->".to_string())).cloned() {
            let ct = self.c_type(&t)?;
            fx.decl(format!("{ct} lm_resv;"));
            fx.task_ret = Some((ct, t));
        }
        // The parameters are the frame's first fields, set by the start function (a count of its own each).
        let mut sets = Vec::new();
        for (id, var) in params {
            let t = self.checked.locals[id as usize].ty.clone();
            let ct = self.c_type(&t)?;
            if var {
                self.say(m, f.name.at, "a task's var parameter");
                return None;
            }
            fx.decl(format!("{ct} l{id};"));
            fx.declared.insert(id);
            sets.push(assign_line(&format!("lm_f->l{id}"), &format!("l{id}"), &ct));
        }
        self.block(&mut fx, &f.body);
        self.out_defers(&mut fx);
        // Without a result: ended by itself (no script error, not stopped), a call's pending gives `()`.
        if fx.task_ret.is_none() {
            fx.tail.insert(0, format!("    if (lm_st == 0 && !lm_f->h.stop) lumart_task_done(lm_f, {}, {});", c_string(file), f.name.at.line));
        }
        self.tasks_used = true;
        let mut out = fx.task_step(&frame, &step, f.name.at);
        let _ = writeln!(
            out,
            "{head} {{\n    {frame}* lm_f = ({frame}*)lumart_task_new(sizeof({frame}), {step}, lm_ret, {}, {});\n    if (lm_f == 0) return 1;",
            c_string(file),
            f.name.at.line
        );
        for s in sets {
            let _ = writeln!(out, "    {s}");
        }
        out.push_str("    return 0;\n}\n\n");
        self.protos.push(format!("{head};"));
        Some(out)
    }

    /// An `on start` that waits: a task whose first step runs now, as the handler; a script error in that step
    /// is the handler's, the rest of the task runs from the frames after.
    pub(super) fn start_task_handler(&mut self, fx: &Fx, name: &str, at: Pos) -> String {
        let frame = format!("{name}_frame");
        let step = format!("{name}_step");
        self.tasks_used = true;
        let mut out = fx.task_step(&frame, &step, at);
        let _ = writeln!(
            out,
            "int32_t {name}(void) {{\n    lumart_task t;\n    if (lumart_task_new(sizeof({frame}), {step}, &t, {}, {}) == 0) return 1;\n    return lumart_task_first(t) == 1;\n}}\n",
            c_string(&fx.file),
            at.line
        );
        out
    }

    /// `start task(args)`: the arguments as a call's, the task made by its start function; the handle.
    pub(super) fn start_expr(&mut self, fx: &mut Fx, e: &Expr, inner: &Expr) -> Ex {
        let m = fx.module;
        let ExprKind::Call { args, .. } = &inner.kind else { return self.not_yet(fx, e, "start of this kind") };
        let Some(Resolved::Item(item)) = self.resolved(m, inner).cloned() else { return self.not_yet(fx, e, "start of this kind") };
        // Another mod's task runs in that mod's task table: its call gives the pending to wait on (11.1a); no `task`
        // of this mod's can stand for it.
        if matches!(item.owner, super::Owner::Library(_)) {
            return self.not_yet(fx, e, "start of another mod's task (call it: `wait f(args)` waits on its pending)");
        }
        let ast::ItemKind::Task { name, params, body, .. } = &self.item(item).kind else { return self.not_yet(fx, e, "start of this kind") };
        let f = ast::FnDecl { name: name.clone(), params: params.clone(), ret: None, body: body.clone() };
        let c_name = self.task_c(item);
        self.start_ret = true;
        self.call_fn(fx, e, item, "", &c_name, &f, None, args)
    }

    /// A call of a task with a result (11.1a): the task started as `start` starts it, its handle and this mod's
    /// operations the pending.
    pub(super) fn task_pending(&mut self, fx: &mut Fx, e: &Expr, item: ItemRef, args: &[ast::Arg]) -> Ex {
        let ast::ItemKind::Task { name, params, body, ret } = &self.item(item).kind else { return self.not_yet(fx, e, "a call of this task") };
        let watch = ret.is_none();
        let f = ast::FnDecl { name: name.clone(), params: params.clone(), ret: None, body: body.clone() };
        let c_name = self.task_c(item);
        self.start_ret = true;
        let t = self.call_fn(fx, e, item, "", &c_name, &f, None, args)?;
        let t = fx.temp("lumart_task", &t);
        // Without a result: its end by itself is kept as `()` for this pending (lumart_task_watch).
        if watch {
            fx.emit(format!("lumart_task_watch({t});"));
        }
        Ok(fx.temp("lumart_pending", &format!("((lumart_pending){{{t}, &lumart_pending_own}})")))
    }

    /// `return v` in a task with a result (11.1a): `v` kept under the task's handle, then its way out.
    pub(super) fn task_return(&mut self, fx: &mut Fx, e: &Expr, at: Pos) {
        let Some((_, t)) = fx.task_ret.clone() else { return };
        self.value_into(fx, e, "lm_resv", &t);
        let Some(d) = self.desc(&t) else {
            self.say(fx.module, at, "a task giving a value of this type");
            return;
        };
        fx.fail_if(&format!("lumart_task_result_keep(lm_f, &lm_resv, {d}, {}, {}) != 0", c_string(&fx.file), at.line));
        fx.emit("goto lm_out;".into());
    }

    /// `wait p` for a pending (11.1a): parks until its task ends. With a value: the value, copied into this mod's memory
    /// by this mod's own copy (a library's task) or as it is. Without one (stopped, failed, read already): this task
    /// ends as stopped, its way out run (the wait is cancelled with it). A copy that failed is this task's script error.
    fn wait_pending(&mut self, fx: &mut Fx, e: &Expr, v: &str, of: &Ty) -> Ex {
        let (file, line) = (c_string(&fx.file), e.at.line);
        // `()`: a byte the task's end keeps (lumart_task_done); the wait gives nothing.
        let unit = *of == Ty::Unit;
        let rc = if unit { "uint8_t".to_string() } else { self.c_type(of).ok_or_else(super::lost)? };
        let inf = if super::counted(&rc) || (!unit && self.holds_ref(of, &mut std::collections::HashSet::new())) {
            match self.in_fn(of) {
                Some(f) => f,
                None => return self.not_yet(fx, e, "wait on a pending of this type"),
            }
        } else {
            "0".to_string()
        };
        let p = fx.temp("lumart_pending", v);
        let out = fx.fresh(&rc);
        let st = fx.temp("int32_t", "0");
        park(fx);
        fx.emit(format!("{st} = {p}.ops != 0 ? {p}.ops->poll({p}.t, &{out}, {inf}, {file}, {line}) : 2;"));
        fx.emit(format!("if ({st} == 0) goto lm_park;"));
        fx.emit(format!("if ({st} == 3) {{ lm_st = 1; goto lm_out; }}"));
        fx.emit(format!("if ({st} == 2) {{ lm_f->h.stop = 1; goto lm_out; }}"));
        Ok(if unit { String::new() } else { out })
    }

    /// `wait N frames`, `wait N seconds`, `wait <task>`: parks until the frame, the world step, or the task's end.
    pub(super) fn wait_expr(&mut self, fx: &mut Fx, e: &Expr, amount: &Expr, unit: Option<ast::WaitUnit>) -> Ex {
        let m = fx.module;
        if !fx.task {
            return self.not_yet(fx, e, "wait here");
        }
        let (file, line) = (c_string(&fx.file), e.at.line);
        let from = self.ty(m, amount);
        let v = self.expr(fx, amount)?;
        if let (None, Ty::Pending(of)) = (unit, &from) {
            let of = (**of).clone();
            return self.wait_pending(fx, e, &v, &of);
        }
        let cond = match unit {
            Some(ast::WaitUnit::Frames) => {
                // An i32 (the checker's), widened by the cast.
                let n = v;
                let end = fx.temp("uint64_t", &format!("lumart_wait_frames((int64_t)({n}), {file}, {line})"));
                format!("lumart_now_frame < {end}")
            }
            Some(ast::WaitUnit::Seconds) => {
                let n = self.coerce(fx, &v, &from, &Ty::Float(Scalar::F32), amount.at)?;
                let end = fx.temp("uint64_t", &format!("lumart_wait_seconds((double)({n}), {file}, {line})"));
                format!("lumart_now_step < {end}")
            }
            None => match from {
                Ty::Task => {
                    let t = fx.temp("lumart_task", &v);
                    format!("lumart_task_alive({t})")
                }
                _ => return self.not_yet(fx, e, "wait on a pending result"),
            },
        };
        park(fx);
        fx.emit(format!("if ({cond}) goto lm_park;"));
        Ok(String::new())
    }

    /// `wait until c`: parks, then reads `c` on each frame until it holds.
    pub(super) fn wait_until(&mut self, fx: &mut Fx, e: &Expr, c: &Expr) -> Ex {
        if !fx.task {
            return self.not_yet(fx, e, "wait here");
        }
        park(fx);
        let v = self.expr(fx, c)?;
        fx.emit(format!("if (!({v})) goto lm_park;"));
        Ok(String::new())
    }
}

/// The step to resume at set, the frame kept, and its label: what follows runs on a later frame.
fn park(fx: &mut Fx) {
    fx.waits += 1;
    let k = fx.waits;
    fx.emit(format!("lm_f->h.step = {k}u;"));
    fx.emit("goto lm_park;".into());
    fx.lines.push(format!("lm_w{k}:;"));
}

impl Fx {
    /// The frame struct and the step function of a task's body (see the module's notes).
    pub(super) fn task_step(&self, frame: &str, step: &str, at: Pos) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "typedef struct {frame} {{\n    lumart_task_head h;");
        let names: Vec<String> = self.decls.iter().map(|d| decl_name(d)).collect();
        for d in &self.decls {
            let _ = writeln!(out, "{d}");
        }
        for b in &self.builders {
            let _ = writeln!(out, "    lumart_sb {b};");
        }
        let _ = writeln!(out, "}} {frame};\n");
        let _ = writeln!(out, "#line {} {}", at.line, c_string(&self.file));
        let _ = writeln!(out, "static int32_t {step}(void* lm_fp) {{\n    {frame}* lm_f = ({frame}*)lm_fp;\n    int32_t lm_st = 0;");
        for d in &self.decls {
            let d = d.trim_end_matches(';');
            let _ = writeln!(out, "{d} = {{0}};");
        }
        for b in &self.builders {
            let _ = writeln!(out, "    lumart_sb {b};\n    lumart_sb_init(&{b});");
        }
        if self.scratch {
            out.push_str("    const uint64_t lm_sm = lumart_scratch_mark();\n");
        }
        // In: every local and temporary (the parameters on the first run, zeros beside them), the builders once a
        // wait has kept them.
        for n in &names {
            let _ = writeln!(out, "    __builtin_memcpy(&{n}, &lm_f->{n}, sizeof {n});");
        }
        if !self.builders.is_empty() {
            out.push_str("    if (lm_f->h.step != 0) {\n");
            for b in &self.builders {
                let _ = writeln!(out, "        __builtin_memcpy(&{b}, &lm_f->{b}, sizeof {b});");
            }
            out.push_str("    }\n");
        }
        out.push_str("    if (lm_f->h.stop) goto lm_out;\n");
        if self.waits > 0 {
            out.push_str("    switch (lm_f->h.step) {\n");
            for k in 1..=self.waits {
                let _ = writeln!(out, "    case {k}u:\n        goto lm_w{k};");
            }
            out.push_str("    default:\n        break;\n    }\n");
        }
        for l in &self.lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str("    goto lm_out;\n");
        if self.waits > 0 {
            // Out: the frame holds every count now; the C locals are dropped without giving any back.
            out.push_str("lm_park:\n");
            for n in &names {
                let _ = writeln!(out, "    __builtin_memcpy(&lm_f->{n}, &{n}, sizeof {n});");
            }
            for b in &self.builders {
                let _ = writeln!(out, "    __builtin_memcpy(&lm_f->{b}, &{b}, sizeof {b});");
            }
            if self.scratch {
                out.push_str("    lumart_scratch_release(lm_sm);\n");
            }
            out.push_str("    return LUMART_PARKED;\n");
        }
        out.push_str("lm_out:\n");
        out.push_str(&self.way_out());
        out
    }
}

/// The variable a declaration (`ct name;`, `ct name[n];`) declares.
pub(super) fn decl_name(d: &str) -> String {
    let d = d.trim().trim_end_matches(';');
    let last = d.rsplit(' ').next().unwrap_or(d);
    last.split('[').next().unwrap_or(last).to_string()
}
