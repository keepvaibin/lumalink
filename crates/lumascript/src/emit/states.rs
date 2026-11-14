//! State machines (docs/modding/language/language.md, section 11.2; docs/modding/language/compiled-form.md, section 7.4): one machine per object of a mod's class, kept in the
//! class's data beside the object, run from the class's `control` (its override, or one the back end adds).
//!
//! A machine `M` is the struct `lm_..._M_m` (its state, the state asked for by `go`, whether the state's task is
//! parked, whether the machine was stopped, and one frame per waiting state, in a union). Each state's `enter` and
//! its own statements are one task-like step function (`_t<i>`: its locals in its frame between frames, a `wait`
//! parks it, as task.rs writes a task); its `update` (`_u<i>`) runs each frame after it and its `exit` (`_x<i>`) as
//! the state is left. `go S` asks for S and leaves the block it is in (from a function or hook of the class it only
//! asks); the machine's run function (`_run`) leaves the state (stopping its parked task, which gives back what it
//! holds, then `exit`) and enters S in the same frame, at most 16 times a frame (more wait for the next).
//!
//! A waiting state's frame may hold text, a container or a text builder: if the object goes while the state waits,
//! the class's `gone` calls the machine's (`_gone`), which gives back what the parked frame holds. The task is not
//! resumed and its defers do not run: no script code runs for an object that is gone.

use super::{c_string, counted, Fx, Gen, HookFx};
use crate::ast::{self, HookKind, ItemKind, Pos};
use crate::check::{ItemRef, Owner, Resolved};
use std::fmt::Write as _;

/// The most state changes one run of a machine makes; the rest are made on the next frame.
const GO_PER_FRAME: u32 = 16;

/// A machine being written: its item and its C names.
pub(super) struct Machine {
    pub(super) item: ItemRef,
    /// The machine's C prefix (`lm_<module>__<M>`).
    pub(super) c: String,
    /// Its field in the class's data.
    pub(super) field: String,
}

impl Gen<'_> {
    /// The machines (`states M for K`) of class `class`, in module and item order, each with its field in K's data.
    pub(super) fn machines_of(&self, class: ItemRef) -> Vec<Machine> {
        let mut out = Vec::new();
        for (m, source) in self.sources.iter().enumerate() {
            for (i, item) in source.module.items.iter().enumerate() {
                let ItemKind::States(s) = &item.kind else { continue };
                if self.checked.named.get(&(m, s.class.at)) == Some(&Resolved::Item(class)) {
                    let me = ItemRef { owner: Owner::Mod, module: m, item: i };
                    let k = out.len();
                    out.push(Machine { item: me, c: self.item_c(me, &s.name.name), field: format!("lm_mach_{k}") });
                }
            }
        }
        out
    }

    /// The C of one machine of class `class` (data type `data`): its struct and frames in lm.h's globals, its
    /// functions returned (written into the class's module), with the name of its `_gone` when a waiting state's frame
    /// holds counted values or builders.
    pub(super) fn machine(&mut self, class: ItemRef, data: &str, mach: &Machine) -> Option<(String, Option<String>)> {
        let ItemKind::States(s) = &self.item(mach.item).kind else { return None };
        let m = mach.item.module;
        let file = format!("{}.lum", self.sources[m].path);
        let mt = format!("{}_m", mach.c);
        let mut out = String::new();
        let mut frames = Vec::new();
        let mut tasks = Vec::new();
        let mut updates = Vec::new();
        let mut exits = Vec::new();
        // Each waiting state's frame parts that hold counts: (state, [lm_rel_ / lumart_sb_free line on lm_f]).
        let mut holding: Vec<(usize, Vec<String>)> = Vec::new();
        for (i, state) in s.states.iter().enumerate() {
            let new_fx = |gen: &Self| {
                let mut fx = Fx::new(&file, m);
                // `self` is the object of the class's `control` call; no proceed, skip or result here.
                fx.hook = Some(HookFx { kind: HookKind::After, result_c: None, texts: Vec::new(), outs: Vec::new(), name: String::new(), line: 0 });
                fx.class_data = Some(class);
                fx.machine = Some((mach.item, "lm_m".to_string()));
                let _ = gen;
                fx
            };
            // The task: `enter`, then the state's own statements.
            let enter = state.parts.iter().find_map(|p| match p {
                ast::StatePart::Enter(b) => Some(b),
                _ => None,
            });
            let stmts: Vec<&ast::Stmt> = state.parts.iter().filter_map(|p| match p {
                ast::StatePart::Stmt(st) => Some(&**st),
                _ => None,
            }).collect();
            if enter.is_some() || !stmts.is_empty() {
                let mut fx = new_fx(self);
                fx.task = true;
                if let Some(b) = enter {
                    self.block(&mut fx, b);
                }
                for st in stmts {
                    self.stmt(&mut fx, st);
                }
                self.out_defers(&mut fx);
                let frame = format!("{}_f{i}", mach.c);
                let step = format!("{}_t{i}", mach.c);
                if fx.waits > 0 {
                    // Its waits read lumart's frame and step, which the mod's frame handler keeps (time's on_frame).
                    self.tasks_used = true;
                    let mut gives = Vec::new();
                    for d in &fx.decls {
                        if let Some(ct) = decl_type(d).filter(|ct| counted(ct)) {
                            gives.push(format!("lm_rel_{ct}(&lm_f->{});", super::task::decl_name(d)));
                        }
                    }
                    for b in &fx.builders {
                        gives.push(format!("lumart_sb_free(&lm_f->{b});"));
                    }
                    if !gives.is_empty() {
                        holding.push((i, gives));
                    }
                }
                let head = format!("static int32_t {step}(LumaCall* call, {data}* lm_d, {mt}* lm_m, {frame}* lm_f)");
                let (def, text) = fx.machine_step(&frame, &head, state.name.at);
                self.globals.push(def);
                frames.push((i, frame));
                out.push_str(&text);
                tasks.push((i, step));
            }
            for (kind, list) in [("u", &mut updates), ("x", &mut exits)] {
                let b = state.parts.iter().find_map(|p| match (kind, p) {
                    ("u", ast::StatePart::Update(b)) | ("x", ast::StatePart::Exit(b)) => Some(b),
                    _ => None,
                });
                let Some(b) = b else { continue };
                let mut fx = new_fx(self);
                fx.emit("(void)lm_d;".into());
                fx.emit("(void)lm_m;".into());
                self.block(&mut fx, b);
                self.out_defers(&mut fx);
                let name = format!("{}_{kind}{i}", mach.c);
                let head = format!("static int32_t {name}(LumaCall* call, {data}* lm_d, {mt}* lm_m)");
                out.push_str(&fx.function(&head, b.at));
                list.push((i, name));
            }
        }
        // The machine's struct (lm.h): before the class's data, which holds it.
        let mut def = format!("typedef struct {mt} {{\n    int32_t state; /* -1 before its first run */\n    int32_t next;  /* the state `go` asked for, -1: none */\n    uint8_t running; /* the state's task is parked */\n    uint8_t stopped; /* `stop`, or a script error: the machine runs no more */\n    uint8_t started;\n    uint8_t reserved;\n");
        if frames.is_empty() {
            def.push_str("    uint64_t none;\n");
        } else {
            def.push_str("    union {\n");
            for (i, f) in &frames {
                let _ = writeln!(def, "        {f} s{i};");
            }
            def.push_str("    } f;\n");
        }
        let _ = write!(def, "}} {mt};");
        self.globals.push(def);
        // Its run: the parked task resumed, `update`, then each state asked for entered.
        let start = s.states.iter().position(|st| st.name.name == s.start.name).unwrap_or(0);
        let has_task = |i: usize| tasks.iter().any(|(k, _)| *k == i);
        let run = format!("{}_run", mach.c);
        let _ = writeln!(out, "static int32_t {run}(LumaCall* call, {data}* lm_d) {{\n    {mt}* lm_m = &lm_d->{};\n    int32_t rc = 0;\n    if (lm_m->stopped) return 0;\n    if (!lm_m->started) {{\n        lm_m->started = 1;\n        lm_m->state = {start};\n        lm_m->next = -1;\n        lm_m->running = {};\n    }}", mach.field, u8::from(has_task(start)));
        let _ = writeln!(out, "    for (uint32_t lm_go = 0; lm_go <= {GO_PER_FRAME}u; lm_go++) {{");
        // The task, from where it parked (or its start).
        out.push_str("        if (lm_m->running) {\n            switch (lm_m->state) {\n");
        for (i, step) in &tasks {
            let _ = writeln!(out, "            case {i}: rc = {step}(call, lm_d, lm_m, &lm_m->f.s{i}); break;");
        }
        out.push_str("            default: rc = 0; break;\n            }\n            if (rc != LUMART_PARKED) lm_m->running = 0;\n            if (rc == 1) lm_m->stopped = 1;\n        }\n");
        out.push_str("        if (!lm_m->stopped && lm_m->next < 0) {\n            switch (lm_m->state) {\n");
        for (i, f) in &updates {
            let _ = writeln!(out, "            case {i}: if ({f}(call, lm_d, lm_m) != 0) lm_m->stopped = 1; break;");
        }
        out.push_str("            default: break;\n            }\n        }\n");
        // Stopped: a parked task gives back what it holds; the machine ends where it is.
        let stop_task = |out: &mut String, indent: &str| {
            let _ = writeln!(out, "{indent}if (lm_m->running) {{\n{indent}    lm_m->running = 0;\n{indent}    switch (lm_m->state) {{");
            for (i, step) in &tasks {
                let _ = writeln!(out, "{indent}    case {i}: lm_m->f.s{i}.h.stop = 1; (void){step}(call, lm_d, lm_m, &lm_m->f.s{i}); break;");
            }
            let _ = writeln!(out, "{indent}    default: break;\n{indent}    }}\n{indent}}}");
        };
        out.push_str("        if (lm_m->stopped) {\n");
        stop_task(&mut out, "            ");
        out.push_str("            return 0;\n        }\n");
        let _ = writeln!(out, "        if (lm_m->next < 0 || lm_go == {GO_PER_FRAME}u) return 0;");
        // Leave: the task stopped, `exit`; enter the next: its frame zeroed, its task from its start.
        stop_task(&mut out, "        ");
        out.push_str("        switch (lm_m->state) {\n");
        for (i, f) in &exits {
            let _ = writeln!(out, "        case {i}: if ({f}(call, lm_d, lm_m) != 0) lm_m->stopped = 1; break;");
        }
        out.push_str("        default: break;\n        }\n        if (lm_m->stopped) return 0;\n");
        out.push_str("        lm_m->state = lm_m->next;\n        lm_m->next = -1;\n");
        if !frames.is_empty() {
            out.push_str("        __builtin_memset(&lm_m->f, 0, sizeof lm_m->f);\n");
        }
        out.push_str("        switch (lm_m->state) {\n");
        for (i, _) in &tasks {
            let _ = writeln!(out, "        case {i}:");
        }
        if !tasks.is_empty() {
            out.push_str("            lm_m->running = 1;\n            break;\n");
        }
        out.push_str("        default:\n            lm_m->running = 0;\n            break;\n        }\n    }\n    (void)rc;\n    return 0;\n}\n");
        // Its object gone: a parked frame (it parked once: step set, the frame its locals' owner) gives back what it
        // holds. A frame not parked holds nothing of its own (its task gave back at its way out).
        let gone = if holding.is_empty() {
            None
        } else {
            let gone = format!("{}_gone", mach.c);
            let _ = writeln!(out, "\nstatic void {gone}({data}* lm_d) {{\n    {mt}* lm_m = &lm_d->{};\n    if (!lm_m->running) return;\n    lm_m->running = 0;\n    switch (lm_m->state) {{", mach.field);
            for (i, gives) in &holding {
                let _ = writeln!(out, "    case {i}: {{\n        {}_f{i}* lm_f = &lm_m->f.s{i};\n        if (lm_f->h.step == 0) break;", mach.c);
                for g in gives {
                    let _ = writeln!(out, "        {g}");
                }
                out.push_str("        break;\n    }\n");
            }
            out.push_str("    default:\n        break;\n    }\n}\n");
            Some(gone)
        };
        Some((out, gone))
    }

    /// The C of `go M.S`'s or `is_state(M.S)`'s machine here: `lm_m` in M's own blocks, the object's in a function or
    /// hook of M's class; None elsewhere.
    pub(super) fn machine_here(&self, fx: &Fx, machine: ItemRef) -> Option<String> {
        if let Some((item, c)) = &fx.machine {
            if *item == machine {
                return Some(c.clone());
            }
        }
        let class = fx.class_data?;
        self.machines_of(class).into_iter().find(|mach| mach.item == machine).map(|mach| format!("(&lm_d->{})", mach.field))
    }

    /// A state's number in its machine.
    pub(super) fn state_number(&self, machine: ItemRef, state: &str) -> Option<usize> {
        let ItemKind::States(s) = &self.item(machine).kind else { return None };
        s.states.iter().position(|st| st.name.name == state)
    }

    /// `go S` / `go M.S`.
    pub(super) fn go_stmt(&mut self, fx: &mut Fx, at: Pos, path: &[ast::Ident]) {
        let m = fx.module;
        let Some(last) = path.last() else { return };
        let Some(Resolved::State { machine, state }) = self.checked.named.get(&(m, last.at)).cloned() else {
            self.say(m, at, "go to this state");
            return;
        };
        let (Some(place), Some(n)) = (self.machine_here(fx, machine), self.state_number(machine, &state)) else {
            self.say(m, at, "go from here");
            return;
        };
        fx.emit(format!("{place}->next = {n};"));
        // In the machine's own blocks `go` leaves the block (its defers run, its counts are given back).
        if fx.machine.as_ref().is_some_and(|(item, _)| *item == machine) {
            fx.emit("goto lm_out;".into());
        }
    }

    /// `is_state(S)` / `is_state(M.S)`: whether the machine is in that state.
    pub(super) fn is_state_value(&mut self, fx: &mut Fx, e: &ast::Expr, args: &[ast::Arg]) -> super::Ex {
        let m = fx.module;
        let at = match args.first().map(|a| &a.value.kind) {
            Some(ast::ExprKind::Name { name, .. }) => name.at,
            Some(ast::ExprKind::Member { name, .. }) => name.at,
            _ => return self.not_yet(fx, e, "is_state of this"),
        };
        let Some(Resolved::State { machine, state }) = self.checked.named.get(&(m, at)).cloned() else { return self.not_yet(fx, e, "is_state of this") };
        let (Some(place), Some(n)) = (self.machine_here(fx, machine), self.state_number(machine, &state)) else {
            return self.not_yet(fx, e, "is_state from here");
        };
        Ok(format!("((uint8_t)({place}->state == {n}))"))
    }
}

impl Fx {
    /// A state's task: its frame (for lm.h) and its step function, as [`Fx::task_step`] writes a task's, with the
    /// object's call, its data and its machine as parameters.
    fn machine_step(&self, frame: &str, head: &str, at: Pos) -> (String, String) {
        let mut def = String::new();
        let _ = writeln!(def, "typedef struct {frame} {{\n    lumart_task_head h;");
        for d in &self.decls {
            let _ = writeln!(def, "{d}");
        }
        for b in &self.builders {
            let _ = writeln!(def, "    lumart_sb {b};");
        }
        let _ = write!(def, "}} {frame};");
        let names: Vec<String> = self.decls.iter().map(|d| super::task::decl_name(d)).collect();
        let mut out = String::new();
        let _ = writeln!(out, "#line {} {}", at.line, c_string(&self.file));
        let _ = writeln!(out, "{head} {{\n    int32_t lm_st = 0;\n    (void)call;\n    (void)lm_d;\n    (void)lm_m;");
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
        (def, out)
    }
}

/// The C type a declaration (`ct name;`, `ct name[n];`) declares.
fn decl_type(d: &str) -> Option<String> {
    let d = d.trim().trim_end_matches(';');
    d.rsplit_once(' ').map(|(ct, _)| ct.to_string())
}
