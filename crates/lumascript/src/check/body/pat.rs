//! Patterns: what they bind and match, and whether a `match` covers every value (section 4: a missing
//! case is an error naming it). Coverage is the usefulness algorithm over the arms' patterns (Maranget, "Warnings for
//! pattern matching", 2007), giving the values no arm takes.

use super::{Resolved, Walk, Want};
use crate::ast::{self, PatternKind, Pos};
use crate::check::{ItemRef, ModTarget, Name, Ty};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::GameTy;

/// A pattern as coverage reads it.
#[derive(Clone, Debug)]
pub(super) enum Pat {
    /// A name, `else`, or a part nothing checks: every value.
    Any,
    Ctor(Ctor, Vec<Pat>),
    /// One value of a type whose values cannot all be listed (a number, a text).
    Lit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Ctor {
    True,
    False,
    None,
    Some,
    /// The mod's enum's case, by its index.
    Case(usize),
    /// A game enum's value, by its number (two names of one number are one value).
    GameValue(i64),
    Tuple,
}

/// A value no arm takes, for the message.
#[derive(Clone, Debug)]
enum W {
    Any,
    /// Some value of a type that cannot be listed: only `else` covers it.
    Other,
    Ctor(String, Vec<W>),
}

/// The most values a message lists.
const SHOWN: usize = 8;

/// The names a pattern binds.
pub(super) fn pattern_binds<'p>(p: &'p ast::Pattern, out: &mut Vec<&'p ast::Ident>) {
    match &p.kind {
        PatternKind::Tuple(items) => items.iter().for_each(|i| pattern_binds(i, out)),
        PatternKind::Literal(_) => {}
        PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => out.push(&path[0]),
        PatternKind::Path { args, fields, .. } => {
            if let Some(args) = args {
                args.iter().for_each(|a| pattern_binds(a, out));
            }
            if let Some(fields) = fields {
                out.extend(fields.iter());
            }
        }
    }
}

/// Names and tuples of names: a pattern that matches every value of its type.
pub(super) fn irrefutable_shape(p: &ast::Pattern) -> bool {
    match &p.kind {
        PatternKind::Tuple(items) => items.iter().all(irrefutable_shape),
        PatternKind::Path { path, args: None, fields: None } => path.len() == 1,
        _ => false,
    }
}

/// What a case path names.
enum CaseOf {
    Mod { item: ItemRef, index: usize },
    Game(super::GameValue),
    /// `<svc>.<Enum>.<Case>`: the service, the enum, the case and its number.
    Service { service: String, ty: String, case: String, number: i64 },
    /// `<svc>.Event.<Kind>`: the service's stream event kind, its number and its fields' types.
    Stream { service: String, kind: String, number: i64, fields: Vec<(String, Ty)> },
}

impl<'w, 'a> Walk<'w, 'a> {
    /// A pattern against a value of `ty`: it binds its names and gives what it matches.
    pub(super) fn pattern(&mut self, p: &'a ast::Pattern, ty: &Ty, var: bool) -> Pat {
        if let Ty::Opt(inner) = ty {
            return match &p.kind {
                PatternKind::Literal(e) if matches!(e.kind, ast::ExprKind::None) => {
                    self.expr(e, Want::Ty(ty));
                    Pat::Ctor(Ctor::None, vec![])
                }
                PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => {
                    self.bind(&path[0], ty.clone(), var);
                    Pat::Any
                }
                _ => {
                    let inner = (**inner).clone();
                    Pat::Ctor(Ctor::Some, vec![self.pattern(p, &inner, var)])
                }
            };
        }
        match &p.kind {
            PatternKind::Path { path, args: None, fields: None } if path.len() == 1 => {
                self.bind(&path[0], ty.clone(), var);
                Pat::Any
            }
            PatternKind::Tuple(items) => {
                let tys = self.tuple_parts(ty, items.len(), p.at);
                let parts = items.iter().zip(tys.iter()).map(|(i, t)| self.pattern(i, t, var)).collect();
                Pat::Ctor(Ctor::Tuple, parts)
            }
            PatternKind::Literal(e) => {
                match ty {
                    Ty::Error | Ty::Never => {
                        self.expr(e, Want::Any);
                    }
                    t => {
                        self.expr(e, Want::Ty(t));
                    }
                }
                match (&e.kind, ty) {
                    (ast::ExprKind::True, Ty::Bool) => Pat::Ctor(Ctor::True, vec![]),
                    (ast::ExprKind::False, Ty::Bool) => Pat::Ctor(Ctor::False, vec![]),
                    _ => Pat::Lit,
                }
            }
            PatternKind::Path { path, args, fields } => self.case_pattern(p.at, path, args.as_deref(), fields.as_deref(), ty, var),
        }
    }

    /// `E.Case`, `E.Case(a, b)`, `E.Case { a, b }`, `module.E.Case...`, or a game enum's value written with `.` for
    /// `::` (`MarioStatus.MarioStatus_Swim`, `Mario.MovementStates...`).
    fn case_pattern(&mut self, at: Pos, path: &'a [ast::Ident], args: Option<&'a [ast::Pattern]>, fields: Option<&'a [ast::Ident]>, ty: &Ty, var: bool) -> Pat {
        let last = &path[path.len() - 1];
        let Some(found) = self.case_path(path) else {
            self.bind_all(args, fields, var);
            return Pat::Any;
        };
        match found {
            CaseOf::Mod { item, index } => {
                if !matches!(ty, Ty::Enum(e) if *e == item) && !matches!(ty, Ty::Error | Ty::Never) {
                    let (found, expected) = (self.c.item_name(item), self.ty_text(ty));
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                    self.bind_all(args, fields, var);
                    return Pat::Any;
                }
                let ast::ItemKind::Enum(decl) = &self.item_of(item).kind else { return Pat::Any };
                let case = &decl.cases[index];
                self.named.insert(last.at, Resolved::Case { item, case: case.name.name.clone() });
                let field_ty = |w: &Self, f: &str| w.declared(item, &format!("{}.{f}", case.name.name));
                let mut parts = vec![Pat::Any; case.fields.len()];
                match (args, fields) {
                    (Some(args), _) => {
                        if args.len() != case.fields.len() {
                            let d = Diagnostic::new(code::ARG_COUNT, at)
                                .with("name", Arg::Text(format!("{}.{}", decl.name.name, case.name.name)))
                                .with("found", Arg::Text(args.len().to_string()))
                                .with("expected", Arg::Text(case.fields.len().to_string()));
                            self.say(d);
                        }
                        for (k, a) in args.iter().enumerate() {
                            let t = case.fields.get(k).map_or(Ty::Error, |f| field_ty(self, &f.name.name));
                            let pat = self.pattern(a, &t, var);
                            if k < parts.len() {
                                parts[k] = pat;
                            }
                        }
                    }
                    (None, Some(names)) => {
                        for n in names {
                            match case.fields.iter().position(|f| f.name.name == n.name) {
                                Some(k) => {
                                    let t = field_ty(self, &n.name);
                                    self.bind(n, t, var);
                                    let _ = k;
                                }
                                None => {
                                    let d = Diagnostic::new(code::NO_MEMBER, n.at).with("name", Arg::Text(n.name.clone())).with("found", Arg::Text(format!("{}.{}", decl.name.name, case.name.name)));
                                    self.say(d);
                                    self.bind(n, Ty::Error, var);
                                }
                            }
                        }
                    }
                    (None, None) => {}
                }
                Pat::Ctor(Ctor::Case(index), parts)
            }
            CaseOf::Game(v) => {
                let enum_ty = Ty::Game(GameTy::Enum(v.enum_name.clone()));
                let fits = v.enum_name.is_empty() && matches!(ty, Ty::Int(_)) || *ty == enum_ty || matches!(ty, Ty::Error | Ty::Never);
                if !fits {
                    let (found, expected) = (self.ty_text(&enum_ty), self.ty_text(ty));
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                }
                if args.is_some() || fields.is_some() {
                    let d = Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(v.value.clone())).with("found", Arg::Text("1".into())).with("expected", Arg::Text("0".into()));
                    self.say(d);
                    self.bind_all(args, fields, var);
                }
                let number = v.number;
                self.named.insert(last.at, Resolved::GameEnumValue { enum_name: v.enum_name, value: v.value, number });
                if fits && !matches!(ty, Ty::Int(_)) {
                    Pat::Ctor(Ctor::GameValue(number), vec![])
                } else {
                    Pat::Lit
                }
            }
            CaseOf::Stream { service, kind, number, fields: decl } => {
                let enum_ty = Ty::Service { service: service.clone(), name: "Event".into() };
                let shown = format!("{}.Event.{}", service.rsplit('.').next().unwrap_or(&service), crate::svc::case_name(&kind.to_ascii_uppercase()));
                if *ty != enum_ty && !matches!(ty, Ty::Error | Ty::Never) {
                    let (found, expected) = (self.ty_text(&enum_ty), self.ty_text(ty));
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                    self.bind_all(args, fields, var);
                    return Pat::Any;
                }
                self.named.insert(last.at, Resolved::StreamCase { service, kind, number });
                let mut parts = vec![Pat::Any; decl.len()];
                match (args, fields) {
                    (Some(args), _) => {
                        if args.len() != decl.len() {
                            let d = Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(shown)).with("found", Arg::Text(args.len().to_string())).with("expected", Arg::Text(decl.len().to_string()));
                            self.say(d);
                        }
                        for (k, a) in args.iter().enumerate() {
                            let t = decl.get(k).map_or(Ty::Error, |f| f.1.clone());
                            let pat = self.pattern(a, &t, var);
                            if k < parts.len() {
                                parts[k] = pat;
                            }
                        }
                    }
                    (None, Some(names)) => {
                        for n in names {
                            match decl.iter().find(|f| f.0 == n.name) {
                                Some((_, t)) => {
                                    self.bind(n, t.clone(), var);
                                }
                                None => {
                                    let d = Diagnostic::new(code::NO_MEMBER, n.at).with("name", Arg::Text(n.name.clone())).with("found", Arg::Text(shown.clone()));
                                    self.say(d);
                                    self.bind(n, Ty::Error, var);
                                }
                            }
                        }
                    }
                    (None, None) => {}
                }
                Pat::Ctor(Ctor::GameValue(number), parts)
            }
            CaseOf::Service { service, ty: name, case, number } => {
                let enum_ty = Ty::Service { service: service.clone(), name: name.clone() };
                if *ty != enum_ty && !matches!(ty, Ty::Error | Ty::Never) {
                    let (found, expected) = (self.ty_text(&enum_ty), self.ty_text(ty));
                    self.say(Diagnostic::new(code::TYPE_MISMATCH, at).with("found", Arg::Text(found)).with("expected", Arg::Text(expected)));
                    self.bind_all(args, fields, var);
                    return Pat::Any;
                }
                if args.is_some() || fields.is_some() {
                    let d = Diagnostic::new(code::ARG_COUNT, at).with("name", Arg::Text(case.clone())).with("found", Arg::Text("1".into())).with("expected", Arg::Text("0".into()));
                    self.say(d);
                    self.bind_all(args, fields, var);
                }
                self.named.insert(last.at, Resolved::ServiceCase { service, ty: name, case, number });
                Pat::Ctor(Ctor::GameValue(number), vec![])
            }
        }
    }

    /// Binds a pattern's names as `Ty::Error` (the pattern was said wrong; what follows is still checked).
    fn bind_all(&mut self, args: Option<&'a [ast::Pattern]>, fields: Option<&'a [ast::Ident]>, var: bool) {
        let mut idents = Vec::new();
        if let Some(args) = args {
            args.iter().for_each(|a| pattern_binds(a, &mut idents));
        }
        if let Some(fields) = fields {
            idents.extend(fields.iter());
        }
        for i in idents {
            self.bind(i, Ty::Error, var);
        }
    }

    /// What a case path names, or None once said why not.
    fn case_path(&mut self, path: &'a [ast::Ident]) -> Option<CaseOf> {
        let written = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
        // The mod's enum: `E.Case`, or `module.E.Case`.
        let enum_item = match path.len() {
            2 => match self.c.scope_type(self.m, &path[0].name) {
                Some(Ty::Enum(e)) => Some(e),
                _ => None,
            },
            3 => match self.c.scope(self.m).names.get(&path[0].name).cloned() {
                Some(Name::Module { target: ModTarget::Module { owner, module }, .. }) => {
                    let m = crate::check::ModRef { owner, module };
                    match self.c.scope(m).names.get(&path[1].name) {
                        Some(Name::Item { item, public: true, .. }) => match self.c.item_type(m.item(*item)) {
                            Some(Ty::Enum(e)) => Some(e),
                            _ => None,
                        },
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(item) = enum_item {
            let ast::ItemKind::Enum(decl) = &self.item_of(item).kind else { return None };
            let last = &path[path.len() - 1];
            return match decl.cases.iter().position(|c| c.name.name == last.name) {
                Some(index) => Some(CaseOf::Mod { item, index }),
                None => {
                    let d = Diagnostic::new(code::NO_MEMBER, last.at).with("name", Arg::Text(last.name.clone())).with("found", Arg::Text(decl.name.name.clone()));
                    self.say(d);
                    None
                }
            };
        }
        // A service's enum's case: `<module>.<Enum>.<Case>` after `use svc.<service>`; a stream event's,
        // `<module>.Event.<Kind>`.
        if path.len() == 3 {
            if let Some(Name::Module { target: ModTarget::Service(service), .. }) = self.c.scope(self.m).names.get(&path[0].name).cloned() {
                if let Some(face) = self.game().services.faces.get(&service).filter(|f| path[1].name == "Event" && !f.stream.is_empty()) {
                    let last = &path[2];
                    let found = face.stream.iter().find(|(k, _)| crate::svc::case_name(&k.to_ascii_uppercase()) == last.name);
                    return match found {
                        Some((kind, k)) => {
                            let fields = k.fields.iter().map(|(n, t)| (n.clone(), super::service::sty(&service, t))).collect();
                            Some(CaseOf::Stream { service: service.clone(), kind: kind.clone(), number: k.number, fields })
                        }
                        None => {
                            let found = Arg::Text(format!("{}.Event", path[0].name));
                            self.say(Diagnostic::new(code::NO_MEMBER, last.at).with("name", Arg::Text(last.name.clone())).with("found", found));
                            None
                        }
                    };
                }
                if let Some(cases) = self.game().services.faces.get(&service).and_then(|f| f.enums.get(&path[1].name)) {
                    let last = &path[2];
                    return match cases.iter().find(|(c, _)| *c == last.name) {
                        Some((c, n)) => Some(CaseOf::Service { service, ty: path[1].name.clone(), case: c.clone(), number: *n }),
                        None => {
                            let found = Arg::Text(format!("{}.{}", path[0].name, path[1].name));
                            self.say(Diagnostic::new(code::NO_MEMBER, last.at).with("name", Arg::Text(last.name.clone())).with("found", found));
                            None
                        }
                    };
                }
            }
        }
        // A game enum's value: the path with `::` (the mod's names come first).
        let game = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("::");
        if let Some(v) = self.c.game_values.get(&game) {
            return Some(CaseOf::Game(v.clone()));
        }
        self.unknown_name(path[0].at, &written);
        None
    }

    /// The values of `subject_ty` no arm of a `match` takes (said at `at`): MATCH_MISSING naming them, or
    /// MATCH_NEEDS_ELSE when only `else` could cover them.
    pub(super) fn coverage(&mut self, at: Pos, subject_ty: &Ty, arms: Vec<Pat>) {
        let rows: Vec<Vec<Pat>> = arms.into_iter().map(|p| vec![p]).collect();
        let found = self.witnesses(&rows, std::slice::from_ref(subject_ty));
        if found.is_empty() {
            return;
        }
        if found.iter().any(|w| has_other(&w[0])) {
            self.say(Diagnostic::new(code::MATCH_NEEDS_ELSE, at));
            return;
        }
        let mut cases: Vec<String> = found.iter().take(SHOWN).map(|w| w_text(&w[0])).collect();
        cases.dedup();
        if found.len() > SHOWN {
            cases.push("...".into());
        }
        self.say(Diagnostic::new(code::MATCH_MISSING, at).with("cases", Arg::Text(cases.join(", "))));
    }

    /// Every value a type has, as constructors with their parts' types and names; None when they cannot be listed.
    fn ctors(&self, t: &Ty) -> Option<Vec<(Ctor, Vec<Ty>, String)>> {
        match t {
            Ty::Bool => Some(vec![(Ctor::True, vec![], "true".into()), (Ctor::False, vec![], "false".into())]),
            Ty::Opt(inner) => Some(vec![(Ctor::None, vec![], "none".into()), (Ctor::Some, vec![(**inner).clone()], String::new())]),
            Ty::Tuple(items) => Some(vec![(Ctor::Tuple, items.clone(), String::new())]),
            Ty::Enum(item) => {
                let ast::ItemKind::Enum(decl) = &self.item_of(*item).kind else { return None };
                Some(
                    decl.cases
                        .iter()
                        .enumerate()
                        .map(|(k, c)| {
                            let tys = c.fields.iter().map(|f| self.declared(*item, &format!("{}.{}", c.name.name, f.name.name))).collect();
                            (Ctor::Case(k), tys, format!("{}.{}", decl.name.name, c.name.name))
                        })
                        .collect(),
                )
            }
            Ty::Game(GameTy::Enum(name)) => {
                let e = self.game().types.as_ref()?.enums.get(name)?;
                let mut seen = std::collections::BTreeSet::new();
                // Named as an arm writes it (`Mode.Walk`; the checker joins the path with `::`).
                let dotted = name.replace("::", ".");
                Some(e.values.iter().filter(|(_, n)| seen.insert(*n)).map(|(v, n)| (Ctor::GameValue(*n), vec![], format!("{dotted}.{v}"))).collect())
            }
            // A service's stream events: a case per kind, with its fields.
            Ty::Service { service, name } if name == "Event" && self.game().services.faces.get(service).is_some_and(|f| !f.stream.is_empty()) => {
                let face = self.game().services.faces.get(service)?;
                let module = service.rsplit('.').next().unwrap_or(service);
                Some(
                    face.stream
                        .iter()
                        .map(|(k, s)| {
                            let tys = s.fields.iter().map(|(_, t)| super::service::sty(service, t)).collect();
                            (Ctor::GameValue(s.number), tys, format!("{module}.Event.{}", crate::svc::case_name(&k.to_ascii_uppercase())))
                        })
                        .collect(),
                )
            }
            // A service's enum: named by the module a `use` binds (its last part).
            Ty::Service { service, name } => {
                let cases = self.game().services.faces.get(service)?.enums.get(name)?;
                let module = service.rsplit('.').next().unwrap_or(service);
                let mut seen = std::collections::BTreeSet::new();
                Some(cases.iter().filter(|(_, n)| seen.insert(*n)).map(|(c, n)| (Ctor::GameValue(*n), vec![], format!("{module}.{name}.{c}"))).collect())
            }
            _ => None,
        }
    }

    /// The rows of values `rows` does not take, over columns of `tys` (at most a few more than [`SHOWN`]).
    fn witnesses(&self, rows: &[Vec<Pat>], tys: &[Ty]) -> Vec<Vec<W>> {
        let Some(t) = tys.first() else {
            return if rows.is_empty() { vec![vec![]] } else { vec![] };
        };
        let rest = &tys[1..];
        // A part that did not check is taken by every pattern: nothing more is said about it.
        if matches!(t, Ty::Error | Ty::Never) {
            let def: Vec<Vec<Pat>> = rows.iter().map(|r| r[1..].to_vec()).collect();
            return self.witnesses(&def, rest).into_iter().map(|w| prepend(W::Any, w)).collect();
        }
        match self.ctors(t) {
            Some(all) if all.is_empty() => vec![],
            Some(all) => {
                let used: Vec<&Ctor> = rows.iter().filter_map(|r| if let Pat::Ctor(c, _) = &r[0] { Some(c) } else { None }).collect();
                let missing: Vec<&(Ctor, Vec<Ty>, String)> = all.iter().filter(|(c, _, _)| !used.contains(&c)).collect();
                if missing.is_empty() {
                    let mut out = Vec::new();
                    for (c, arg_tys, name) in &all {
                        let spec = specialize(rows, c, arg_tys.len());
                        let mut sub: Vec<Ty> = arg_tys.clone();
                        sub.extend(rest.iter().cloned());
                        for w in self.witnesses(&spec, &sub) {
                            let (args, tail) = w.split_at(arg_tys.len());
                            out.push(prepend(ctor_w(c, name, args.to_vec()), tail.to_vec()));
                        }
                        if out.len() > SHOWN {
                            break;
                        }
                    }
                    out
                } else {
                    let def = default_rows(rows);
                    let tails = self.witnesses(&def, rest);
                    let mut out = Vec::new();
                    for tail in tails {
                        for (c, arg_tys, name) in &missing {
                            out.push(prepend(ctor_w(c, name, vec![W::Any; arg_tys.len()]), tail.clone()));
                        }
                        if out.len() > SHOWN {
                            break;
                        }
                    }
                    out
                }
            }
            None => {
                let def = default_rows(rows);
                self.witnesses(&def, rest).into_iter().map(|w| prepend(W::Other, w)).collect()
            }
        }
    }
}

/// The rows that take constructor `c`, with its parts in place of the first column.
fn specialize(rows: &[Vec<Pat>], c: &Ctor, arity: usize) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for r in rows {
        match &r[0] {
            Pat::Ctor(rc, args) if rc == c => {
                let mut row = args.clone();
                row.resize(arity, Pat::Any);
                row.extend(r[1..].iter().cloned());
                out.push(row);
            }
            Pat::Any => {
                let mut row = vec![Pat::Any; arity];
                row.extend(r[1..].iter().cloned());
                out.push(row);
            }
            _ => {}
        }
    }
    out
}

/// The rows whose first column takes every value, without it.
fn default_rows(rows: &[Vec<Pat>]) -> Vec<Vec<Pat>> {
    rows.iter().filter(|r| matches!(r[0], Pat::Any)).map(|r| r[1..].to_vec()).collect()
}

fn prepend(w: W, mut tail: Vec<W>) -> Vec<W> {
    tail.insert(0, w);
    tail
}

fn ctor_w(c: &Ctor, name: &str, args: Vec<W>) -> W {
    match c {
        // `Some(x)` is written as x itself.
        Ctor::Some => args.into_iter().next().unwrap_or(W::Any),
        _ => W::Ctor(name.to_string(), args),
    }
}

fn has_other(w: &W) -> bool {
    match w {
        W::Other => true,
        W::Any => false,
        W::Ctor(_, args) => args.iter().any(has_other),
    }
}

fn w_text(w: &W) -> String {
    match w {
        W::Any | W::Other => "_".into(),
        W::Ctor(name, args) if name.is_empty() => format!("({})", args.iter().map(w_text).collect::<Vec<_>>().join(", ")),
        W::Ctor(name, args) if args.is_empty() || args.iter().all(|a| matches!(a, W::Any)) => name.clone(),
        W::Ctor(name, args) => format!("{name}({})", args.iter().map(w_text).collect::<Vec<_>>().join(", ")),
    }
}
