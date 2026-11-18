//! A mod's `test` blocks (docs/modding/reference/lumascript/tests.md), written only in a test build (`lumalink test`,
//! [`super::emit_tests`]); a mod's own build leaves them out.
//!
//! A plain `test` is a function `lm_test_<k>` the runner calls in a process of its own, with no game: `luma_tests` (the
//! test build's export) gives the mod a core of its own whose every game name, service and other mod's function stops
//! the test and names what was touched. A `test game` is a function, or a task when it waits, run in the game after
//! every `on start`: lumart starts the first, and each one's end (`lumart_test_end`, in its way out) starts the next.
//!
//! What a test says is facts (log lines `fact KEY JSON`, docs/modding/reference/test-files.md, Facts): `expect` for
//! each failed `expect`, `test` for each test's end and `tests` once every test of a kind has run. The harness records
//! them for a game test; the runner reads them from its process for a plain one. LumaLink words them.

use super::expr::{binop_text, expr_text};
use super::{c_string, is_view, Fx, Gen};
use crate::ast::{self, BinOp, Expr, ExprKind, ItemKind, Pos};
use crate::check::{ItemRef, Ty};
use crate::game::GameTy;
use std::collections::HashSet;
use std::fmt::Write as _;

/// `s` as a JSON string.
pub(super) fn json_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A test's name as written: its literal's text (the checker allows no interpolation in it).
pub(super) fn test_name(name: &ast::StrLit) -> String {
    name.parts.iter().map(|p| if let ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect()
}

impl<'a> Gen<'a> {
    /// One `test` item of a test build: its function (a task's start when a game test waits), listed for the runner.
    pub(super) fn test_item(&mut self, m: usize, file: &str, game: bool, name: &ast::StrLit, body: &ast::Block, at: Pos, out: &mut String) {
        let k = self.plain_tests.len() + self.game_tests.len();
        let c_name = format!("lm_test_{k}");
        let mut fx = Fx::new(file, m);
        fx.test = true;
        fx.task = game;
        self.block(&mut fx, body);
        self.out_defers(&mut fx);
        let shown = test_name(name);
        if game {
            // Its end, however it ends (done, a script error, `stop`, the mod stopping): said, and the next one started
            // unless the mod is stopping.
            let stop = if fx.waits > 0 { "lm_f->h.stop" } else { "0" };
            fx.tail.push(format!("    lumart_test_end(lm_st, {stop});"));
            if fx.waits > 0 {
                let code = self.start_task_handler(&fx, &c_name, at);
                out.push_str(&code);
            } else {
                out.push_str(&fx.function(&format!("int32_t {c_name}(void)"), at));
            }
            self.game_tests.push((shown, c_name.clone()));
        } else {
            out.push_str(&fx.function(&format!("int32_t {c_name}(void)"), at));
            self.plain_tests.push((shown, c_name.clone()));
        }
        self.protos.push(format!("int32_t {c_name}(void);"));
    }

    /// `expect cond` in a test: when it does not hold, the fact `expect` with the file, the line, the condition and,
    /// for a comparison, both sides as JSON (each read once, before the comparison, as the comparison reads them).
    pub(super) fn expect(&mut self, fx: &mut Fx, e: &Expr, text: &str) {
        let m = fx.module;
        let inner = strip(e);
        let mut sides = None;
        let cond = match &inner.kind {
            ExprKind::Binary { op: op @ (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge), left, right }
                if !matches!(strip(left).kind, ExprKind::None) && !matches!(strip(right).kind, ExprKind::None) =>
            {
                let (lt, rt, t) = (self.ty(m, left), self.ty(m, right), self.ty(m, inner));
                let (Some(lct), Some(rct)) = (self.c_type(&lt), self.c_type(&rt)) else { return self.expect_plain(fx, e, text) };
                let Ok(l) = self.expr(fx, left) else { return };
                let l = fx.temp(&lct, &l);
                let Ok(r) = self.expr(fx, right) else { return };
                let r = fx.temp(&rct, &r);
                let Ok(c) = self.binary_values(fx, *op, &l, &lt, &r, &rt, &t, &expr_text(inner), inner.at) else { return };
                sides = Some(((l, lt, &**left), (r, rt, &**right), *op));
                c
            }
            _ => {
                let Ok(c) = self.expr(fx, e) else { return };
                c
            }
        };
        let ok = fx.temp("uint8_t", &format!("(uint8_t)({cond})"));
        fx.emit(format!("if (!{ok}) {{"));
        fx.depth += 1;
        let (b, mark) = fx.scratch_builder();
        let head = format!("\"file\": {}, \"line\": {}, \"text\": {}", json_quote(&fx.file), e.at.line, json_quote(text));
        fx.emit(format!("lumart_sb_str(&{b}, {});", c_string(&head)));
        if let Some(((l, lt, le), (r, rt, re), op)) = sides {
            let mut seen = HashSet::new();
            if self.expect_shows(&lt, &mut seen) && self.expect_shows(&rt, &mut seen) {
                fx.emit(format!("lumart_sb_str(&{b}, {});", c_string(&format!(", \"op\": {}, \"left\": ", json_quote(binop_text(op))))));
                if self.value_text(fx, &b, &l, &lt, le, true).is_err() {
                    return;
                }
                fx.emit(format!("lumart_sb_str(&{b}, \", \\\"right\\\": \");"));
                if self.value_text(fx, &b, &r, &rt, re, true).is_err() {
                    return;
                }
            }
        }
        fx.fail_if(&format!("lumart_sb_done(&{b}, {}, {})", c_string(&fx.file), e.at.line));
        fx.emit(format!("lumart_test_expect(lumart_sb_c(&{b}));"));
        fx.emit(format!("lumart_scratch_release({mark});"));
        fx.depth -= 1;
        fx.emit("}".into());
    }

    /// `expect` of a comparison whose sides have no C type of their own (a literal `none`): the condition alone.
    fn expect_plain(&mut self, fx: &mut Fx, e: &Expr, text: &str) {
        let Ok(c) = self.expr(fx, e) else { return };
        let ok = fx.temp("uint8_t", &format!("(uint8_t)({c})"));
        fx.emit(format!("if (!{ok}) {{"));
        fx.depth += 1;
        let (b, mark) = fx.scratch_builder();
        let head = format!("\"file\": {}, \"line\": {}, \"text\": {}", json_quote(&fx.file), e.at.line, json_quote(text));
        fx.emit(format!("lumart_sb_str(&{b}, {});", c_string(&head)));
        fx.fail_if(&format!("lumart_sb_done(&{b}, {}, {})", c_string(&fx.file), e.at.line));
        fx.emit(format!("lumart_test_expect(lumart_sb_c(&{b}));"));
        fx.emit(format!("lumart_scratch_release({mark});"));
        fx.depth -= 1;
        fx.emit("}".into());
    }

    /// Whether a value of `t` is written as JSON by value_text: what text shows, a struct of such fields, an option of
    /// one. A handle, a view, a function and the like are not; their comparison is said without its sides.
    fn expect_shows(&self, t: &Ty, seen: &mut HashSet<ItemRef>) -> bool {
        match t {
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::Value(_) | Ty::Text | Ty::Json | Ty::Bytes => true,
            Ty::Game(GameTy::Enum(_)) => true,
            Ty::Service { service, name } => {
                (service == "svc" && name == "Answer") || self.game.services.faces.get(service).is_some_and(|f| f.enums.contains_key(name))
            }
            Ty::Enum(item) => self.enum_plain(*item),
            Ty::List(x) | Ty::Set(x) => self.expect_shows(x, seen),
            Ty::Map(k, v) => self.expect_shows(k, seen) && self.expect_shows(v, seen),
            Ty::Tuple(xs) => xs.iter().all(|x| self.expect_shows(x, seen)),
            Ty::Opt(x) => !is_view(x) && self.expect_shows(x, seen),
            Ty::Struct(item) => {
                // A struct that holds a list of itself shows as any other: its writer is a function of its own.
                if !seen.insert(*item) {
                    return true;
                }
                let ItemKind::Struct(s) = &self.item(*item).kind else { return false };
                let names: Vec<String> = s.fields.iter().map(|f| f.name.name.clone()).collect();
                names.iter().all(|n| {
                    let ft = self.declared_ty(*item, &format!(".{n}"));
                    self.expect_shows(&ft, seen)
                })
            }
            _ => false,
        }
    }

    /// The test build's part of mod.c: the tables of tests, the stand-ins for every name of the game a plain test may
    /// reach, and `luma_tests`, the runner's entry. `slots` are the import slots by kind, as mod.c declares them.
    pub(super) fn tests_c(&self, c: &mut String, slots: &TestSlots) {
        c.push_str("\n/* The test build (lumalink test): the plain tests and the runner's entry. */\n");
        let _ = writeln!(c, "static const LumartTest lm_plain_tests[{}] = {{", self.plain_tests.len().max(1));
        for (name, f) in &self.plain_tests {
            let _ = writeln!(c, "    {{{}, {f}}},", c_string(name));
        }
        if self.plain_tests.is_empty() {
            c.push_str("    {0, 0},\n");
        }
        c.push_str("};\n");
        // A game function, a virtual call or another mod's function: a stand-in that stops the test, naming it.
        for (i, (_, name, kind)) in slots.functions.iter().enumerate() {
            let what = c_string(name);
            match kind {
                FnSlot::Game => {
                    let _ = writeln!(
                        c,
                        "static void lm_tt_{i}(void** a, void* r) {{\n    (void)a;\n    (void)r;\n    lumart_test_touched({what});\n}}\nstatic const LumaFunction lm_ttf_{i} = {{{what}, \"\", lm_tt_{i}, 0}};"
                    );
                }
                FnSlot::Mod => {
                    let _ = writeln!(c, "static void lm_tt_{i}(void) {{\n    lumart_test_touched({what});\n}}");
                }
            }
        }
        // A service: its own table, whose every function stops the test naming the service; its version the newest,
        // so every function the mod asks for is there. The mods service's `option` gives the option's default from
        // luma.json (the runner's), as a player who never changed it has it.
        for (i, s) in slots.services.iter().enumerate() {
            let _ = writeln!(c, "static void lm_tts_{i}(void) {{\n    lumart_test_touched({});\n}}\nstatic {} lm_ttt_{i};", c_string(&s.name), s.table);
        }
        c.push_str("\nstatic void lm_tests_bind(void) {\n");
        for (i, (slot, _, kind)) in slots.functions.iter().enumerate() {
            match kind {
                FnSlot::Game => {
                    let _ = writeln!(c, "    {slot} = &lm_ttf_{i};");
                }
                FnSlot::Mod => {
                    let _ = writeln!(c, "    {slot} = (const void*)(uintptr_t)lm_tt_{i};");
                }
            }
        }
        for (i, s) in slots.services.iter().enumerate() {
            let _ = writeln!(c, "    lumart_test_table(&lm_ttt_{i}, sizeof lm_ttt_{i}, {}, lm_tts_{i});\n    {} = &lm_ttt_{i};", s.major, s.slot);
            if s.name == "mods" {
                let _ = writeln!(c, "    lm_ttt_{i}.option = lumart_test_option;");
            }
        }
        for slot in &slots.minus_one {
            let _ = writeln!(c, "    {slot} = -1;");
        }
        c.push_str("}\n\n");
        // What a plain test reads made first: the options (a key's input action aside, which is the game's) and the
        // modules' values. A mod class is defined by the game, so a plain test makes none.
        c.push_str("static int32_t lm_tests_prepare(void) {\n");
        for (i, o) in self.options.iter().enumerate() {
            if !matches!(o.3, Ty::Service { .. }) {
                let _ = writeln!(c, "    lm_opt_read_{i}();");
            }
        }
        for i in &self.inits {
            let _ = writeln!(c, "    if ({i}() != 0) return 1;");
        }
        c.push_str("    return 0;\n}\n\n");
        // The runner's entry: the core the runtime gives the mod, the slots bound, the modules' values made, each plain
        // test run (its facts said through the host).
        let _ = writeln!(
            c,
            "LUMA_C LUMA_EXPORT int32_t luma_tests(const LumartTestHost* host) {{\n    return lumart_tests_run(host, luma_begin, lm_tests_bind, lm_tests_prepare, lm_plain_tests, {});\n}}",
            self.plain_tests.len()
        );
    }

    /// The game tests' table, before start (which runs them): none when there are none.
    pub(super) fn tests_game_table(&self, c: &mut String) {
        if self.game_tests.is_empty() {
            return;
        }
        let _ = writeln!(c, "\n/* The test build (lumalink test): the game tests, run in order from start. */\nstatic const LumartTest lm_game_tests[{}] = {{", self.game_tests.len());
        for (name, f) in &self.game_tests {
            let _ = writeln!(c, "    {{{}, {f}}},", c_string(name));
        }
        c.push_str("};\n");
    }

    /// The game tests' first start, at the end of start (after every `on start`): none when there are none.
    pub(super) fn tests_start(&self, c: &mut String) {
        if !self.game_tests.is_empty() {
            let _ = writeln!(c, "    lumart_tests_game(lm_game_tests, {});", self.game_tests.len());
        }
    }
}

/// The import slots of a test build, by what the runner binds to each.
#[derive(Default)]
pub(super) struct TestSlots {
    /// (slot, name, kind): a game function or virtual call (`const LumaFunction*`), or another mod's function.
    pub functions: Vec<(String, String, FnSlot)>,
    /// Every service, given or optional (an optional one given as any other).
    pub services: Vec<TestService>,
    /// Field offsets and class numbers: -1, as for a name the game does not have.
    pub minus_one: Vec<String>,
}

/// A service's slot, its name, its table's C type and its major version (a C name).
pub(super) struct TestService {
    pub slot: String,
    pub name: String,
    pub table: String,
    pub major: String,
}

pub(super) enum FnSlot {
    Game,
    Mod,
}

fn strip(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren(x) => strip(x),
        _ => e,
    }
}
