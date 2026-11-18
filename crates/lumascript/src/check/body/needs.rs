//! Part (g): what a game call needs, proved where it is (Nebula's docs/plan/M2-NEEDS-PROOF.md, N-2023). Each game
//! function's row lists its needs (luma_functions.json `needs`: `scene`, scene objects, `MarioActor`, `UserFile`,
//! `CometScheduler`); a call is refused (S0154) unless each is proved at its place:
//!
//! 1. a handler proves its set (the `guarantees` of the facts) in its body up to its first `wait`: `on stage ready`,
//!    `on stage "NAME"` the set `stage`; `on play`, `on player ready` the set `player`; `on stage ended` `system`;
//! 2. a hook's body proves what its game function needs (every override's, with `every override of`), and an
//!    `override fn`'s what the virtual it replaces needs: the game is calling it; an `override fn` also proves the
//!    set `stage`'s needs (an object of a mod's class lives only while they are there, [`Walk::prove_object`]);
//! 3. `if can(F)` (and `guard`, `while`, `&&`; `if !can(F)` whose block leaves) proves F's needs;
//! 4. `if let x = G()` proves what G's result is null without (`null_when`);
//! 5. a function of the mod (a module's `fn`, a struct's or class's own) proves nothing of its own: what its body
//!    needs and does not prove becomes its needs, to a fixed point, and each call of it must prove them. One given
//!    away (a value, an export, an `override fn`, a closure) has no caller the checker sees: it proves its own.
//!
//! A `wait` ends every proof made before it (the stage may have ended), on every path a loop goes back round too.
//! The kernel still checks each call at run time (LumaHook.cpp `luma::present`): this says it at build time.

use super::expr::NameKind;
use super::{Resolved, Walk};
use crate::ast::{self, Expr, ExprKind, Pos};
use crate::check::{ItemRef, ModRef, Ty};
use crate::diag::{code, Arg, Diagnostic};
use std::collections::{BTreeSet, HashMap, HashSet};

/// A function of the mod whose needs its callers prove: its item (the function's, or its struct's or class's) and key
/// (`""`, or `.name`).
pub(crate) type FnKey = (ItemRef, String);

/// What a call needing proof calls.
#[derive(Clone, Debug)]
pub(crate) enum Callee {
    /// A game function's row, its name and needs.
    Game { name: String, needs: Vec<String> },
    /// A function of the mod.
    Mod { key: FnKey, name: String },
}

/// One call whose needs are proved where it is: the needs proved there, each with the tick it was proved at (a loop
/// that waits takes back what was proved before it).
#[derive(Clone, Debug)]
pub(crate) struct NeedSite {
    m: ModRef,
    at: Pos,
    callee: Callee,
    /// The function of the mod it is in, when its needs are its callers' to prove.
    caller: Option<FnKey>,
    proved: Vec<(String, u32)>,
}

/// The calls of the mod's bodies, kept until every body is walked (part (g) is said after them).
#[derive(Default, Debug)]
pub(crate) struct Needs {
    sites: Vec<NeedSite>,
    /// The functions whose needs are their callers'.
    fns: HashSet<FnKey>,
    /// The functions given away: a module `fn` named as a value.
    given_away: HashSet<FnKey>,
}

impl<'w, 'a> Walk<'w, 'a> {
    /// A row's needs (none without facts).
    pub(super) fn row_needs(&self, name: &str, signature: &str) -> Vec<String> {
        self.game().facts.as_ref().and_then(|f| f.get(name, signature)).map(|f| f.needs.clone()).unwrap_or_default()
    }

    /// A set's needs by its name (none when the facts have no such set).
    fn set_needs(&self, set: &str) -> Vec<String> {
        self.game().facts.as_ref().and_then(|f| f.guarantees.iter().find(|g| g.name == set)).map(|g| g.needs.clone()).unwrap_or_default()
    }

    /// `needs` proved here from now on, in this scope, until a `wait`.
    pub(super) fn prove(&mut self, needs: Vec<String>) {
        if needs.is_empty() {
            return;
        }
        let t = self.next_tick();
        let frame = self.frame_mut();
        for n in needs {
            frame.proved.push((n, t));
        }
    }

    /// The handler's set, at its body's start.
    pub(super) fn prove_handler(&mut self, event: &ast::OnEvent) {
        let set = match event {
            ast::OnEvent::Stage(_) | ast::OnEvent::StageReady => "stage",
            ast::OnEvent::Play | ast::OnEvent::PlayerReady => "player",
            ast::OnEvent::StageEnded => "system",
            _ => return,
        };
        let needs = self.set_needs(set);
        self.prove(needs);
    }

    /// A hook's or an override's body: what each row it runs for needs (the game is calling it).
    pub(super) fn prove_rows(&mut self, rows: &[(String, String)]) {
        let mut common: Option<Vec<String>> = None;
        for (n, s) in rows {
            let needs = self.row_needs(n, s);
            common = Some(match common {
                None => needs,
                Some(c) => c.into_iter().filter(|x| needs.contains(x)).collect(),
            });
        }
        self.prove(common.unwrap_or_default());
    }

    /// An `override fn`'s body: the object of a mod's class is made only while a game scene places its objects or
    /// once it is made (LumaCore.make refuses before; the placement is GameScene::initAfterScenarioChosen's phase 3,
    /// after phases 0 to 2 and GameScene::init made every scene object of the set `stage`), and it lives as long as the
    /// scene: the set's needs are there whenever the game calls one of its virtuals (the scene's making need not be
    /// finished).
    pub(super) fn prove_object(&mut self) {
        let needs = self.set_needs("stage");
        self.prove(needs);
    }

    /// The proofs that stand here: made in a scope still open, with no `wait` after them on any path to here.
    fn proved_now(&self) -> Vec<(String, u32)> {
        self.frame().proved.iter().filter(|(_, t)| self.waited_from(*t).is_none()).cloned().collect()
    }

    /// A closure that runs at once inside its statement (a built-in's closure argument): the proofs and the function
    /// around it.
    pub(super) fn closure_inherits(&mut self) {
        let n = self.frames.len();
        if n < 2 || !self.frames[n - 1].runs_now {
            return;
        }
        let proved = {
            let outer = &self.frames[n - 2];
            outer.proved.iter().filter(|(_, t)| outer.path.waits.iter().all(|(w, _)| *w < *t)).cloned().collect::<Vec<_>>()
        };
        let key = self.frames[n - 2].fn_key.clone();
        let inner = &mut self.frames[n - 1];
        inner.proved = proved;
        inner.fn_key = key;
    }

    /// A function of the mod being walked: its needs are its callers'.
    pub(super) fn needs_are_callers(&mut self, key: FnKey) {
        if self.record {
            self.c.needs.fns.insert(key.clone());
        }
        self.frame_mut().fn_key = Some(key);
    }

    /// A module `fn` named as a value: given away, it proves its own needs.
    pub(super) fn given_away(&mut self, item: ItemRef) {
        if self.record {
            self.c.needs.given_away.insert((item, String::new()));
        }
    }

    /// A call of a game function's row: kept with what is proved here.
    pub(super) fn game_needs(&mut self, at: Pos, name: &str, signature: &str) {
        let needs = self.row_needs(name, signature);
        if !needs.is_empty() {
            self.need_site(at, Callee::Game { name: name.to_string(), needs });
        }
        // What it makes is there after it (MR::declareCoin makes CoinHolder), as gen_types.py's merge counts it.
        let makes = self.game().facts.as_ref().and_then(|f| f.get(name, signature)).map(|f| f.makes.clone()).unwrap_or_default();
        self.prove(makes);
    }

    /// A call of a function of the mod's.
    pub(super) fn mod_needs(&mut self, at: Pos, key: FnKey, name: String) {
        if key.0.owner == crate::check::Owner::Mod {
            self.need_site(at, Callee::Mod { key, name });
        }
    }

    fn need_site(&mut self, at: Pos, callee: Callee) {
        if !self.record {
            return;
        }
        let caller = self.frame().fn_key.clone();
        let proved = self.proved_now();
        let m = self.m;
        self.c.needs.sites.push(NeedSite { m, at, callee, caller, proved });
    }

    /// Where a loop starts: the calls kept from here on are the loop's.
    pub(super) fn sites_mark(&self) -> usize {
        self.c.needs.sites.len()
    }

    /// A loop that waits on a path back round: its calls do not keep what was proved before it.
    pub(super) fn loop_waited(&mut self, from: usize, start: u32) {
        for s in &mut self.c.needs.sites[from..] {
            s.proved.retain(|(_, t)| *t >= start);
        }
    }

    /// `can(F)`: F a game function's name (`MR::f`, a quoted row `` `MR::f(int)` ``, a bare game name); every row of
    /// it the script may call, or the one written. A `bool`.
    pub(super) fn can_call(&mut self, e: &'a Expr, args: &'a [ast::Arg]) -> Ty {
        let named = match args {
            [a] if a.name.is_none() => match &a.value.kind {
                ExprKind::Game(gn) => match gn.text.find('(') {
                    Some(k) if gn.quoted => Some((gn.text[..k].trim_end().to_string(), Some(gn.text[k..].to_string()), gn.at)),
                    _ => Some((gn.text.clone(), None, gn.at)),
                },
                ExprKind::Name { name, targs } if targs.is_empty() && matches!(self.name_kind(&name.name), NameKind::Game) => Some((name.name.clone(), None, name.at)),
                _ => None,
            },
            _ => None,
        };
        let Some((name, written, at)) = named else {
            self.say(Diagnostic::new(code::CAN_WHAT, e.at));
            return Ty::Bool;
        };
        let Some(rows) = self.game().functions.get(&name) else {
            self.unknown_name(at, &name);
            return Ty::Bool;
        };
        let wanted = written.as_ref().map(|w| crate::ctype::parse_signature(&format!("void{w}")));
        let signatures: Vec<String> = rows
            .iter()
            .filter(|(_, tell)| !tell)
            .map(|(s, _)| s.clone())
            .filter(|s| match &wanted {
                None => true,
                Some(w) => match (w, crate::ctype::parse_signature(s)) {
                    (Ok(a), Ok(b)) => a.params == b.params && a.konst == b.konst,
                    _ => false,
                },
            })
            .collect();
        if signatures.is_empty() {
            self.say(Diagnostic::new(code::CAN_WHAT, at));
            return Ty::Bool;
        }
        self.res.insert(e.id, Resolved::Can { name, signatures });
        Ty::Bool
    }

    /// `holds("SET")`: the kernel's check of a set's needs (LumaCore.holds), proving them in its branch as `can` does a
    /// function's. A name that is no set the game proves is said (S0156); a game with no sets knows none to refuse.
    pub(super) fn holds_call(&mut self, e: &'a Expr, args: &'a [ast::Arg]) -> Ty {
        let sets: Vec<String> = self.game().facts.as_ref().map(|f| f.guarantees.iter().map(|g| g.name.clone()).collect()).unwrap_or_default();
        let set = match args {
            [a] if a.name.is_none() => super::call::literal_text(&a.value),
            _ => None,
        };
        match set {
            Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') && (sets.is_empty() || sets.contains(&s)) => {
                self.res.insert(e.id, Resolved::Holds { set: s });
            }
            _ => {
                let named: Vec<String> = sets.iter().map(|s| format!("\"{s}\"")).collect();
                self.say(Diagnostic::new(code::HOLDS_WHAT, e.at).with("sets", Arg::Text(named.join(", "))));
            }
        }
        Ty::Bool
    }

    /// What `e` proves when it is `holds` (true: `can(F)`, `a && b`; false: `!can(F)`, `a || b`).
    pub(super) fn proved_by(&self, e: &Expr, holds: bool, out: &mut Vec<String>) {
        match &e.kind {
            ExprKind::Paren(x) => self.proved_by(x, holds, out),
            ExprKind::Not(x) => self.proved_by(x, !holds, out),
            ExprKind::Binary { op: ast::BinOp::And, left, right } if holds => {
                self.proved_by(left, true, out);
                self.proved_by(right, true, out);
            }
            ExprKind::Binary { op: ast::BinOp::Or, left, right } if !holds => {
                self.proved_by(left, false, out);
                self.proved_by(right, false, out);
            }
            ExprKind::Call { .. } if holds => {
                match self.res.get(&e.id) {
                    Some(Resolved::Can { name, signatures }) => {
                        for s in signatures {
                            out.extend(self.row_needs(name, s));
                        }
                    }
                    Some(Resolved::Holds { set }) => out.extend(self.set_needs(set)),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// `if let x = G()`: what G's result is null without.
    pub(super) fn proved_by_let(&self, value: &Expr) -> Vec<String> {
        let mut v = value;
        while let ExprKind::Paren(x) = &v.kind {
            v = x;
        }
        match (&v.kind, self.res.get(&v.id)) {
            (ExprKind::Call { .. }, Some(Resolved::GameFn { name, signature })) => {
                self.game().facts.as_ref().and_then(|f| f.get(name, signature)).map(|f| f.null_when.clone()).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
}

/// After every body: each function's needs to a fixed point, then each call no caller proves for is said (S0154).
pub(super) fn say(c: &mut crate::check::Checker<'_>) {
    let needs = std::mem::take(&mut c.needs);
    let open = |k: &FnKey| needs.fns.contains(k) && !needs.given_away.contains(k);
    let mut of: HashMap<FnKey, BTreeSet<String>> = HashMap::new();
    let callee_needs = |of: &HashMap<FnKey, BTreeSet<String>>, callee: &Callee| -> BTreeSet<String> {
        match callee {
            Callee::Game { needs, .. } => needs.iter().cloned().collect(),
            Callee::Mod { key, .. } if open(key) => of.get(key).cloned().unwrap_or_default(),
            Callee::Mod { .. } => BTreeSet::new(),
        }
    };
    let missing = |of: &HashMap<FnKey, BTreeSet<String>>, s: &NeedSite| -> BTreeSet<String> {
        callee_needs(of, &s.callee).into_iter().filter(|n| !s.proved.iter().any(|(p, _)| p == n)).collect()
    };
    loop {
        let mut changed = false;
        for s in &needs.sites {
            let Some(k) = s.caller.as_ref().filter(|k| open(k)) else { continue };
            let more = missing(&of, s);
            let mine = of.entry(k.clone()).or_default();
            for n in more {
                changed |= mine.insert(n);
            }
        }
        if !changed {
            break;
        }
    }
    let sets: Vec<(String, Vec<String>)> = c.facts.game.facts.as_ref().map(|f| f.guarantees.iter().map(|g| (g.name.clone(), g.needs.clone())).collect()).unwrap_or_default();
    let within = |set: &str, m: &BTreeSet<String>| sets.iter().find(|(n, _)| n == set).is_some_and(|(_, needs)| m.iter().all(|x| needs.contains(x)));
    for s in &needs.sites {
        if s.caller.as_ref().is_some_and(|k| open(k)) {
            continue;
        }
        let m = missing(&of, s);
        if m.is_empty() {
            continue;
        }
        let how = if within("stage", &m) {
            "Script.Needs.Stage"
        } else if within("player", &m) {
            "Script.Needs.Player"
        } else {
            "Script.Needs.Can"
        };
        let found = match &s.callee {
            Callee::Game { name, .. } | Callee::Mod { name, .. } => name.clone(),
        };
        let listed = m.into_iter().collect::<Vec<_>>().join(", ");
        let d = Diagnostic::new(code::NEEDS_NOT_PROVED, s.at).with("found", Arg::Text(found)).with("needs", Arg::Text(listed)).with("how", Arg::Phrase(how));
        c.say(s.m, d);
    }
}
