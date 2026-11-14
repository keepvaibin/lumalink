//! Statements and control flow: blocks, bindings, assignment, `if`, `match`, `guard`, `while`, `for` over a range,
//! `return`, `skip`, `log`; and patterns (docs/modding/language/language.md, section 4).
//!
//! A condition list (`if a, let x = y`) is a flag: each condition is read only when the ones before it held, its
//! bindings made as it holds, and the arm's block runs inside; the next arm runs when the flag stayed down. No C
//! `switch`, so the script's `break` and `continue` are C's own.

use super::game::GamePlace;
use super::{lost, assign_line, c_string, components, expr_text, is_view, Ex, Fx, Gen, NotWritten};
use crate::ast::{self, ArmBody, BinOp, Block, Cond, Expr, ExprKind, Pattern, PatternKind, Pos, Stmt, StmtKind};
use crate::check::{Resolved, Ty};
use crate::game::GameTy;

/// Where a block's value goes: a C place and its type (None: the block gives none).
pub(super) type Target<'t> = Option<(&'t str, &'t Ty)>;

impl<'a> Gen<'a> {
    pub(super) fn block(&mut self, fx: &mut Fx, b: &Block) {
        self.block_value(fx, b, None);
    }

    /// A block; with a target, its last statement, when an expression, is the block's value.
    pub(super) fn block_value(&mut self, fx: &mut Fx, b: &Block, target: Target<'_>) {
        fx.emit("{".into());
        fx.depth += 1;
        fx.defer_scopes.push(Vec::new());
        let n = b.stmts.len();
        for (i, s) in b.stmts.iter().enumerate() {
            match (&s.kind, target) {
                (StmtKind::Expr(e), Some((place, t))) if i + 1 == n => {
                    fx.line_marker(s.at);
                    self.value_into(fx, e, place, t);
                }
                _ => self.stmt(fx, s),
            }
        }
        // The block's defers, the last reached first.
        let scope = fx.defer_scopes.pop().unwrap_or_default();
        for (flag, d) in scope.iter().rev() {
            self.run_defer(fx, flag, d);
        }
        fx.depth -= 1;
        fx.emit("}".into());
    }

    /// A deferred statement, when it was reached and has not run.
    pub(super) fn run_defer(&mut self, fx: &mut Fx, flag: &str, d: &Stmt) {
        fx.emit(format!("if ({flag}) {{"));
        fx.depth += 1;
        fx.emit(format!("{flag} = 0;"));
        self.stmt(fx, d);
        fx.depth -= 1;
        fx.emit("}".into());
    }

    /// Before a `break` or `continue`: the defers of the blocks it leaves, innermost first.
    fn leave_loop_blocks(&mut self, fx: &mut Fx) {
        let from = fx.loops.last().copied().unwrap_or(0);
        let scopes: Vec<Vec<(String, Stmt)>> = fx.defer_scopes[from..].to_vec();
        for scope in scopes.iter().rev() {
            for (flag, d) in scope.iter().rev() {
                self.run_defer(fx, flag, d);
            }
        }
    }

    /// The function's way out runs every defer still set (a `return`, a `skip`, a script error), the last reached
    /// first: written after `lm_out`.
    pub(super) fn out_defers(&mut self, fx: &mut Fx) {
        if fx.all_defers.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut fx.lines);
        // After lm_out a statement may have stopped part way, its temporaries still holding counts: the defers get
        // temporaries of their own.
        fx.free.clear();
        let depth = fx.depth;
        fx.depth = 1;
        let all = fx.all_defers.clone();
        for (flag, d) in all.iter().rev() {
            self.run_defer(fx, flag, d);
        }
        fx.tail = std::mem::replace(&mut fx.lines, lines);
        fx.depth = depth;
    }

    /// `place = e`, the value made to the place's type.
    pub(super) fn value_into(&mut self, fx: &mut Fx, e: &Expr, place: &str, t: &Ty) {
        let m = fx.module;
        // An `if` or a `match` writes its arms into the place directly.
        match &e.kind {
            ExprKind::If { arms, otherwise } => return self.if_chain(fx, arms, otherwise.as_ref(), Some((place, t))),
            ExprKind::Match { subject, arms } => return self.match_arms(fx, subject, arms, Some((place, t))),
            _ => {}
        }
        let from = self.ty(m, e);
        let Ok(v) = self.expr(fx, e) else { return };
        if matches!(from, Ty::Never) {
            return;
        }
        if let Ok(v) = self.coerce(fx, &v, &from, t, e.at) {
            let ct = self.c_type(t).unwrap_or_default();
            fx.set(place, &v, &ct);
        }
    }

    pub(super) fn stmt(&mut self, fx: &mut Fx, s: &Stmt) {
        let mark = fx.live.len();
        let made = fx.made.len();
        self.stmt_inner(fx, s);
        fx.release_since(mark);
        fx.recycle_since(made);
    }

    fn stmt_inner(&mut self, fx: &mut Fx, s: &Stmt) {
        let m = fx.module;
        super::writing(m, s.at);
        fx.line_marker(s.at);
        match &s.kind {
            StmtKind::Expr(e) => match &e.kind {
                ExprKind::If { arms, otherwise } => self.if_chain(fx, arms, otherwise.as_ref(), None),
                ExprKind::Match { subject, arms } => self.match_arms(fx, subject, arms, None),
                _ => {
                    if let Ok(x) = self.expr(fx, e) {
                        if !x.is_empty() {
                            fx.emit(format!("(void)({x});"));
                        }
                    }
                }
            },
            StmtKind::Binding(b) => self.binding(fx, b),
            StmtKind::Assign { target, op, value } => self.assign(fx, target, *op, value, s.at),
            StmtKind::While { cond, body } => {
                fx.emit("for (;;) {".into());
                fx.depth += 1;
                if let Ok(c) = self.expr(fx, cond) {
                    fx.emit(format!("if (!({c})) break;"));
                }
                fx.loops.push(fx.defer_scopes.len());
                self.block(fx, body);
                fx.loops.pop();
                fx.depth -= 1;
                fx.emit("}".into());
            }
            StmtKind::For { pattern, iter, body } => self.for_loop(fx, pattern, iter, body, s.at),
            StmtKind::Guard { conds, otherwise } => {
                // The conditions' bindings stay for the rest of the block; `otherwise` must leave (the checker said).
                let ok = fx.fresh("uint8_t");
                fx.emit(format!("{ok} = 0;"));
                let opened = self.conds(fx, conds);
                fx.emit(format!("{ok} = 1;"));
                self.close(fx, opened);
                fx.emit(format!("if (!{ok}) {{"));
                fx.depth += 1;
                self.block(fx, otherwise);
                fx.depth -= 1;
                fx.emit("}".into());
            }
            StmtKind::Break => {
                self.leave_loop_blocks(fx);
                fx.emit("break;".into());
            }
            StmtKind::Continue => {
                self.leave_loop_blocks(fx);
                fx.emit("continue;".into());
            }
            StmtKind::Defer(d) => {
                let flag = fx.fresh_kept("uint8_t");
                fx.emit(format!("{flag} = 1;"));
                if let Some(scope) = fx.defer_scopes.last_mut() {
                    scope.push((flag.clone(), (**d).clone()));
                }
                fx.all_defers.push((flag, (**d).clone()));
            }
            StmtKind::Return(v) if !fx.inline_ret.is_empty() => self.inline_return(fx, v.as_ref()),
            StmtKind::Return(None) => fx.emit("goto lm_out;".into()),
            StmtKind::Return(Some(e)) if fx.task_ret.is_some() => self.task_return(fx, e, s.at),
            StmtKind::Return(Some(e)) => match fx.ret.clone() {
                Some((_, t)) => {
                    self.value_into(fx, e, "(*lm_ret)", &t);
                    // An override's value is the call's result (class.rs).
                    if let Some(rc) = fx.override_rc.clone() {
                        fx.emit(format!("LUMA_RESULT(call, {rc}) = *lm_ret;"));
                    }
                    fx.emit("goto lm_out;".into());
                }
                None => self.say(m, e.at, "return with a value here"),
            },
            // A log line is made in the scratch (docs/modding/language/language.md, section 6.4) and given back once the core has it (LumaCore.log
            // copies); an empty line is "", never null. Its text is any text value: a literal with its
            // interpolations written in place, else the value's text (`log text "Key"`, `log reply`).
            StmtKind::Log { fact: None, value } => {
                let (b, mark) = fx.scratch_builder();
                if self.value_text_into(fx, &b, value, false).is_ok() {
                    fx.fail_if(&format!("lumart_sb_done(&{b}, {}, {})", c_string(&fx.file), s.at.line));
                    fx.emit(format!("lumart_log(lumart_sb_c(&{b}));"));
                    fx.emit(format!("lumart_scratch_release({mark});"));
                }
            }
            StmtKind::Fail(value) => {
                // The words made in the scratch, reported (S0921: the text itself; inside an export the caller's
                // line, lumart_caller), then the way out every script error takes.
                let (b, mark) = fx.scratch_builder();
                if self.value_text_into(fx, &b, value, false).is_ok() {
                    fx.fail_if(&format!("lumart_sb_done(&{b}, {}, {})", c_string(&fx.file), s.at.line));
                    fx.emit(format!(
                        "{{ const char* lm_words[1] = {{ lumart_sb_c(&{b}) }}; lumart_raise({}, {}, 921, 1, lm_words); }}",
                        c_string(&fx.file),
                        s.at.line
                    ));
                    fx.emit(format!("lumart_scratch_release({mark});"));
                    fx.emit("lm_st = 1; goto lm_out;".into());
                }
            }
            StmtKind::Log { fact: Some(key), value } => {
                // A fact for the harness: the log line `fact <key> <JSON>` (docs/modding/lumalink/test-harness.md, section 4).
                let (b, mark) = fx.scratch_builder();
                fx.emit(format!("lumart_sb_str(&{b}, \"fact \");"));
                if self.text_into(fx, &b, key).is_err() {
                    return;
                }
                fx.emit(format!("lumart_sb_str(&{b}, \" \");"));
                if self.json_into(fx, &b, value).is_ok() {
                    fx.fail_if(&format!("lumart_sb_done(&{b}, {}, {})", c_string(&fx.file), s.at.line));
                    fx.emit(format!("lumart_log(lumart_sb_c(&{b}));"));
                    fx.emit(format!("lumart_scratch_release({mark});"));
                }
            }
            StmtKind::Skip(v) => {
                let Some(hook) = fx.hook.clone() else { return };
                if let (Some(e), Some(rc)) = (v, &hook.result_c) {
                    let from = self.ty(m, e);
                    let Ok(x) = self.expr(fx, e) else { return };
                    let _ = from;
                    fx.emit(format!("LUMA_RESULT(call, {rc}) = {x};"));
                }
                fx.emit("*lm_skip = 1;".into());
                fx.emit("goto lm_out;".into());
            }
            // `expect` belongs to tests, which `lumalink test` builds (4.8); in a mod's code it is a check.
            StmtKind::Expect(_) => self.say(m, s.at, "expect outside a test"),
            // `stop` (11.1): the task's way out (its defers, its counts); in an `on start`, the handler's end.
            // In a machine (docs/modding/language/language.md, section 11.2): the machine stops where it is.
            StmtKind::Stop if fx.task || fx.machine.is_some() => {
                if let Some((_, mp)) = fx.machine.clone() {
                    fx.emit(format!("{mp}->stopped = 1;"));
                }
                fx.emit("goto lm_out;".into());
            }
            StmtKind::Go(path) => self.go_stmt(fx, s.at, path),
            other => self.say(m, s.at, stmt_name(other)),
        }
    }

    fn binding(&mut self, fx: &mut Fx, b: &ast::Binding) {
        let m = fx.module;
        if let PatternKind::Path { path, args: None, fields: None } = &b.pattern.kind {
            if path.len() == 1 {
                if let Some(Resolved::Local(id)) = self.checked.named.get(&(m, path[0].at)) {
                    let t = self.checked.locals[*id as usize].ty.clone();
                    let Some(place) = self.declare_local(fx, *id, b.at) else { return };
                    match &b.value {
                        Some(value) => self.value_into(fx, value, &place, &t),
                        None => {
                            // `var x: T`: its zero, made again each time the statement runs (a loop's body).
                            let ct = self.c_type(&t).unwrap_or_default();
                            fx.set(&place, &format!("({ct}){{0}}"), &ct);
                        }
                    }
                    return;
                }
            }
        }
        // `let (a, b) = t`, `let P { x, y } = p`: the value once, then the pattern binds (it cannot fail: docs/modding/language/language.md, section 4).
        let Some(value) = &b.value else {
            self.say(m, b.at, "a pattern without a value");
            return;
        };
        let t = self.ty(m, value);
        let Some(ct) = self.c_type(&t) else {
            let what = format!("a value of type {}", self.ty_text(&t));
            self.say(m, b.at, &what);
            return;
        };
        let Ok(v) = self.expr(fx, value) else { return };
        let subject = fx.temp(&ct, &v);
        let Ok((_, binds)) = self.pattern(fx, &b.pattern, &subject, &t) else { return };
        for line in binds {
            fx.emit(line);
        }
    }

    /// A local's C place, declared at the function's top the first time.
    pub(super) fn declare_local(&mut self, fx: &mut Fx, id: crate::check::LocalId, at: Pos) -> Option<String> {
        if fx.declared.insert(id) {
            let t = self.checked.locals[id as usize].ty.clone();
            let Some(ct) = self.c_type(&t) else {
                let what = format!("a local of type {}", self.ty_text(&t));
                self.say(fx.module, at, &what);
                return None;
            };
            fx.decl(format!("{ct} l{id};"));
        }
        Some(fx.local(id))
    }

    fn assign(&mut self, fx: &mut Fx, target: &Expr, op: ast::AssignOp, value: &Expr, at: Pos) {
        let m = fx.module;
        let lt = self.ty(m, target);
        let mark = fx.rechecks.len();
        self.assign_at(fx, target, op, value, at, &lt, mark);
        fx.rechecks.truncate(mark);
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_at(&mut self, fx: &mut Fx, target: &Expr, op: ast::AssignOp, value: &Expr, at: Pos, lt: &Ty, mark: usize) {
        let m = fx.module;
        let lt = lt.clone();
        if let (ast::AssignOp::Set, ExprKind::Index { base, index }) = (op, &target.kind) {
            if let Ty::Map(k, v) = self.ty(m, base) {
                self.map_set(fx, target, base, index, &k, &v, value, mark);
                return;
            }
        }
        // A hook's `const char *` result given a text: the text's kept copy (docs/modding/language/compiled-form.md: the game may hold the
        // pointer), null for none; a NUL byte inside is S0918.
        if let (ast::AssignOp::Set, ExprKind::Result) = (op, &target.kind) {
            if lt == Ty::Text || matches!(&lt, Ty::Opt(x) if **x == Ty::Text) {
                let Some(rc) = fx.hook.as_ref().and_then(|h| h.result_c.clone()) else { return };
                let want = Ty::Opt(Box::new(Ty::Text));
                let from = self.ty(m, value);
                let Ok(v) = self.expr(fx, value) else { return };
                let Ok(v) = self.coerce(fx, &v, &from, &want, value.at) else { return };
                let Some(ct) = self.c_type(&want) else { return };
                if !matches!(strip(value).kind, ExprKind::None | ExprKind::Str(_)) {
                    self.kept.push((fx.file.clone(), at.line, "result".to_string()));
                }
                let o = fx.temp(&ct, &v);
                // A `const char16_t *` result: the text's UTF-16 kept copy, made once.
                let (kc, keep) = if rc == "const uint16_t*" { ("const uint16_t*", "lumart_text_keep16") } else { ("const char*", "lumart_text_keep") };
                let k = fx.fresh(kc);
                fx.emit(format!("{k} = 0;"));
                fx.emit(format!("if ({o}.some) {{"));
                fx.depth += 1;
                fx.emit(format!("{k} = {keep}({o}.value, {}, {}, \"result\", \"result\");", c_string(&fx.file), at.line));
                fx.fail_if(&format!("{k} == 0"));
                fx.depth -= 1;
                fx.emit("}".into());
                fx.emit(format!("LUMA_RESULT(call, {rc}) = ({rc}){k};"));
                return;
            }
        }
        // A game place is found once (its object read once); a bit-field is written through LUMA_BITS_SET. A field
        // of a game struct copy is written in the variable's own block (game.rs copy_address).
        fx.writing = true;
        let game = self.game_place_of(fx, target);
        let place = match game {
            Some(Err(NotWritten)) => return,
            Some(Ok(GamePlace::Bits { object, slot, width, of })) => {
                let bits = GamePlace::Bits { object: object.clone(), slot: slot.clone(), width, of };
                let Ok(v) = self.assigned_value(fx, target, op, value, &lt, bits, at) else { return };
                self.bits_write(fx, target, &object, &slot, width, of, &v);
                return;
            }
            Some(Ok(GamePlace::Address(_))) => {
                self.say(m, target.at, "a whole game object written");
                return;
            }
            Some(Ok(GamePlace::Array { .. })) => {
                self.say(m, target.at, "a whole game array written");
                return;
            }
            Some(Ok(GamePlace::Lvalue(l))) => l,
            None => match self.place(fx, target) {
                Ok(p) => p,
                Err(NotWritten) => return,
            },
        };
        let ct = self.c_type(&lt).unwrap_or_default();
        if op == ast::AssignOp::Set {
            if fx.rechecks.len() == mark {
                self.value_into(fx, value, &place, &lt);
                return;
            }
            // Inside a container: the value made first, the place's checks again, then the store.
            let r = fx.fresh(&ct);
            self.value_into(fx, value, &r, &lt);
            fx.recheck_since(mark);
            fx.set(&place, &r, &ct);
            return;
        }
        let Ok(x) = self.assigned_value(fx, target, op, value, &lt, GamePlace::Lvalue(place.clone()), at) else { return };
        fx.recheck_since(mark);
        fx.set(&place, &x, &ct);
    }

    /// `m[k] = v`: the map's place, the key, the value, in that order; the key added or its value replaced.
    #[allow(clippy::too_many_arguments)]
    fn map_set(&mut self, fx: &mut Fx, target: &Expr, base: &Expr, index: &Expr, k: &Ty, v: &Ty, value: &Expr, mark: usize) {
        let m = fx.module;
        let (file, line) = (c_string(&fx.file), target.at.line);
        let (Some(kc), Some(vc), Some(kd), Some(vd)) = (self.c_type(k), self.c_type(v), self.desc(k), self.desc(v)) else {
            let _ = lost();
            return;
        };
        let Ok(b) = self.place(fx, base) else { return };
        let Ok(kv) = self.expr(fx, index) else { return };
        let kf = self.ty(m, index);
        let Ok(kv) = self.coerce(fx, &kv, &kf, k, index.at) else { return };
        let kt = fx.temp(&kc, &kv);
        let vt = fx.fresh(&vc);
        self.value_into(fx, value, &vt, v);
        fx.recheck_since(mark);
        let added = fx.fresh("uint8_t");
        fx.fail_if(&format!("lumart_map_put(&{b}, {kd}, {vd}, &{kt}, &{vt}, &{added}, {file}, {line})"));
    }

    /// The value `target op= value` gives (`=`: the value, made to the target's type), the target read from `place`.
    #[allow(clippy::too_many_arguments)]
    fn assigned_value(&mut self, fx: &mut Fx, target: &Expr, op: ast::AssignOp, value: &Expr, lt: &Ty, place: GamePlace, at: Pos) -> Ex {
        let m = fx.module;
        if op == ast::AssignOp::Set {
            let ct = self.c_type(lt).ok_or_else(lost)?;
            let r = fx.fresh(&ct);
            self.value_into(fx, value, &r, lt);
            return Ok(r);
        }
        let old = self.game_read(fx, place)?;
        // The old value read before the value: a call in it may change the place (`n += bump()`).
        let old = match self.c_type(lt) {
            Some(ct) => fx.temp(&ct, &old),
            None => old,
        };
        let v = self.expr(fx, value)?;
        let bin = match op {
            ast::AssignOp::Add => BinOp::Add,
            ast::AssignOp::Sub => BinOp::Sub,
            ast::AssignOp::Mul => BinOp::Mul,
            ast::AssignOp::Div => BinOp::Div,
            ast::AssignOp::Rem => BinOp::Rem,
            ast::AssignOp::And => BinOp::BitAnd,
            ast::AssignOp::Or => BinOp::BitOr,
            ast::AssignOp::Xor => BinOp::BitXor,
            ast::AssignOp::Shl => BinOp::Shl,
            ast::AssignOp::Shr => BinOp::Shr,
            ast::AssignOp::Set => unreachable!("handled above"),
        };
        let rt = self.ty(m, value);
        let source = format!("{} {}= {}", expr_text(target), super::binop_text(bin), expr_text(value));
        self.binary_values(fx, bin, &old, lt, &v, &rt, lt, &source, at)
    }

    /// A place written to: a local, a module value, a component or field of one, a hook's argument, `result`.
    pub(super) fn place(&mut self, fx: &mut Fx, e: &Expr) -> Ex {
        let m = fx.module;
        fx.writing = true;
        match self.game_place_of(fx, e) {
            Some(Ok(GamePlace::Lvalue(l))) => return Ok(l),
            Some(Ok(_)) => return self.not_yet(fx, e, "this game place as a whole"),
            Some(Err(NotWritten)) => return Err(lost()),
            None => {}
        }
        if let Some(p) = self.class_field_place(fx, e) {
            return p;
        }
        match &e.kind {
            ExprKind::Name { .. } => match self.resolved(m, e) {
                Some(Resolved::Local(id)) => Ok(fx.local(*id)),
                Some(Resolved::Item(item)) => Ok(self.item_value(*item)),
                _ => self.not_yet(fx, e, "a place of this kind"),
            },
            ExprKind::Result => match fx.hook.as_ref().and_then(|h| h.result_c.clone()) {
                Some(rc) => Ok(format!("LUMA_RESULT(call, {rc})")),
                None => self.not_yet(fx, e, "result of a function with none"),
            },
            ExprKind::Member { base, optional: false, .. } => match self.resolved(m, e) {
                Some(Resolved::Component(i)) => {
                    let Ty::Value(v) = self.ty(m, base) else { return self.not_yet(fx, e, "a component of this value") };
                    let b = self.place(fx, base)?;
                    Ok(format!("{b}.{}", components(v)[*i]))
                }
                Some(Resolved::Member { name, .. }) if matches!(self.ty(m, base), Ty::Struct(_)) => {
                    let b = self.place(fx, base)?;
                    Ok(format!("{b}.{name}"))
                }
                Some(Resolved::TupleIndex(i)) => {
                    let b = self.place(fx, base)?;
                    Ok(format!("{b}._{i}"))
                }
                // A number field of a service's struct (the checker's rule): the header's field, as the script has it.
                Some(Resolved::ServiceField { field, .. }) => {
                    let field = field.clone();
                    let b = self.place(fx, base)?;
                    Ok(format!("{b}.{field}"))
                }
                _ => self.not_yet(fx, e, "a member place of this kind"),
            },
            ExprKind::Paren(x) => self.place(fx, x),
            // `xs[i]` / `m[k]` written (or a field of one): the container made this variable's own (copy on write),
            // then the element's storage; an index outside is S0902, a missing key S0907.
            ExprKind::Index { base, index } => {
                let bt = self.ty(m, base);
                let (file, line) = (c_string(&fx.file), e.at.line);
                match &bt {
                    // `b[i] = v` on bytes: the block made this variable's own, then the byte.
                    Ty::Bytes => {
                        let b = self.place(fx, base)?;
                        let i = self.expr(fx, index)?;
                        let i = fx.temp("int64_t", &format!("(int64_t)({i})"));
                        let from = fx.lines.len();
                        fx.fail_if(&format!("(uint64_t){i} >= lumart_list_n({b}) && lumart_index_error({i}, lumart_list_n({b}), {file}, {line})"));
                        fx.fail_if(&format!("lumart_list_own(&{b}, &lumart_u8, 0u, {file}, {line})"));
                        let checks = fx.lines[from..].to_vec();
                        fx.rechecks.push(checks);
                        Ok(format!("lumart_bytes_p({b})[(uint64_t){i}]"))
                    }
                    Ty::List(x) => {
                        let x = (**x).clone();
                        let ec = self.c_type(&x).ok_or_else(lost)?;
                        let d = self.desc(&x).ok_or_else(lost)?;
                        let b = self.place(fx, base)?;
                        let i = self.expr(fx, index)?;
                        let i = fx.temp("int64_t", &format!("(int64_t)({i})"));
                        let from = fx.lines.len();
                        fx.fail_if(&format!("(uint64_t){i} >= lumart_list_n({b}) && lumart_index_error({i}, lumart_list_n({b}), {file}, {line})"));
                        fx.fail_if(&format!("lumart_list_own(&{b}, {d}, 0u, {file}, {line})"));
                        let checks = fx.lines[from..].to_vec();
                        fx.rechecks.push(checks);
                        Ok(format!("(*({ec}*)lumart_list_at({b}, (uint64_t){i}))"))
                    }
                    Ty::Map(k, v) => {
                        let (k, v) = ((**k).clone(), (**v).clone());
                        let kd = self.desc(&k).ok_or_else(lost)?;
                        let vd = self.desc(&v).ok_or_else(lost)?;
                        let vc = self.c_type(&v).ok_or_else(lost)?;
                        let b = self.place(fx, base)?;
                        let kt = self.map_key(fx, &k, index)?;
                        let from = fx.lines.len();
                        let at = self.map_find_at(fx, e, &b, &k, &kt)?;
                        fx.fail_if(&format!("lumart_map_own(&{b}, {kd}, {vd}, 0u, {file}, {line})"));
                        let checks = fx.lines[from..].to_vec();
                        fx.rechecks.push(checks);
                        Ok(format!("(*({vc}*)lumart_map_val({b}, {at}))"))
                    }
                    _ => self.not_yet(fx, e, "this place"),
                }
            }
            ExprKind::SelfValue => match &fx.self_c {
                Some(c) => Ok(c.clone()),
                None => self.not_yet(fx, e, "self as a place"),
            },
            _ => self.not_yet(fx, e, "this place"),
        }
    }

    // ------------------------------------------------------------ if, match, guard

    /// Opens the conditions in order; each `if` opened is closed by [`Gen::close`]. A condition that did not write is
    /// said and taken as false.
    fn conds(&mut self, fx: &mut Fx, conds: &[Cond]) -> usize {
        let m = fx.module;
        let mut opened = 0;
        for c in conds {
            match &c.pattern {
                None => {
                    let v = self.expr(fx, &c.value).unwrap_or_else(|NotWritten| "0".into());
                    fx.emit(format!("if ({v}) {{"));
                }
                Some(p) => {
                    let t = self.ty(m, &c.value);
                    let Some(ct) = self.c_type(&t) else {
                        let what = format!("if let on {}", self.ty_text(&t));
                        self.say(m, c.value.at, &what);
                        fx.emit("if (0) {".into());
                        fx.depth += 1;
                        opened += 1;
                        continue;
                    };
                    let v = self.expr(fx, &c.value).unwrap_or_else(|NotWritten| format!("({ct}){{0}}"));
                    let subject = fx.temp(&ct, &v);
                    // `if let x = opt`: the binding's own type tells whether it takes the value inside.
                    let (test, binds) = self.pattern(fx, p, &subject, &t).unwrap_or_else(|NotWritten| ("0".into(), Vec::new()));
                    fx.emit(format!("if ({test}) {{"));
                    fx.depth += 1;
                    for b in binds {
                        fx.emit(b);
                    }
                    fx.depth -= 1;
                }
            }
            fx.depth += 1;
            opened += 1;
        }
        opened
    }

    fn close(&mut self, fx: &mut Fx, opened: usize) {
        for _ in 0..opened {
            fx.depth -= 1;
            fx.emit("}".into());
        }
    }

    pub(super) fn if_chain(&mut self, fx: &mut Fx, arms: &[(Vec<Cond>, Block)], otherwise: Option<&Block>, target: Target<'_>) {
        let done = fx.fresh("uint8_t");
        fx.emit(format!("{done} = 0;"));
        for (i, (conds, body)) in arms.iter().enumerate() {
            if i > 0 {
                fx.emit(format!("if (!{done}) {{"));
                fx.depth += 1;
            }
            let opened = self.conds(fx, conds);
            fx.emit(format!("{done} = 1;"));
            self.block_value(fx, body, target);
            self.close(fx, opened);
            if i > 0 {
                fx.depth -= 1;
                fx.emit("}".into());
            }
        }
        if let Some(b) = otherwise {
            fx.emit(format!("if (!{done}) {{"));
            fx.depth += 1;
            self.block_value(fx, b, target);
            fx.depth -= 1;
            fx.emit("}".into());
        }
    }

    pub(super) fn match_arms(&mut self, fx: &mut Fx, subject: &Expr, arms: &[ast::Arm], target: Target<'_>) {
        let m = fx.module;
        let t = self.ty(m, subject);
        let Some(ct) = self.c_type(&t) else {
            let what = format!("match on {}", self.ty_text(&t));
            self.say(m, subject.at, &what);
            return;
        };
        let Ok(v) = self.expr(fx, subject) else { return };
        let s = fx.temp(&ct, &v);
        let done = fx.fresh("uint8_t");
        fx.emit(format!("{done} = 0;"));
        for arm in arms {
            fx.emit(format!("if (!{done}) {{"));
            fx.depth += 1;
            let (test, binds) = match &arm.pattern {
                None => ("1".to_string(), Vec::new()),
                Some(p) => match self.pattern(fx, p, &s, &t) {
                    Ok(x) => x,
                    Err(NotWritten) => ("0".into(), Vec::new()),
                },
            };
            fx.emit(format!("if ({test}) {{"));
            fx.depth += 1;
            fx.emit(format!("{done} = 1;"));
            for b in binds {
                fx.emit(b);
            }
            match &arm.body {
                ArmBody::Block(b) => self.block_value(fx, b, target),
                ArmBody::Stmt(st) => match (&st.kind, target) {
                    (StmtKind::Expr(e), Some((place, pt))) => {
                        fx.line_marker(st.at);
                        self.value_into(fx, e, place, pt);
                    }
                    _ => self.stmt(fx, st),
                },
            }
            fx.depth -= 1;
            fx.emit("}".into());
            fx.depth -= 1;
            fx.emit("}".into());
        }
    }

    // ------------------------------------------------------------ patterns (docs/modding/language/language.md, section 4)

    /// A pattern against `subject` (a C place of type `t`, read only): the C test (pure, it reads only the subject)
    /// and the bindings to make when it holds.
    pub(super) fn pattern(&mut self, fx: &mut Fx, p: &Pattern, subject: &str, t: &Ty) -> Result<(String, Vec<String>), NotWritten> {
        let m = fx.module;
        match &p.kind {
            PatternKind::Tuple(ps) => {
                let Ty::Tuple(ts) = t else { return self.say_at(fx, p.at, "a tuple pattern on this value") };
                let mut tests = Vec::new();
                let mut binds = Vec::new();
                for (i, (sp, st)) in ps.iter().zip(ts.iter()).enumerate() {
                    let (tst, b) = self.pattern(fx, sp, &format!("{subject}._{i}"), st)?;
                    tests.push(tst);
                    binds.extend(b);
                }
                Ok((all(tests), binds))
            }
            PatternKind::Literal(e) => {
                let lt = self.ty(m, e);
                match (&e.kind, t) {
                    (ExprKind::None, Ty::Opt(x)) if is_view(x) => Ok((format!("{subject} == 0"), Vec::new())),
                    (ExprKind::None, Ty::Opt(_)) => Ok((format!("!{subject}.some"), Vec::new())),
                    // Text by its bytes (a literal is a static block: no count taken).
                    (_, Ty::Text) => {
                        let v = self.literal(fx, e, &lt)?;
                        Ok((format!("lumart_text_eq({subject}, {v})"), Vec::new()))
                    }
                    (_, Ty::Opt(x)) if **x == Ty::Text => {
                        let v = self.literal(fx, e, &lt)?;
                        Ok((format!("({subject}.some && lumart_text_eq({subject}.value, {v}))"), Vec::new()))
                    }
                    (_, Ty::Opt(x)) if !is_view(x) => {
                        let v = self.literal(fx, e, &lt)?;
                        Ok((format!("({subject}.some && {subject}.value == {v})"), Vec::new()))
                    }
                    (_, Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Game(GameTy::Enum(_))) => {
                        let v = self.literal(fx, e, &lt)?;
                        Ok((format!("{subject} == {v}"), Vec::new()))
                    }
                    _ => {
                        let what = format!("a literal pattern on {}", self.ty_text(t));
                        self.say_at(fx, p.at, &what)
                    }
                }
            }
            PatternKind::Path { path, args, fields } => {
                let last = &path[path.len() - 1];
                match self.checked.named.get(&(m, last.at)) {
                    // A name binds the value: `x` against `T?` binds the value inside when `x` is a `T`.
                    Some(Resolved::Local(id)) => {
                        let id = *id;
                        let lt = self.checked.locals[id as usize].ty.clone();
                        let place = self.declare_local(fx, id, p.at).ok_or_else(lost)?;
                        let lc = self.c_type(&lt).unwrap_or_default();
                        // A plain game record held in the game (a view of it, through `?.` too) bound to a name: the
                        // name holds the script's copy, as coerce makes it for any other store, never the
                        // game's address as a block.
                        let viewed = match t {
                            Ty::Opt(x) => (**x).clone(),
                            other => other.clone(),
                        };
                        if let (Ty::Game(GameTy::View { record, .. }), Ty::Game(GameTy::Struct(r))) = (&viewed, &lt) {
                            if record == r {
                                let size = self.size_slot(r);
                                let copy = format!("if (lumart_record_copy(&{place}, (const void*)({subject}), {size}, {}, {})) {{ lm_st = 1; goto lm_out; }}", c_string(&fx.file), p.at.line);
                                let test = if matches!(t, Ty::Opt(_)) { format!("{subject} != 0") } else { "1".to_string() };
                                return Ok((test, vec![copy]));
                            }
                        }
                        match t {
                            Ty::Opt(x) if lt == **x && is_view(x) => Ok((format!("{subject} != 0"), vec![assign_line(&place, subject, &lc)])),
                            Ty::Opt(x) if lt == **x => Ok((format!("{subject}.some"), vec![assign_line(&place, &format!("{subject}.value"), &lc)])),
                            _ => Ok(("1".into(), vec![assign_line(&place, subject, &lc)])),
                        }
                    }
                    Some(Resolved::Case { item, case }) => {
                        let (item, case) = (*item, case.clone());
                        let tag = self.case_tag(item, &case).ok_or_else(lost)?;
                        let mut tests = vec![format!("{subject}.tag == {tag}u")];
                        let mut binds = Vec::new();
                        let crate::ast::ItemKind::Enum(decl) = &self.item(item).kind else { return Err(lost()) };
                        let c = &decl.cases[tag];
                        if let Some(args) = args {
                            for (sp, f) in args.iter().zip(c.fields.iter()) {
                                let ft = self.declared_ty(item, &format!("{case}.{}", f.name.name));
                                let (tst, b) = self.pattern(fx, sp, &format!("{subject}.u.{case}.{}", f.name.name), &ft)?;
                                tests.push(tst);
                                binds.extend(b);
                            }
                        }
                        if let Some(names) = fields {
                            for n in names {
                                let Some(Resolved::Local(id)) = self.checked.named.get(&(m, n.at)) else { continue };
                                let id = *id;
                                let place = self.declare_local(fx, id, n.at).ok_or_else(lost)?;
                                let lc = self.c_type(&self.checked.locals[id as usize].ty.clone()).unwrap_or_default();
                                binds.push(assign_line(&place, &format!("{subject}.u.{case}.{}", n.name), &lc));
                            }
                        }
                        Ok((all(tests), binds))
                    }
                    Some(Resolved::GameEnumValue { number, .. } | Resolved::ServiceCase { number, .. }) => Ok((format!("{subject} == (int32_t){number}"), Vec::new())),
                    // A stream event's case: its kind's number, its fields the members of lm_se_<service>.
                    Some(Resolved::StreamCase { service, kind, number }) => {
                        let (service, kind, number) = (service.clone(), kind.clone(), *number);
                        let decl = self.game.services.faces.get(&service).and_then(|f| f.stream.get(&kind)).map(|k| k.fields.clone()).ok_or_else(lost)?;
                        let mut tests = vec![format!("{subject}.kind == (int32_t){number}")];
                        let mut binds = Vec::new();
                        if let Some(args) = args {
                            for (sp, (f, st)) in args.iter().zip(decl.iter()) {
                                let ft = super::service::service_sty(&service, st);
                                let (tst, b) = self.pattern(fx, sp, &format!("{subject}.{f}"), &ft)?;
                                tests.push(tst);
                                binds.extend(b);
                            }
                        }
                        if let Some(names) = fields {
                            for n in names {
                                let Some(Resolved::Local(id)) = self.checked.named.get(&(m, n.at)) else { continue };
                                let id = *id;
                                let place = self.declare_local(fx, id, n.at).ok_or_else(lost)?;
                                let lc = self.c_type(&self.checked.locals[id as usize].ty.clone()).unwrap_or_default();
                                binds.push(assign_line(&place, &format!("{subject}.{}", n.name), &lc));
                            }
                        }
                        Ok((all(tests), binds))
                    }
                    _ => self.say_at(fx, p.at, "this pattern"),
                }
            }
        }
    }

    /// A literal of a pattern, as C.
    fn literal(&mut self, fx: &mut Fx, e: &Expr, t: &Ty) -> Ex {
        match &e.kind {
            ExprKind::Neg(x) => {
                let v = self.literal(fx, x, t)?;
                Ok(format!("(-{v})"))
            }
            _ => self.expr(fx, e),
        }
    }

    // ------------------------------------------------------------ for

    /// `for x in a..b` and `a..=b` over integers: the bounds read once; the loop variable a copy each turn.
    fn for_loop(&mut self, fx: &mut Fx, pattern: &Pattern, iter: &Expr, body: &Block, at: Pos) {
        let m = fx.module;
        let it = self.ty(m, iter);
        if matches!(it, Ty::List(_) | Ty::Set(_) | Ty::Map(..)) {
            self.for_container(fx, pattern, iter, &it, body);
            return;
        }
        let (ExprKind::Binary { op: op @ (BinOp::Range | BinOp::RangeIncl), left, right }, Ty::Range(elem)) = (&strip(iter).kind, self.ty(m, iter)) else {
            let what = format!("for over {}", self.ty_text(&self.ty(m, iter)));
            self.say(m, at, &what);
            return;
        };
        let Some(ct) = self.c_type(&elem) else {
            let _ = lost();
            return;
        };
        let Ok(a) = self.expr(fx, left) else { return };
        let i = fx.temp(&ct, &a);
        let Ok(b) = self.expr(fx, right) else { return };
        let end = fx.temp(&ct, &b);
        let bind = |g: &mut Gen<'a>, fx: &mut Fx| -> Option<String> {
            match &pattern.kind {
                PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => match g.checked.named.get(&(m, path[0].at)) {
                    Some(Resolved::Local(id)) => g.declare_local(fx, *id, pattern.at),
                    _ => None,
                },
                _ => None,
            }
        };
        let Some(place) = bind(self, fx) else {
            self.say(m, pattern.at, "this pattern in for");
            return;
        };
        if *op == BinOp::Range {
            fx.emit(format!("while ({i} < {end}) {{"));
            fx.depth += 1;
            fx.emit(format!("{place} = {i};"));
            fx.emit(format!("{i}++;"));
        } else {
            // `a..=b` up to the type's last value: the step after `b` is never made.
            let go = fx.fresh("uint8_t");
            fx.emit(format!("{go} = ({i} <= {end});"));
            fx.emit(format!("while ({go}) {{"));
            fx.depth += 1;
            fx.emit(format!("{place} = {i};"));
            fx.emit(format!("if ({i} == {end}) {go} = 0; else {i}++;"));
        }
        fx.loops.push(fx.defer_scopes.len());
        self.block(fx, body);
        fx.loops.pop();
        fx.depth -= 1;
        fx.emit("}".into());
    }
}

impl<'a> Gen<'a> {
    /// A closure written as a builtin's argument, run here on `args` (each parameter's value and the type it has): its
    /// body written in place, so nothing is made at run time for the closure and no call is made; it reads and writes
    /// the names around it directly (it runs at once, inside the statement). `return v` gives the result `ret` and
    /// leaves the body, its own defers run first.
    pub(super) fn inline_closure(&mut self, fx: &mut Fx, params: &[ast::Param], body: &Block, args: &[(String, Ty)], ret: &Ty) -> Ex {
        let m = fx.module;
        let r = match ret {
            Ty::Unit => None,
            t => {
                let ct = self.c_type(t).ok_or_else(lost)?;
                Some((fx.fresh(&ct), t.clone()))
            }
        };
        let label = format!("lm_cl{}", fx.tmp);
        fx.tmp += 1;
        fx.emit("{".into());
        fx.depth += 1;
        for (p, (v, from)) in params.iter().zip(args) {
            let Some(n) = &p.name else { continue };
            let Some(Resolved::Local(id)) = self.checked.named.get(&(m, n.at)) else { return Err(lost()) };
            let id = *id;
            let lt = self.checked.locals[id as usize].ty.clone();
            let place = self.declare_local(fx, id, n.at).ok_or_else(lost)?;
            let lc = self.c_type(&lt).ok_or_else(lost)?;
            let v = self.coerce(fx, v, from, &lt, n.at)?;
            fx.set(&place, &v, &lc);
        }
        fx.inline_ret.push((r.clone(), label.clone(), fx.defer_scopes.len()));
        let loops = std::mem::take(&mut fx.loops);
        self.block(fx, body);
        fx.loops = loops;
        fx.inline_ret.pop();
        fx.depth -= 1;
        fx.emit(format!("{label}:;"));
        fx.emit("}".into());
        Ok(r.map(|(t, _)| t).unwrap_or_default())
    }

    /// `return` inside a closure written in place: the value into the closure's result, the closure's defers, out.
    fn inline_return(&mut self, fx: &mut Fx, v: Option<&Expr>) {
        let Some((r, label, mark)) = fx.inline_ret.last().cloned() else { return };
        if let (Some(e), Some((place, t))) = (v, &r) {
            self.value_into(fx, e, place, t);
        }
        let scopes: Vec<Vec<(String, Stmt)>> = fx.defer_scopes[mark..].to_vec();
        for scope in scopes.iter().rev() {
            for (flag, d) in scope.iter().rev() {
                self.run_defer(fx, flag, d);
            }
        }
        fx.emit(format!("goto {label};"));
    }

    /// `for x in xs` over a list or a set, `for (k, v) in m` over a map: the container read once and held (a write in
    /// the body copies it, so the loop sees what it was given), each element copied out before the body; a set and a
    /// map go in key order.
    fn for_container(&mut self, fx: &mut Fx, pattern: &Pattern, iter: &Expr, it: &Ty, body: &Block) {
        let elem = match it {
            Ty::List(x) | Ty::Set(x) => (**x).clone(),
            Ty::Map(k, v) => Ty::Tuple(vec![(**k).clone(), (**v).clone()]),
            _ => return,
        };
        let (Some(ct), Some(ec)) = (self.c_type(it), self.c_type(&elem)) else {
            let _ = lost();
            return;
        };
        let Ok(v) = self.expr(fx, iter) else { return };
        let held = fx.temp(&ct, &v);
        let n = match it {
            Ty::List(_) => format!("lumart_list_n({held})"),
            _ => format!("lumart_map_n({held})"),
        };
        let i = fx.temp("uint64_t", "0");
        let x = fx.fresh(&ec);
        fx.emit(format!("while ({i} < {n}) {{"));
        fx.depth += 1;
        match it {
            Ty::List(_) => fx.set(&x, &format!("(*({ec}*)lumart_list_at({held}, {i}))"), &ec),
            Ty::Set(_) => fx.set(&x, &format!("(*({ec}*)lumart_map_key({held}, {i}))"), &ec),
            Ty::Map(k, v) => {
                let (Some(kc), Some(vc)) = (self.c_type(k), self.c_type(v)) else {
                    let _ = lost();
                    return;
                };
                fx.set(&format!("{x}._0"), &format!("(*({kc}*)lumart_map_key({held}, {i}))"), &kc);
                fx.set(&format!("{x}._1"), &format!("(*({vc}*)lumart_map_val({held}, {i}))"), &vc);
            }
            _ => {}
        }
        // The step before the body, so `continue` takes it.
        fx.emit(format!("{i}++;"));
        let binds = self.pattern(fx, pattern, &x, &elem).map(|(_, b)| b).unwrap_or_default();
        for b in binds {
            fx.emit(b);
        }
        fx.loops.push(fx.defer_scopes.len());
        self.block(fx, body);
        fx.loops.pop();
        fx.depth -= 1;
        fx.emit("}".into());
    }
}

fn strip(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren(x) => strip(x),
        _ => e,
    }
}

fn all(tests: Vec<String>) -> String {
    let tests: Vec<String> = tests.into_iter().filter(|t| t != "1").collect();
    if tests.is_empty() {
        "1".into()
    } else {
        format!("({})", tests.join(" && "))
    }
}

pub(super) fn stmt_name(s: &StmtKind) -> &'static str {
    match s {
        StmtKind::Binding(_) => "let or var",
        StmtKind::Fn(_) => "a local fn",
        StmtKind::Return(_) => "return",
        StmtKind::Break => "break",
        StmtKind::Continue => "continue",
        StmtKind::Stop => "stop",
        StmtKind::Skip(_) => "skip",
        StmtKind::Go(_) => "go",
        StmtKind::Defer(_) => "defer",
        StmtKind::Log { .. } => "log",
        StmtKind::Expect(_) => "expect",
        StmtKind::Fail(_) => "fail",
        StmtKind::Guard { .. } => "guard",
        StmtKind::While { .. } => "while",
        StmtKind::For { .. } => "for",
        StmtKind::Expr(_) => "an expression",
        StmtKind::Assign { .. } => "an assignment",
    }
}
