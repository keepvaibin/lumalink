//! Services in a body (docs/modding/language/language.md): the faces LumaLink reads from the game's SDK headers
//! ([`crate::svc`]) give `use svc.<name>`'s calls with their arguments and results, requests with their
//! closures or `wait`, struct fields, enum cases, constants, the event records, `svc.Answer`, and
//! `on <name>.<kind>` (S0107). Codes S0107, S0108, S0111, S0134 and S0135.

use super::call::P;
use super::expr::fold_int;
use super::{Resolved, Walk, Want};
use crate::ast::{self, Expr, Pos};
use crate::check::{service_type, Ty, SHARED_SERVICE};
use crate::diag::{code, Arg, Diagnostic};
use crate::game::{GameTy, Scalar, Value};
use crate::svc::{case_name, Face, Gives, SParam, STy, Shape, Unfit};

/// `svc.Answer`, the answers of every service.
pub(crate) fn answer_ty() -> Ty {
    Ty::Service { service: "svc".into(), name: "Answer".into() }
}

/// The record an event kind's closure and `on` take: `<Kind>Event` (`time.TimerEvent`).
pub(crate) fn event_name(kind: &str) -> String {
    format!("{}Event", case_name(&kind.to_ascii_uppercase()))
}

/// A C number type as LumaScript's.
fn number_ty(c: &str) -> Ty {
    match c {
        "int8_t" => Ty::Int(Scalar::I8),
        "int16_t" => Ty::Int(Scalar::I16),
        "int32_t" => Ty::Int(Scalar::I32),
        "int64_t" => Ty::Int(Scalar::I64),
        "uint8_t" => Ty::Int(Scalar::U8),
        "uint16_t" => Ty::Int(Scalar::U16),
        "uint32_t" => Ty::Int(Scalar::U32),
        "uint64_t" => Ty::Int(Scalar::U64),
        "float" => Ty::Float(Scalar::F32),
        "double" => Ty::Float(Scalar::F64),
        // The reader gives no other (svc::NUMBERS).
        _ => Ty::Error,
    }
}

/// A face's type as the checker's.
pub(crate) fn sty(service: &str, t: &STy) -> Ty {
    match t {
        STy::Bool => Ty::Bool,
        STy::Number(c) => number_ty(c),
        STy::Text => Ty::Text,
        STy::Bytes => Ty::Bytes,
        STy::Struct(n) | STy::Enum(n) | STy::Handle(n) => Ty::Service { service: service.to_string(), name: n.clone() },
        STy::Vec(2) => Ty::Value(Value::Vec2),
        STy::Vec(3) => Ty::Value(Value::Vec3),
        STy::Vec(_) => Ty::Value(Value::Vec4),
        STy::Shared(c) => Ty::Service { service: SHARED_SERVICE.to_string(), name: c.strip_prefix("Luma").unwrap_or(c).to_string() },
        STy::Actor => Ty::Game(GameTy::View { record: "LiveActor".into(), konst: false }),
        STy::ContentName(kind) => Ty::Content(kind.clone()),
        STy::Opt(x) => Ty::Opt(Box::new(sty(service, x))),
        STy::Color => Ty::Value(Value::Color),
        STy::List(x) => Ty::List(Box::new(sty(service, x))),
        STy::HandleOf(owner, kind) | STy::StructOf(owner, kind) => Ty::Service { service: owner.clone(), name: kind.clone() },
        STy::Fixed(x, n) => Ty::Tuple(vec![sty(service, x); usize::from(*n)]),
        STy::Answer => answer_ty(),
    }
}

/// The outs as one value: nothing, the one, or a tuple.
fn outs(service: &str, o: &[STy]) -> Ty {
    match o {
        [] => Ty::Unit,
        [one] => sty(service, one),
        more => Ty::Tuple(more.iter().map(|t| sty(service, t)).collect()),
    }
}

/// What an event of `kind` carries as `data`: its take function's text or bytes, if the table has one.
fn take_data(face: &Face, kind: &str) -> Option<STy> {
    face.event_data(kind).map(|(d, _)| d)
}

/// The functions that open requests whose events are of `kind`, as a script names them.
fn requests_of(face: &Face, kind: &str) -> Vec<String> {
    face.functions
        .iter()
        .filter(|f| matches!(&f.shape, Shape::Callable { gives: Gives::Request { kind: k, .. }, .. } if k == kind))
        .map(|f| format!("`{}.{}`", face.name, f.name))
        .collect()
}

impl<'w, 'a> Walk<'w, 'a> {
    fn face(&self, service: &str) -> Option<&'a Face> {
        self.game().services.faces.get(service)
    }

    fn no_name_in(&mut self, at: Pos, name: &str, service: &str) -> Ty {
        let d = Diagnostic::new(code::NO_NAME_IN_MODULE, at).with("name", Arg::Text(name.to_string())).with("module", Arg::Text(format!("svc.{service}")));
        self.say(d);
        Ty::Error
    }

    /// Whether `name` is a struct of the service (or of the shared header), not an event's record.
    pub(in crate::check) fn service_struct_named(&self, service: &str, name: &str) -> bool {
        if service == SHARED_SERVICE {
            let c = format!("Luma{name}");
            return self.game().services.faces.values().any(|f| f.shared_structs.contains_key(&c));
        }
        self.face(service).is_some_and(|f| f.structs.contains_key(name))
    }

    /// The fields of a service's struct or event record by its name (`Now`, `TimerEvent`; `Place` of the shared
    /// header): None when `name` is no record.
    pub(super) fn service_fields(&self, service: &str, name: &str) -> Option<Vec<(String, Ty)>> {
        if service == SHARED_SERVICE {
            let c = format!("Luma{name}");
            let fields = self.game().services.faces.values().find_map(|f| f.shared_structs.get(&c))?;
            return Some(fields.iter().map(|(n, t)| (n.clone(), sty(service, t))).collect());
        }
        let face = self.face(service)?;
        if let Some(fields) = face.structs.get(name) {
            return Some(fields.iter().map(|(n, t)| (n.clone(), sty(service, t))).collect());
        }
        let kind = face.events.keys().find(|k| event_name(k) == name)?;
        // status (`none` for LUMA_OK), value and flags; `data` when the kind has a take.
        let mut fields = vec![
            ("status".to_string(), Ty::Opt(Box::new(answer_ty()))),
            ("value".to_string(), Ty::Int(Scalar::U64)),
            ("flags".to_string(), Ty::Int(Scalar::U32)),
        ];
        if let Some(data) = take_data(face, kind) {
            fields.push(("data".to_string(), sty(service, &data)));
        }
        Some(fields)
    }

    /// `x.field` of a value of a service's type.
    pub(super) fn service_field(&mut self, e: &'a Expr, service: &str, record: &str, field: &'a ast::Ident) -> Ty {
        let found = self.service_fields(service, record).and_then(|fs| fs.into_iter().find(|(n, _)| *n == field.name));
        match found {
            Some((_, t)) => {
                self.res.insert(e.id, Resolved::ServiceField { service: service.to_string(), record: record.to_string(), field: field.name.clone() });
                t
            }
            None => {
                let d = Diagnostic::new(code::NO_MEMBER, field.at).with("name", Arg::Text(field.name.clone())).with("found", Arg::Text(format!("{service}.{record}")));
                self.say(d);
                Ty::Error
            }
        }
    }

    /// `<svc>.<name>` as a value: a constant; a function or type is no value; a `set by on` member is S0108.
    pub(super) fn service_member(&mut self, e: &'a Expr, service: &str, name: &'a ast::Ident) -> Ty {
        let Some(face) = self.face(service) else { return Ty::Error };
        if let Some(v) = face.constants.get(&name.name) {
            self.res.insert(e.id, Resolved::ServiceConst { service: service.to_string(), name: name.name.clone(), value: *v });
            return Ty::Int(if u32::try_from(*v).is_ok() { Scalar::U32 } else { Scalar::U64 });
        }
        let shown = format!("{service}.{}", name.name);
        if let Some(f) = face.function(&name.name) {
            if let Shape::SetBy(what) = &f.shape {
                return self.set_by(name.at, &shown, what);
            }
            return self.not_a_value(e.at, &shown, "Script.Kind.Function");
        }
        if service_type(&self.game().services, service, &name.name).is_some() {
            return self.not_a_value(e.at, &shown, "Script.Kind.Type");
        }
        self.no_name_in(name.at, &name.name, service)
    }

    fn set_by(&mut self, at: Pos, shown: &str, what: &str) -> Ty {
        self.say(Diagnostic::new(code::SVC_SET_BY, at).with("name", Arg::Text(shown.to_string())).with("what", Arg::Text(what.to_string())));
        Ty::Error
    }

    /// `on draw <svc>.Point.<Case>`: `<svc>` the draw service (or the name `use`d for it), the case one of its
    /// points (luma_draw.h LumaDrawPoint), resolved at the case's name for the back end.
    pub(super) fn draw_point(&mut self, path: &'a [ast::Ident]) {
        let Some(first) = path.first() else { return };
        // The word `draw` is the service here without a `use` (section 10 writes `on draw draw.Point.HudAfter`), unless
        // the mod names something else so.
        let draw = match self.name_kind(&first.name) {
            super::expr::NameKind::Module(crate::check::ModTarget::Service(s)) => s == "draw",
            super::expr::NameKind::Unknown => first.name == "draw",
            _ => false,
        };
        match path {
            [_, ty, case] if draw && ty.name == "Point" => {
                let Some(cases) = self.face("draw").and_then(|f| f.enums.get("Point")).cloned() else { return };
                match cases.iter().find(|(c, _)| *c == case.name) {
                    Some((c, number)) => {
                        self.named.insert(case.at, Resolved::ServiceCase { service: "draw".into(), ty: "Point".into(), case: c.clone(), number: *number });
                    }
                    None => {
                        let d = Diagnostic::new(code::NO_MEMBER, case.at).with("name", Arg::Text(case.name.clone())).with("found", Arg::Text("draw.Point".into()));
                        self.say(d);
                    }
                }
            }
            _ => {
                let found = path.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(".");
                self.say(Diagnostic::new(code::TYPE_MISMATCH, first.at).with("found", Arg::Text(found)).with("expected", Arg::Text("draw.Point".into())));
            }
        }
    }

    /// `<svc>.<Enum>.<Case>`, `<svc>.Answer.<Case>`.
    pub(super) fn service_case(&mut self, e: &'a Expr, service: &str, ty_name: &'a ast::Ident, case: &'a ast::Ident) -> Ty {
        let services = &self.game().services;
        let (owner, cases): (&str, Vec<(String, i64)>) = if ty_name.name == "Answer" {
            ("svc", services.answers.clone())
        } else {
            match self.face(service).and_then(|f| f.enums.get(&ty_name.name)) {
                Some(cases) => (service, cases.clone()),
                None => {
                    let shown = format!("{service}.{}", ty_name.name);
                    if service_type(services, service, &ty_name.name).is_some() {
                        let d = Diagnostic::new(code::NO_MEMBER, case.at).with("name", Arg::Text(case.name.clone())).with("found", Arg::Text(shown));
                        self.say(d);
                        return Ty::Error;
                    }
                    return self.no_name_in(ty_name.at, &ty_name.name, service);
                }
            }
        };
        match cases.iter().find(|(c, _)| *c == case.name) {
            Some((c, number)) => {
                let ty = Ty::Service { service: owner.to_string(), name: ty_name.name.clone() };
                self.res.insert(e.id, Resolved::ServiceCase { service: owner.to_string(), ty: ty_name.name.clone(), case: c.clone(), number: *number });
                ty
            }
            None => {
                let d = Diagnostic::new(code::NO_MEMBER, case.at).with("name", Arg::Text(case.name.clone())).with("found", Arg::Text(format!("{owner}.{}", ty_name.name)));
                self.say(d);
                Ty::Error
            }
        }
    }

    /// `<svc>.<f>(args)`.
    pub(super) fn service_call(&mut self, e: &'a Expr, service: &str, name: &'a ast::Ident, args: &'a [ast::Arg]) -> Ty {
        // A service the game does not describe: said once, at its `use`.
        let Some(face) = self.face(service) else {
            self.args_any(args);
            return Ty::Error;
        };
        let callee = format!("{service}.{}", name.name);
        let Some(f) = face.function(&name.name) else {
            self.args_any(args);
            if service_type(&self.game().services, service, &name.name).is_some() {
                self.say(Diagnostic::new(code::NOT_CALLABLE, name.at).with("found", Arg::Text(callee)));
                return Ty::Error;
            }
            return self.no_name_in(name.at, &name.name, service);
        };
        let (params, gives) = match &f.shape {
            Shape::Callable { params, gives } => (params, gives),
            Shape::SetBy(what) => {
                self.args_any(args);
                return self.set_by(name.at, &callee, what);
            }
            Shape::Take { .. } => {
                self.args_any(args);
                let d = Diagnostic::new(code::SVC_NOT_CALLABLE, name.at).with("found", Arg::Text(callee)).with("why", Arg::Phrase("Script.Why.SvcTake"));
                self.say(d);
                return Ty::Error;
            }
            Shape::NotCallable(Unfit::Param { param, why, .. }) => {
                self.args_any(args);
                let d = Diagnostic::new(code::SVC_PARAM, name.at).with("found", Arg::Text(callee)).with("param", Arg::Text(param.clone())).with("why", Arg::Phrase(why.phrase()));
                self.say(d);
                return Ty::Error;
            }
            Shape::NotCallable(Unfit::Native) => {
                self.args_any(args);
                let d = Diagnostic::new(code::SVC_NOT_CALLABLE, name.at).with("found", Arg::Text(callee)).with("why", Arg::Phrase("Script.Why.SvcNative"));
                self.say(d);
                return Ty::Error;
            }
            Shape::NotCallable(Unfit::Form(_)) => {
                self.args_any(args);
                let d = Diagnostic::new(code::SVC_NOT_CALLABLE, name.at).with("found", Arg::Text(callee)).with("why", Arg::Phrase("Script.Why.SvcForm"));
                self.say(d);
                return Ty::Error;
            }
        };
        let mut ps: Vec<P> = params
            .iter()
            .map(|p| match p {
                SParam::Value { name, ty } => P::new(name, sty(service, ty)),
                // A function the service calls back: a `fn(args)` value, kept until replaced or the mod's end.
                SParam::Callback { name, args, .. } => P::new(name, Ty::Fn(args.iter().map(|a| sty(service, a)).collect(), Box::new(Ty::Unit))),
                // A request's closure takes its kind's record; a stream's, the service's Event.
                SParam::Closure { optional } => {
                    let record = match gives {
                        Gives::Request { kind, .. } => Ty::Service { service: service.to_string(), name: event_name(kind) },
                        _ => Ty::Service { service: service.to_string(), name: "Event".into() },
                    };
                    let f = Ty::Fn(vec![record], Box::new(Ty::Unit));
                    P { name: None, ty: if *optional { Ty::Opt(Box::new(f)) } else { f }, var: false, default: false }
                }
            })
            .collect();
        // a request closed after its one event may be waited, without its closure.
        let waited = match gives {
            Gives::Request { waitable: true, .. } => {
                let closure_at = params.iter().position(|p| matches!(p, SParam::Closure { .. }));
                let positional = args.iter().filter(|a| a.name.is_none()).count();
                match closure_at {
                    Some(i) if positional == ps.len() - 1 => {
                        ps.remove(i);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        self.args_against(&callee, &ps, args, e.at);
        // S0111: a literal outside a range the member's comment states.
        for (param, low, high) in &f.ranges {
            let named = args.iter().find(|a| a.name.as_ref().is_some_and(|n| n.name == *param));
            let at = ps.iter().position(|p| p.name.as_deref() == Some(param.as_str()));
            let arg = named.or_else(|| at.and_then(|i| args.iter().filter(|a| a.name.is_none()).nth(i)));
            let Some(arg) = arg else { continue };
            if let Some(v) = fold_int(&arg.value) {
                if v < i128::from(*low) || v > i128::from(*high) {
                    let d = Diagnostic::new(code::SVC_RANGE, arg.value.at)
                        .with("name", Arg::Text(callee.clone()))
                        .with("param", Arg::Text(param.clone()))
                        .with("found", Arg::Text(v.to_string()))
                        .with("low", Arg::Text(low.to_string()))
                        .with("high", Arg::Text(high.to_string()));
                    self.say(d);
                }
            }
        }
        self.res.insert(e.id, Resolved::ServiceFn { service: service.to_string(), name: name.name.clone(), waited });
        match gives {
            Gives::Outs(o) => outs(service, o),
            Gives::Answered(o) if o.is_empty() => Ty::Bool,
            Gives::Answered(o) => Ty::Opt(Box::new(outs(service, o))),
            Gives::Number(c) => number_ty(c),
            Gives::Request { kind, .. } if waited => {
                let r = take_data(face, kind).map_or(Ty::Int(Scalar::U64), |d| sty(service, &d));
                Ty::Pending(Box::new(Ty::Opt(Box::new(r))))
            }
            Gives::Request { handle, .. } | Gives::Stream { handle: Some(handle) } => Ty::Opt(Box::new(sty(service, handle))),
            // A watch: as a call with no outs.
            Gives::Stream { handle: None } if f.answers.is_empty() => Ty::Unit,
            Gives::Stream { handle: None } => Ty::Bool,
        }
    }

    /// `<svc>.<T> { field: value, ... }`: a service's struct or the shared header's, every field given (the compiler
    /// sets `size` and zeroes `reserved`).
    pub(super) fn service_struct_lit(&mut self, at: Pos, service: &str, ty_name: &'a ast::Ident, fields: &'a [(ast::Ident, Option<Expr>)]) -> Ty {
        let ty = service_type(&self.game().services, service, &ty_name.name);
        let (owner, record) = match &ty {
            Some(Ty::Service { service: s, name }) => (s.clone(), name.clone()),
            _ => {
                self.no_name_in(ty_name.at, &ty_name.name, service);
                return self.struct_quiet(fields);
            }
        };
        let shown = format!("{owner}.{record}");
        let decl = match self.face(&owner).map(|f| f.structs.contains_key(&record)) {
            Some(true) => self.service_fields(&owner, &record),
            _ if owner == SHARED_SERVICE => self.service_fields(&owner, &record),
            _ => None,
        };
        let Some(decl) = decl else {
            self.say(Diagnostic::new(code::NOT_A_STRUCT, at).with("found", Arg::Text(shown)));
            return self.struct_quiet(fields);
        };
        let mut given: Vec<&str> = Vec::new();
        for (ident, value) in fields {
            let Some((_, fty)) = decl.iter().find(|(n, _)| *n == ident.name) else {
                self.say(Diagnostic::new(code::NO_MEMBER, ident.at).with("name", Arg::Text(ident.name.clone())).with("found", Arg::Text(shown.clone())));
                if let Some(v) = value {
                    self.expr(v, Want::Ty(&Ty::Error));
                }
                continue;
            };
            if given.contains(&ident.name.as_str()) {
                self.say(Diagnostic::new(code::ARG_TWICE, ident.at).with("name", Arg::Text(ident.name.clone())));
            }
            given.push(&ident.name);
            self.named.insert(ident.at, Resolved::ServiceField { service: owner.clone(), record: record.clone(), field: ident.name.clone() });
            match value {
                Some(v) => {
                    self.expr(v, Want::Ty(fty));
                }
                None => {
                    let found = self.shorthand(ident);
                    self.check_fit(ident.at, found, fty);
                }
            }
        }
        // A pointer field (svc::FieldPtr: text, bytes, a list) left out is null: the service's "null: ..." default.
        let pointers: Vec<String> = self.face(&owner).and_then(|f| f.field_ptrs.get(&record)).map(|p| p.keys().cloned().collect()).unwrap_or_default();
        for (n, _) in &decl {
            if !given.contains(&n.as_str()) && !pointers.contains(n) {
                self.say(Diagnostic::new(code::FIELD_MISSING, at).with("name", Arg::Text(n.clone())).with("found", Arg::Text(shown.clone())));
            }
        }
        Ty::Service { service: owner, name: record }
    }

    /// `on <svc>.<kind>(e)`: a mod-wide event of the table binds its record; a kind whose events go to a request's
    /// function is S0107.
    pub(super) fn service_on(&mut self, service: &str, kind: &'a ast::Ident, names: Option<&'a [ast::Ident]>) {
        let bind_error = |w: &mut Self| {
            for n in names.unwrap_or(&[]) {
                w.bind(n, Ty::Error, false);
            }
        };
        let Some(face) = self.face(service) else { return bind_error(self) };
        if !face.events.contains_key(&kind.name) {
            self.no_name_in(kind.at, &kind.name, service);
            return bind_error(self);
        }
        let requests = requests_of(face, &kind.name);
        if !requests.is_empty() {
            let d = Diagnostic::new(code::SVC_PER_REQUEST, kind.at).with("event", Arg::Text(format!("{service}.{}", kind.name))).with("call", Arg::Text(requests.join(", ")));
            self.say(d);
            return bind_error(self);
        }
        let record = Ty::Service { service: service.to_string(), name: event_name(&kind.name) };
        match names.unwrap_or(&[]) {
            [] => {}
            [one] => {
                self.bind(one, record, false);
            }
            [first, rest @ ..] => {
                self.bind(first, record, false);
                for n in rest {
                    self.bind(n, Ty::Error, false);
                }
                let d = Diagnostic::new(code::ARG_COUNT, rest[0].at)
                    .with("name", Arg::Text(format!("on {service}.{}", kind.name)))
                    .with("found", Arg::Text((rest.len() + 1).to_string()))
                    .with("expected", Arg::Text("1".into()));
                self.say(d);
            }
        }
    }
}
