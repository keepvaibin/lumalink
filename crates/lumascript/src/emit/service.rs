//! Calls of a service's functions (docs/modding/language/language.md).
//!
//! Each service a mod calls is a kind 5 import (`LUMA_SERVICE(slot, "<name>", "<major>.<minor>")`, docs/modding/language/compiled-form.md
//! and docs/modding/lumalink/loader.md): the slot is the table, its header is included in `lm.h`, and a call is
//! `slot-><member>(luma_rt_self, args..., &outs...)`, its parameters walked as [`crate::svc`]'s face reading walked
//! them. The service reports the mod's own mistakes; a mistake gives zeros or nothing. The one run-time error
//! of a call is a text holding a NUL (S0918, lumart_text_cstr).
//!
//! Written so far: values (bool, numbers, the service's enums and handles, an actor's view, colours), `text` without
//! and with its size, structs of plain fields (by value, by `const T*`, `T?` for a pointer that may be null, and as
//! outs, `size` set), callbacks (`X fn, void* user`: the script's `fn` value, kept until the mod ends, called through
//! a trampoline), outs of those kinds, every result form, out buffers and arrays (C1b, C1c), and requests (C1a): with
//! a closure, kept until the mod ends and called with the event's record; waited, a one-step task that parks until
//! the event. Content names and stream events are S0800 at the call until they are written.

use super::{c_string, lost, Ex, Fx, Gen};
use crate::ast::{self, Expr};
use crate::check::{Ty, SHARED_SERVICE};
use crate::svc::{array_in, array_out, camel, case_name, floats_in, items_by, length_out, lent_out, pixels_in, sized_in_32, FieldPtr, Gives, SParam, STy, Shape, CT};

/// A face's type as the back end writes it: the checker's ([`crate::check::service_sty`]) with a content name (C2's
/// `LumaModelName`) as the text it is at run time.
pub(super) fn service_sty(service: &str, t: &STy) -> Ty {
    match t {
        STy::ContentName(_) => Ty::Text,
        STy::Opt(x) if matches!(**x, STy::ContentName(_)) => Ty::Opt(Box::new(Ty::Text)),
        STy::List(x) if matches!(**x, STy::ContentName(_)) => Ty::List(Box::new(Ty::Text)),
        _ => crate::check::service_sty(service, t),
    }
}

/// How one C parameter is passed.
enum Pass {
    /// An argument: the script's value of the face's type, then how it is written.
    Arg(STy, ArgForm),
    /// An out: a local of the header's C type, passed by address.
    Out(STy, String),
    /// A function the service calls back: the script's `fn` value as `user`, a trampoline as the function.
    Callback { args: Vec<STy>, c_args: Vec<CT>, c_fn: String },
    /// An out buffer (`char*|void* buf, uint64_t size`): `text`, `bytes` or `list<text>`; or an array out
    /// (`T* out, uint32_t capacity, uint32_t* count`, C1c): `list<T>`. Called twice: the length, then the fill.
    Buffer { ty: STy, form: Form },
    /// A request's `LumaEventFn fn, void* user`: the closure given, or a waited request's box.
    Event,
    /// Bytes the service lends (`const void** data, uint64_t* bytes`, [`lent_out`]): copied for LUMA_OK.
    Lent,
}

/// How an out buffer's length comes back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// The `int64_t` result (`mods.option`, `save.get`); `nul`: a text the service ends with a NUL, which needs a
    /// byte of room more than its length.
    Result { nul: bool },
    /// The `uint64_t* length` after the size (`mods.kit_read`), with an `int32_t` status; `narrow`: a `uint32_t` size
    /// and length (`input.text_peek`); `nul`: the size counts the text's 0 byte (the comment's "size - 1").
    LengthOut { narrow: bool, nul: bool },
    /// An array: the `uint32_t*` count after the capacity, with an `int32_t` status (`mods.needing`).
    Count,
}

/// Where an out buffer's arguments go in the argument list, filled when the call is written (twice: once to ask the
/// length, once to fill).
const BUFFER_ARGS: &str = "\u{1}buffer\u{1}";

#[derive(Clone)]
enum ArgForm {
    /// As the value is.
    Value,
    /// `const char*`, NUL-ended (a NUL inside: S0918).
    CText,
    /// `const char*`/`const void*`/`const uint8_t*` then `uint64_t`: bytes and length.
    Sized,
    /// `const T*` of a struct: the value's address (`T?`: null for none).
    Ref,
    /// `const T* x, uint32_t count`: a list's elements and their number (C1c).
    List,
    /// `const T x[N]`, T the C element type: a vec's floats or a tuple's values, as an array made in the call.
    Array(String),
    /// `uint32_t w, uint32_t h, const void* rgba8` ([`pixels_in`]): the bytes, their length w x h x 4 checked.
    Pixels,
    /// `const void*` then `uint32_t size` ([`sized_in_32`]): the bytes and their length, which must fit 32 bits.
    Sized32,
    /// `const N* x, uint64_t count` whose count another member gives per item ([`items_by`]): the numbers, and their
    /// number over that member's answer for the argument of pass `handle`; a length not a whole number of items is a
    /// script error naming both numbers.
    Items { by: String, handle: usize, count: String },
}

impl Gen<'_> {
    /// A service's enum or `svc.Answer`, one C `int32_t`: `==` compares it as C does (handles take `same`).
    pub(super) fn service_number(&self, service: &str, name: &str) -> bool {
        self.service_struct(service, name).is_none() && self.service_c_type(service, name).as_deref() == Some("int32_t")
    }

    /// The C type of a service's value: a handle is a LumaHandle, an enum and `svc.Answer` are int32_t, a struct of
    /// plain fields the header's own.
    pub(super) fn service_c_type(&self, service: &str, name: &str) -> Option<String> {
        if service == "svc" && name == "Answer" {
            return Some("int32_t".into());
        }
        if let Some((c, _)) = self.service_struct(service, name) {
            return Some(c);
        }
        let face = self.game.services.faces.get(service)?;
        if name == "Request" || face.handles.iter().any(|h| h == name) {
            Some("uint64_t".into())
        } else if face.enums.contains_key(name) {
            Some("int32_t".into())
        } else {
            None
        }
    }

    /// The event kind whose record `name` is (`ReadEvent` of fs's READ), and what its take gives as `data`.
    fn event_kind(&self, service: &str, name: &str) -> Option<(String, Option<STy>)> {
        let face = self.game.services.faces.get(service)?;
        let kind = face.events.keys().find(|k| format!("{}Event", case_name(&k.to_ascii_uppercase())) == name)?;
        Some((kind.clone(), face.event_data(kind).map(|(d, _)| d)))
    }

    /// A service type's C type ([`Self::service_c_type`]), an event record's included: made once,
    /// `lm_ev_<service>_<kind>` holding `status` (`svc.Answer?`), `value`, `flags` and the taken `data`.
    pub(super) fn service_type_c(&mut self, service: &str, name: &str) -> Option<String> {
        if let Some(face) = self.game.services.faces.get(service) {
            if !face.header.is_empty() {
                self.svc_type_headers.insert(face.header.clone());
            }
        }
        if name == "Event" && self.game.services.faces.get(service).is_some_and(|f| !f.stream.is_empty()) {
            return self.stream_c_type(service);
        }
        if let Some(c) = self.service_c_type(service, name) {
            return Some(c);
        }
        if let Some((c, _, _)) = self.text_struct(service, name) {
            return Some(c);
        }
        let Some((kind, data)) = self.event_kind(service, name) else { return None };
        let key = format!("event {service} {kind}");
        if let Some(n) = self.svc_events.get(&key) {
            return Some(n.clone());
        }
        let status = self.c_type(&Ty::Opt(Box::new(Ty::Service { service: "svc".into(), name: "Answer".into() })))?;
        let data = match &data {
            Some(d) => Some(self.c_type(&service_sty(service, d))?),
            None => None,
        };
        let c = format!("lm_ev_{}_{kind}", super::module_c(service));
        let field = data.as_ref().map(|d| format!("\n    {d} data;")).unwrap_or_default();
        self.typedefs.push(format!("typedef struct {c} {{\n    {status} status;\n    uint64_t value;\n    uint32_t flags;{field}\n}} {c};"));
        if let Some(d) = data {
            self.counted_helpers(&c, &[(None, vec![("data".to_string(), d)])]);
        }
        self.svc_events.insert(key, c.clone());
        Some(c)
    }

    /// `void lm_evf_<service>_<kind>(r, ev, file, line)`: an event's record from the LumaEvent: status `none`
    /// for LUMA_OK, else its answer; value; flags; and, for a kind with a take, the take called here, inside the
    /// event's call, once, with room for the length the event's value gives (its comment: "value = the file's bytes";
    /// a text one byte more, for take_text's NUL). No memory for it: status `Memory` and no data. A take whose result
    /// is not that length (system.clipboard's take_image) is not written yet: None.
    fn event_fill(&mut self, service: &str, kind: &str, slot: &str) -> Option<String> {
        let key = format!("fill {service} {kind}");
        if let Some(n) = self.svc_events.get(&key) {
            return Some(n.clone());
        }
        let record = format!("{}Event", case_name(&kind.to_ascii_uppercase()));
        let rc = self.service_type_c(service, &record)?;
        let (_, data) = self.event_kind(service, &record)?;
        let name = format!("lm_evf_{}_{kind}", super::module_c(service));
        let head = format!("void {name}({rc}* r, const LumaEvent* ev, const char* file, uint32_t line)");
        let mut body = String::from(
            "    r->value = ev->value;\n    r->flags = ev->flags;\n    if (ev->status != LUMA_OK) {\n        r->status.some = 1;\n        r->status.value = ev->status;\n        return;\n    }\n",
        );
        let face = self.game.services.faces.get(service)?;
        let take = face.event_data(kind).and_then(|(_, t)| t).cloned();
        if let (Some(data), None) = (&data, &take) {
            // The event's own bytes (LumaEvent.data, valid during the call), copied.
            let fill = if *data == STy::Text {
                "lumart_text_make(&r->data, (const char*)ev->data, ev->data_size, file, line) != 0".to_string()
            } else {
                "lumart_list_own(&r->data, &lumart_u8, ev->data_size, file, line) != 0".to_string()
            };
            let copy = if *data == STy::Text { "" } else { " else {\n        __builtin_memcpy(lumart_bytes_p(r->data), ev->data, ev->data_size);\n        r->data->len = ev->data_size;\n    }" };
            body.push_str(&format!(
                "    if (ev->data == 0 || ev->data_size == 0) return;\n    if ({fill}) {{\n        r->status.some = 1;\n        r->status.value = LUMA_E_MEMORY;\n    }}{copy}\n"
            ));
        }
        if let (Some(data), Some(take)) = (data, take) {
            let (r, ps) = (&take.c.result, &take.c.params);
            if !(r.base == "int64_t" && r.ptrs == 0 && ps.len() == 4) {
                return None;
            }
            let buf = c_text(&ps[2].ty);
            let nul = u8::from(data == STy::Text);
            body.push_str(&format!(
                "    if (ev->value == 0) return;\n    lumart_bytes b = 0;\n    int64_t got = LUMA_E_MEMORY;\n    if (ev->value < 0x7fffffffffffffffull && lumart_list_own(&b, &lumart_u8, ev->value + {nul}u, file, line) == 0) {{\n        got = {slot}->{}(luma_rt_self, ev->handle, ({buf})lumart_bytes_p(b), ev->value + {nul}u);\n    }}\n    if (got >= 0) {{\n        const uint64_t n = (uint64_t)got < ev->value ? (uint64_t)got : ev->value;\n",
                take.name
            ));
            if data == STy::Text {
                body.push_str("        if (lumart_text_make(&r->data, (const char*)lumart_bytes_p(b), n, file, line) != 0) got = LUMA_E_MEMORY;\n    }\n");
            } else {
                body.push_str("        b->len = n;\n        r->data = b;\n        return;\n    }\n");
            }
            body.push_str("    if (got < 0) {\n        r->status.some = 1;\n        r->status.value = (int32_t)got;\n    }\n    lm_rel_lumart_bytes(&b);\n");
        }
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!("{head} {{\n{body}}}\n"));
        self.svc_events.insert(key, name.clone());
        Some(name)
    }

    /// A request's trampoline, `void lm_evt_<m>_<n>(void* user, const LumaEvent* ev)`: the record filled, then the
    /// closure `user` called with it as `callback_trampoline` calls (a script error in it ends that call only); the
    /// record given back after. The closure is kept until the mod ends (lumart_fn_keep), past the request's end.
    fn event_trampoline(&mut self, m: usize, service: &str, kind: &str, slot: &str, file: &str, line: u32) -> Option<String> {
        let record = Ty::Service { service: service.to_string(), name: format!("{}Event", case_name(&kind.to_ascii_uppercase())) };
        let rc = self.c_type(&record)?;
        let fill = self.event_fill(service, kind, slot)?;
        let cast = self.fn_cast(&[record], &Ty::Unit)?;
        let n = self.closure_n;
        self.closure_n += 1;
        let name = format!("lm_evt_{m}_{n}");
        let head = format!("void {name}(void* user, const LumaEvent* ev)");
        let rel = if super::counted(&rc) { format!("\n    lm_rel_{rc}(&r);") } else { String::new() };
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!(
            "{head} {{\n    lumart_fn f = (lumart_fn)user;\n    {rc} r = ({rc}){{0}};\n    {fill}(&r, ev, {}, {line}u);\n    (void)(({cast})(f->code))(f, r);{rel}\n}}\n",
            c_string(file)
        ));
        Some(name)
    }

    /// A waited request's parts for value type `of` (`T?`), made once each: the box `lm_rqb_<n>` (`done`, then the
    /// value), a counted block shared by the waiting task's frame `lm_rqf_<n>` and the trampoline; the step
    /// `lm_rqs_<n>`, which parks until `done`, then keeps the value as the task's result and ends. A task stopped first
    /// gives its count back: the trampoline, seeing its own the only one left, writes nothing.
    fn waited_parts(&mut self, of: &Ty) -> Option<(String, String, String)> {
        let vc = self.c_type(of)?;
        let key = format!("waited {vc}");
        if let Some(n) = self.svc_events.get(&key) {
            return Some((format!("lm_rqb_{n}"), format!("lm_rqf_{n}"), format!("lm_rqs_{n}")));
        }
        let d = self.desc(of)?;
        let n = self.svc_events.len().to_string();
        let (bx, fr, st) = (format!("lm_rqb_{n}"), format!("lm_rqf_{n}"), format!("lm_rqs_{n}"));
        self.typedefs.push(format!(
            "typedef struct {bx} {{\n    uint8_t done;\n    {vc} v;\n}} {bx};\ntypedef struct {fr} {{\n    lumart_task_head h;\n    lumart_bytes box;\n}} {fr};"
        ));
        let rel = if super::counted(&vc) { format!("\n    lm_rel_{vc}(&b->v);") } else { String::new() };
        let head = format!("int32_t {st}(void* p)");
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!(
            "{head} {{\n    {fr}* lm_f = ({fr}*)p;\n    {bx}* b = ({bx}*)lumart_bytes_p(lm_f->box);\n    int32_t r = 0;\n    if (!lm_f->h.stop) {{\n        if (b->done == 0) return LUMART_PARKED;\n        if (lumart_task_result_keep(lm_f, &b->v, {d}, \"\", 0u) != 0) r = 1;\n    }}{rel}\n    lm_rel_lumart_bytes(&lm_f->box);\n    return r;\n}}\n"
        ));
        self.tasks_used = true;
        self.svc_events.insert(key, n);
        Some((bx, fr, st))
    }

    /// A waited request's trampoline, `void lm_evw_<m>_<n>(void* user, const LumaEvent* ev)`: `user` is the box with
    /// the trampoline's own count. While the task still holds the box, the record's data (else its value) goes in as
    /// the value, `none` for any answer; `done` is set, and the count given back (the request's one event: it is
    /// closed after it).
    fn waited_trampoline(&mut self, m: usize, service: &str, kind: &str, slot: &str, bx: &str, file: &str, line: u32) -> Option<String> {
        let record = Ty::Service { service: service.to_string(), name: format!("{}Event", case_name(&kind.to_ascii_uppercase())) };
        let rc = self.c_type(&record)?;
        let fill = self.event_fill(service, kind, slot)?;
        let has_data = self.event_kind(service, &format!("{}Event", case_name(&kind.to_ascii_uppercase())))?.1.is_some();
        let n = self.closure_n;
        self.closure_n += 1;
        let name = format!("lm_evw_{m}_{n}");
        let head = format!("void {name}(void* user, const LumaEvent* ev)");
        // The data moves into the box (the record no longer holds it); a value is copied.
        let set = if has_data { "b->v.value = r.data;\n            r.data = 0;" } else { "b->v.value = r.value;" };
        let rel = if super::counted(&rc) { format!("\n        lm_rel_{rc}(&r);") } else { String::new() };
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!(
            "{head} {{\n    lumart_bytes box = (lumart_bytes)user;\n    {bx}* b = ({bx}*)lumart_bytes_p(box);\n    if (box->rc > 1 && b->done == 0) {{\n        {rc} r = ({rc}){{0}};\n        {fill}(&r, ev, {}, {line}u);\n        if (!r.status.some) {{\n            b->v.some = 1;\n            {set}\n        }}\n        b->done = 1;{rel}\n    }}\n    lm_rel_lumart_bytes(&box);\n}}\n",
            c_string(file)
        ));
        Some(name)
    }

    /// A service's struct holding text (`char host[48]`, 's `from`): the script's copy `lm_ss_<service>_<T>`, its
    /// text a counted `text`, made from the header's struct by `lm_sc_<service>_<T>(d, s, file, line)` (the text up to
    /// its NUL, within the array). Its fields are text and the plain values that are copied as they are (numbers,
    /// bool, the service's enums and handles); None for any other.
    fn text_struct(&mut self, service: &str, name: &str) -> Option<(String, String, Vec<(String, STy)>)> {
        let fields = self.text_struct_fields(service, name)?;
        let c = self.game.services.faces.get(service)?.struct_c.get(name)?.clone();
        let script = format!("lm_ss_{}_{name}", super::module_c(service));
        let make = format!("lm_sc_{}_{name}", super::module_c(service));
        let key = format!("text struct {service} {name}");
        if self.svc_events.contains_key(&key) {
            return Some((script, make, fields));
        }
        let mut members = Vec::new();
        let mut parts = Vec::new();
        let mut body = String::new();
        // To C (lm_sw_): zeroed, `size` set, each field written back; text checked to fit its array with its NUL.
        let mut back = format!("    *d = ({c}){{0}};\n");
        if self.service_sized(&c) {
            back.push_str(&format!("    d->size = (uint32_t)sizeof({c});\n"));
        }
        let shown = format!("{service}.{name}");
        let ptrs = self.game.services.faces.get(service)?.field_ptrs.get(name).cloned().unwrap_or_default();
        for (f, t) in &fields {
            // A pointer field (svc::FieldPtr): read as a copy, written as pointers into the script's value for the call.
            if let Some(how) = ptrs.get(f) {
                let ct = self.c_type(&service_sty(service, t))?;
                members.push(format!("    {ct} {f};"));
                parts.push((f.clone(), ct.clone()));
                let (fname, param) = (c_string(&shown), c_string(f));
                match how {
                    FieldPtr::Text => {
                        body.push_str(&format!("    if (lumart_text_from_game(&d->{f}, s->{f}, file, line) != 0) return 1;\n"));
                        // Empty text is null: the service's "null: ..." (a method's GET, an empty start).
                        back.push_str(&format!(
                            "    if (lumart_text_n(s->{f}) != 0 && (d->{f} = lumart_text_cstr(s->{f}, file, line, {fname}, {param})) == 0) return 1;\n"
                        ));
                    }
                    FieldPtr::Bytes { len } => {
                        body.push_str(&format!("    if (lumart_bytes_from(&d->{f}, s->{f}, (uint64_t)s->{len}, file, line) != 0) return 1;\n"));
                        back.push_str(&format!(
                            "    d->{len} = lumart_list_n(s->{f});\n    d->{f} = d->{len} != 0 ? (const void*)lumart_bytes_p(s->{f}) : 0;\n"
                        ));
                    }
                    FieldPtr::Numbers { count, elem_c } => {
                        let desc = self.desc(&service_sty(service, &STy::Number(elem_c.clone())))?;
                        body.push_str(&format!(
                            "    if (s->{f} != 0) {{\n        for (uint64_t i = 0; i < (uint64_t)s->{count}; i++) {{\n            if (lumart_list_push(&d->{f}, {desc}, &s->{f}[i], file, line) != 0) return 1;\n        }}\n    }}\n"
                        ));
                        // The count is the list's own: never a number the script set.
                        back.push_str(&format!(
                            "    {{\n        const uint64_t n = lumart_list_n(s->{f});\n        if (n > 0xFFFFFFFFu) return lumart_overflow(file, line, \"u32\", {param});\n        d->{f} = n != 0 ? (const {elem_c}*)lumart_list_at(s->{f}, 0) : 0;\n        d->{count} = (uint32_t)n;\n    }}\n"
                        ));
                    }
                    FieldPtr::Rows { stride, height } => {
                        body.push_str(&format!(
                            "    if (lumart_bytes_from(&d->{f}, s->{f}, (uint64_t)s->{stride} * (uint64_t)s->{height}, file, line) != 0) return 1;\n"
                        ));
                        let len = c_string(&format!("{f}.len()"));
                        back.push_str(&format!(
                            "    {{\n        const uint64_t want = (uint64_t)s->{stride} * (uint64_t)s->{height};\n        if (lumart_list_n(s->{f}) != want) return lumart_wrong_length(file, line, {fname}, {len}, lumart_list_n(s->{f}), want);\n        d->{f} = want != 0 ? (const void*)lumart_bytes_p(s->{f}) : 0;\n    }}\n"
                        ));
                    }
                    FieldPtr::List { count, elem, elem_c } => {
                        let item = service_sty(service, &STy::Struct(elem.clone()));
                        let desc = self.desc(&item)?;
                        let plain = self.service_struct(service, elem).is_some();
                        if plain {
                            body.push_str(&format!(
                                "    for (uint64_t i = 0; i < (uint64_t)s->{count}; i++) {{\n        if (lumart_list_push(&d->{f}, {desc}, &s->{f}[i], file, line) != 0) return 1;\n    }}\n"
                            ));
                            back.push_str(&format!(
                                "    {{\n        const uint64_t n = lumart_list_n(s->{f});\n        {elem_c}* a = 0;\n        if (n > 0xFFFFFFFFu) return lumart_overflow(file, line, \"u32\", {param});\n        if (n != 0 && (a = ({elem_c}*)lumart_scratch_zeroed(n * sizeof({elem_c}), file, line)) == 0) return 1;\n        for (uint64_t i = 0; i < n; i++) a[i] = *(const {elem_c}*)lumart_list_at(s->{f}, i);\n        d->{f} = a;\n        d->{count} = (uint32_t)n;\n    }}\n"
                            ));
                        } else {
                            let (es, emake, _) = self.text_struct(service, elem)?;
                            let ewrite = format!("lm_sw_{}_{elem}", super::module_c(service));
                            let rel = if super::counted(&es) { format!("lm_rel_{es}(&e); ") } else { String::new() };
                            body.push_str(&format!(
                                "    for (uint64_t i = 0; i < (uint64_t)s->{count}; i++) {{\n        {es} e = {{0}};\n        if ({emake}(&e, &s->{f}[i], file, line) != 0 || lumart_list_push(&d->{f}, {desc}, &e, file, line) != 0) {{ {rel}return 1; }}\n        {rel}\n    }}\n"
                            ));
                            back.push_str(&format!(
                                "    {{\n        const uint64_t n = lumart_list_n(s->{f});\n        {elem_c}* a = 0;\n        if (n > 0xFFFFFFFFu) return lumart_overflow(file, line, \"u32\", {param});\n        if (n != 0 && (a = ({elem_c}*)lumart_scratch_zeroed(n * sizeof({elem_c}), file, line)) == 0) return 1;\n        for (uint64_t i = 0; i < n; i++) {{\n            if ({ewrite}(&a[i], (const {es}*)lumart_list_at(s->{f}, i), file, line) != 0) return 1;\n        }}\n        d->{f} = a;\n        d->{count} = (uint32_t)n;\n    }}\n"
                            ));
                        }
                    }
                }
                continue;
            }
            // An actor is kept as a handle (a view is not kept, language rule 3): read back as the view while the object
            // lives, S0903 once it has gone; given to the service as null then (the service refuses it, reported).
            let ct = if *t == STy::Actor { "lumart_ref".to_string() } else { self.c_type(&service_sty(service, t))? };
            members.push(format!("    {ct} {f};"));
            parts.push((f.clone(), ct.clone()));
            body.push_str(&match t {
                STy::Text => format!(
                    "    {{\n        uint64_t n = 0;\n        while (n < sizeof s->{f} && s->{f}[n] != 0) n++;\n        if (lumart_text_make(&d->{f}, s->{f}, n, file, line) != 0) return 1;\n    }}\n"
                ),
                STy::Bool => format!("    d->{f} = (uint8_t)(s->{f} != 0);\n"),
                STy::Vec(n) => {
                    let parts: Vec<String> = (0..*n).map(|i| format!("s->{f}[{i}]")).collect();
                    format!("    d->{f} = (lm_vec{n}){{{}}};\n", parts.join(", "))
                }
                STy::Color => format!("    d->{f} = lumart_color_of_rgba8(s->{f});\n"),
                STy::Actor => {
                    let nameobj = self.class_slot("NameObj");
                    format!("    if (s->{f} != 0 && lumart_ref_make(&d->{f}, s->{f}, {nameobj}, file, line) != 0) return 1;\n")
                }
                _ => format!("    d->{f} = ({ct})s->{f};\n"),
            });
            back.push_str(&match t {
                STy::Text => {
                    let (fname, param) = (c_string(&shown), c_string(f));
                    let whole = c_string(&format!("{shown}.{f}"));
                    format!(
                        "    {{\n        const char* t = lumart_text_cstr(s->{f}, file, line, {fname}, {param});\n        if (t == 0) return 1;\n        const uint64_t n = lumart_text_n(s->{f});\n        if (n >= sizeof d->{f}) {{\n            char room[32] = \"char[\";\n            uint64_t v = sizeof d->{f}, k = 5, w = 1;\n            while (w * 10 <= v) w *= 10;\n            for (; w > 0; w /= 10) room[k++] = (char)('0' + (v / w) % 10);\n            room[k] = ']';\n            const char* const a[2] = {{{whole}, room}};\n            return lumart_raise(file, line, 908, 2, a);\n        }}\n        __builtin_memcpy(d->{f}, t, n);\n    }}\n"
                    )
                }
                STy::Bool => format!("    d->{f} = s->{f} != 0;\n"),
                STy::Vec(n) => (0..*n as usize).map(|i| format!("    d->{f}[{i}] = s->{f}.{};\n", ["x", "y", "z", "w"][i])).collect(),
                STy::Color => format!("    d->{f} = lumart_color_rgba8(s->{f});\n"),
                STy::Actor => {
                    let cls = self.class_slot("LiveActor");
                    format!("    d->{f} = lumart_ref_get(s->{f}, {cls});\n")
                }
                _ => format!("    d->{f} = s->{f};\n"),
            });
        }
        self.typedefs.push(format!("typedef struct {script} {{\n{}\n}} {script};", members.join("\n")));
        self.counted_helpers(&script, &[(None, parts)]);
        let head = format!("int32_t {make}({script}* d, const {c}* s, const char* file, uint32_t line)");
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!("{head} {{\n{body}    return 0;\n}}\n"));
        let write = format!("lm_sw_{}_{name}", super::module_c(service));
        let head = format!("int32_t {write}({c}* d, const {script}* s, const char* file, uint32_t line)");
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!("{head} {{\n{back}    return 0;\n}}\n"));
        self.svc_events.insert(key, script.clone());
        Some((script, make, fields))
    }

    /// `lm_sw_<service>_<T>(d, s, file, line)`: the script's copy of a text struct ([`Self::text_struct`]) written as
    /// the header's struct for a call; 1 after a script error it reported (S0918 a NUL, S0908 a text longer than its
    /// array holds).
    fn text_struct_write(&mut self, service: &str, name: &str) -> Option<(String, String)> {
        self.text_struct(service, name)?;
        let c = self.game.services.faces.get(service)?.struct_c.get(name)?.clone();
        Some((c, format!("lm_sw_{}_{name}", super::module_c(service))))
    }

    /// For a parameter of a text struct (or `T?` of one, `opt`): the header's C struct and its lm_sw_ writer.
    fn text_struct_arg(&mut self, service: &str, ty: &STy) -> Option<(String, String, bool)> {
        let (inner, opt) = match ty {
            STy::Opt(x) => (&**x, true),
            x => (x, false),
        };
        let (owner, n) = match inner {
            STy::Struct(n) => (service, n.as_str()),
            STy::StructOf(o, n) => (o.as_str(), n.as_str()),
            _ => return None,
        };
        if self.service_struct(owner, n).is_some() {
            return None;
        }
        let (c, write) = self.text_struct_write(owner, n)?;
        Some((c, write, opt))
    }

    /// A struct of the header holding text (`char host[48]`) or an actor, whose fields are all text, numbers, bools,
    /// enums, handles, vectors, colours or actors: its fields, which [`Self::text_struct`] gives the script as its own
    /// values. One of plain fields only is the header's struct as it is ([`Self::service_struct`]).
    pub(super) fn text_struct_fields(&self, service: &str, name: &str) -> Option<Vec<(String, STy)>> {
        let fields = self.game.services.faces.get(service)?.structs.get(name)?;
        // A list field's items are this face's structs, plain or text structs themselves (svc::FieldPtr::List).
        let item_ok = |t: &STy| match t {
            STy::Struct(n) => self.service_struct(service, n).is_some() || self.text_struct_fields(service, n).is_some(),
            STy::Number(_) => true,
            _ => false,
        };
        let plain = fields.iter().all(|(_, t)| match t {
            STy::Text | STy::Number(_) | STy::Bool | STy::Enum(_) | STy::Handle(_) | STy::Vec(_) | STy::Color | STy::Actor | STy::Bytes => true,
            STy::List(x) => item_ok(x),
            _ => false,
        });
        (plain && fields.iter().any(|(_, t)| matches!(t, STy::Text | STy::Actor | STy::Bytes | STy::List(_)))).then(|| fields.clone())
    }

    /// `<service>.Event`: `lm_se_<service>`, the kind's number in `kind`, then a member per field any kind lists
    /// (a field has one type in every kind, the reader's rule; a kind's members not its own stay zero).
    fn stream_c_type(&mut self, service: &str) -> Option<String> {
        let key = format!("stream {service}");
        if let Some(n) = self.svc_events.get(&key) {
            return Some(n.clone());
        }
        let face = self.game.services.faces.get(service)?;
        let mut fields: Vec<(String, STy)> = Vec::new();
        for k in face.stream.values() {
            for (n, t) in &k.fields {
                match fields.iter().find(|(f, _)| f == n) {
                    Some((_, old)) if old != t => return None,
                    Some(_) => {}
                    None => fields.push((n.clone(), t.clone())),
                }
            }
        }
        let c = format!("lm_se_{}", super::module_c(service));
        let mut members = vec!["    int32_t kind;".to_string()];
        let mut parts = Vec::new();
        for (n, t) in &fields {
            let ct = self.c_type(&service_sty(service, t))?;
            members.push(format!("    {ct} {n};"));
            parts.push((n.clone(), ct));
        }
        self.typedefs.push(format!("typedef struct {c} {{\n{}\n}} {c};", members.join("\n")));
        self.counted_helpers(&c, &[(None, parts)]);
        self.svc_events.insert(key, c.clone());
        Some(c)
    }

    /// `uint8_t lm_esf_<service>(r, ev, file, line)`: a stream event's case from the LumaEvent, its take called
    /// here, inside the event's call: `data` whole (the length asked with a null buffer first, or the take's room and
    /// then a block of the length it gave), `from` the take's out. 0 for an event the closure is not called for: a
    /// kind no line lists, a take that took nothing (an answer: no datagram waiting) or no memory (reported).
    fn stream_fill(&mut self, service: &str, slot: &str) -> Option<String> {
        let key = format!("stream fill {service}");
        if let Some(n) = self.svc_events.get(&key) {
            return Some(n.clone());
        }
        let rc = self.stream_c_type(service)?;
        let face = self.game.services.faces.get(service)?.clone();
        let name = format!("lm_esf_{}", super::module_c(service));
        let mut body = format!("    r->kind = (int32_t)ev->kind;\n    switch (ev->kind) {{\n");
        for k in face.stream.values() {
            body.push_str(&format!("    case {}u: {{\n", k.number));
            let has = |f: &str| k.fields.iter().any(|(n, _)| n == f);
            if has("socket") {
                let from = if k.socket_in_value { "ev->value" } else { "ev->handle" };
                body.push_str(&format!("        r->socket = (uint64_t){from};\n"));
            }
            if has("status") {
                body.push_str("        if (ev->status != LUMA_OK) {\n            r->status.some = 1;\n            r->status.value = ev->status;\n        }\n");
            }
            if has("value") {
                body.push_str("        r->value = ev->value;\n");
            }
            if has("flags") {
                body.push_str("        r->flags = ev->flags;\n");
            }
            let text = k.fields.iter().any(|(n, t)| n == "data" && *t == STy::Text);
            // The bytes into `r->data` (text: made from them); `fail` what no memory does.
            let give = |p: &str, n: &str, fail: &str| -> String {
                if text {
                    format!("if (lumart_text_make(&r->data, (const char*)({p}), {n}, file, line) != 0) {fail}")
                } else {
                    format!("if (lumart_list_own(&r->data, &lumart_u8, {n}, file, line) != 0) {fail} __builtin_memcpy(lumart_bytes_p(r->data), {p}, {n}); r->data->len = {n};")
                }
            };
            match (&k.take, has("data")) {
                (Some(t), _) => {
                    let take = face.functions.iter().find(|f| f.name == *t)?;
                    let buf = c_text(&take.c.params[2].ty);
                    let from = if has("from") {
                        let STy::Struct(pn) = &k.fields.iter().find(|(n, _)| n == "from")?.1 else { return None };
                        let pc = face.struct_c.get(pn)?.clone();
                        let (_, make, _) = self.text_struct(service, pn)?;
                        let sized = if self.service_sized(&pc) { format!(" from.size = (uint32_t)sizeof({pc});") } else { String::new() };
                        Some((pc, make, sized))
                    } else {
                        None
                    };
                    if take.c.params.len() == 5 && from.is_none() {
                        return None;
                    }
                    body.push_str("        lumart_bytes b = 0;\n        int64_t got;\n");
                    if let Some((pc, _, sized)) = &from {
                        body.push_str(&format!("        {pc} from = ({pc}){{0}};{sized}\n"));
                    }
                    let out = if from.is_some() { ", &from" } else { "" };
                    match k.room {
                        None => {
                            body.push_str(&format!("        got = {slot}->{t}(luma_rt_self, ev->handle, 0, 0{out});\n        if (got < 0) return 0;\n        if (got > 0) {{\n            const uint64_t want = (uint64_t)got;\n            if (lumart_list_own(&b, &lumart_u8, want, file, line) != 0) return 0;\n            got = {slot}->{t}(luma_rt_self, ev->handle, ({buf})lumart_bytes_p(b), want{out});\n            if (got < 0) {{ lm_rel_lumart_bytes(&b); return 0; }}\n            const uint64_t n = (uint64_t)got < want ? (uint64_t)got : want;\n            b->len = n;\n"));
                        }
                        Some(room) => {
                            body.push_str(&format!("        if (lumart_list_own(&b, &lumart_u8, {room}u, file, line) != 0) return 0;\n        got = {slot}->{t}(luma_rt_self, ev->handle, ({buf})lumart_bytes_p(b), {room}u{out});\n        if (got < 0) {{ lm_rel_lumart_bytes(&b); return 0; }}\n        {{\n            const uint64_t n = (uint64_t)got < {room}u ? (uint64_t)got : {room}u;\n"));
                        }
                    }
                    // The block made the size taken (a datagram's room is far more than most), or given as it is.
                    if has("data") {
                        let g = give("lumart_bytes_p(b)", "n", "{ lm_rel_lumart_bytes(&b); return 0; }");
                        if text || k.room.is_some() {
                            body.push_str(&format!("            if (n > 0) {{ {g} }}\n"));
                        } else {
                            body.push_str("            r->data = b;\n            b = 0;\n");
                        }
                    }
                    body.push_str("        }\n        lm_rel_lumart_bytes(&b);\n");
                    if let Some((_, make, _)) = &from {
                        body.push_str(&format!("        if ({make}(&r->from, &from, file, line) != 0) return 0;\n"));
                    }
                }
                // A struct of the header (capture.camera's FRAME): the script's copy made of it, inside the call.
                (None, true) if matches!(k.fields.iter().find(|(n, _)| n == "data"), Some((_, STy::Struct(_)))) => {
                    let Some((_, STy::Struct(dn))) = k.fields.iter().find(|(n, _)| n == "data") else { return None };
                    let dc = face.struct_c.get(dn)?.clone();
                    let (_, make, _) = self.text_struct(service, dn)?;
                    let sized = if self.service_sized(&dc) { format!(" && ((const {dc}*)ev->data)->size >= (uint32_t)sizeof({dc})") } else { String::new() };
                    body.push_str(&format!(
                        "        if (ev->data != 0{sized} && {make}(&r->data, (const {dc}*)ev->data, file, line) != 0) return 0;\n"
                    ));
                }
                (None, true) => {
                    let g = give("ev->data", "ev->data_size", "return 0;");
                    body.push_str(&format!("        if (ev->data != 0 && ev->data_size > 0) {{ {g} }}\n"));
                }
                (None, false) => {}
            }
            body.push_str("        return 1;\n    }\n");
        }
        body.push_str("    default:\n        return 0;\n    }\n");
        let head = format!("uint8_t {name}({rc}* r, const LumaEvent* ev, const char* file, uint32_t line)");
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!("{head} {{\n{body}}}\n"));
        self.svc_events.insert(key, name.clone());
        Some(name)
    }

    /// A stream's trampoline `void lm_evs_<m>_<n>(void* user, const LumaEvent* ev)`: the case filled, the closure
    /// called with it (once per event), the case given back. The closure is kept until the mod ends.
    fn stream_trampoline(&mut self, m: usize, service: &str, slot: &str, file: &str, line: u32) -> Option<String> {
        let record = Ty::Service { service: service.to_string(), name: "Event".into() };
        let rc = self.c_type(&record)?;
        let fill = self.stream_fill(service, slot)?;
        let cast = self.fn_cast(&[record], &Ty::Unit)?;
        let n = self.closure_n;
        self.closure_n += 1;
        let name = format!("lm_evs_{m}_{n}");
        let head = format!("void {name}(void* user, const LumaEvent* ev)");
        let rel = if super::counted(&rc) { format!("\n        lm_rel_{rc}(&r);") } else { String::new() };
        // A kind whose take gives one item each (DATAGRAM): as many rounds as the event's value counts, each with its
        // own record; a round whose fill fails (nothing left to take, no memory) ends them.
        let each: Vec<String> = self.game.services.faces.get(service)?.stream.values().filter(|k| k.each).map(|k| format!("ev->kind == {}u", k.number)).collect();
        let rounds = if each.is_empty() { "1".to_string() } else { format!("(({}) && ev->value > 1) ? ev->value : 1", each.join(" || ")) };
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!(
            "{head} {{\n    lumart_fn f = (lumart_fn)user;\n    const uint64_t n = {rounds};\n    for (uint64_t i = 0; i < n; i++) {{\n        {rc} r = ({rc}){{0}};\n        const uint8_t ok = {fill}(&r, ev, {}, {line}u);\n        if (ok) (void)(({cast})(f->code))(f, r);{rel}\n        if (!ok) break;\n    }}\n}}\n",
            c_string(file)
        ));
        Some(name)
    }

    /// A service's struct (or the shared header's) whose fields are all plain values: its C name, which is the
    /// script's value as well (copied as C copies it), and its fields as the script reads them.
    pub(super) fn service_struct(&self, service: &str, name: &str) -> Option<(String, Vec<(String, STy)>)> {
        let (c, fields) = if service == SHARED_SERVICE {
            let c = format!("Luma{name}");
            let fields = self.game.services.faces.values().find_map(|f| f.shared_structs.get(&c))?.clone();
            (c, fields)
        } else {
            let face = self.game.services.faces.get(service)?;
            (face.struct_c.get(name)?.clone(), face.structs.get(name)?.clone())
        };
        fields.iter().all(|(_, t)| self.sty_plain(service, t)).then_some((c, fields))
    }

    fn sty_plain(&self, service: &str, t: &STy) -> bool {
        match t {
            STy::Bool | STy::Number(_) | STy::Vec(_) | STy::Color => true,
            STy::Enum(n) | STy::Handle(n) => self.service_c_type(service, n).is_some(),
            STy::HandleOf(owner, n) => self.service_c_type(owner, n).is_some(),
            STy::Struct(n) => self.service_struct(service, n).is_some(),
            STy::Shared(c) => self.service_struct(SHARED_SERVICE, c.strip_prefix("Luma").unwrap_or(c)).is_some(),
            _ => false,
        }
    }

    /// Whether the C struct `c` starts with `uint32_t size` (set to sizeof wherever the compiler makes one).
    fn service_sized(&self, c: &str) -> bool {
        self.game.services.faces.values().any(|f| f.sized.iter().any(|s| s == c))
    }

    /// `b.field` of a service struct as the script's value.
    pub(super) fn service_field_get(&mut self, fx: &mut Fx, line: u32, b: &str, service: &str, record: &str, field: &str) -> Option<String> {
        // An event record's fields are the script's values as they are (lm_ev_*).
        if self.event_kind(service, record).is_some() {
            return matches!(field, "status" | "value" | "flags" | "data").then(|| format!("{b}.{field}"));
        }
        // A struct holding text or an actor is the script's own values (lm_ss_*); its actor a handle read as the view,
        // S0903 when the object has gone.
        if let Some(fields) = self.text_struct_fields(service, record) {
            let ty = &fields.iter().find(|(n, _)| n == field)?.1;
            if *ty == STy::Actor {
                let cls = self.class_slot("LiveActor");
                let v = fx.temp("void*", &format!("lumart_ref_get({b}.{field}, {cls})"));
                let shown = c_string(&format!("{service}.{record}.{field}"));
                fx.fail_if(&format!("{v} == 0 && lumart_raise({}, {line}, 903, 1, (const char* const[]){{{shown}}})", c_string(&fx.file)));
                return Some(v);
            }
            return Some(format!("{b}.{field}"));
        }
        let (_, fields) = self.service_struct(service, record)?;
        let ty = fields.iter().find(|(n, _)| n == field)?.1.clone();
        Some(match ty {
            STy::Vec(n) => {
                let parts: Vec<String> = (0..n).map(|i| format!("{b}.{field}[{i}]")).collect();
                format!("((lm_vec{n}){{{}}})", parts.join(", "))
            }
            STy::Color => format!("lumart_color_of_rgba8({b}.{field})"),
            STy::Bool => format!("((uint8_t)({b}.{field} != 0))"),
            STy::Enum(_) => format!("((int32_t){b}.{field})"),
            STy::Handle(_) | STy::HandleOf(..) => format!("((uint64_t){b}.{field})"),
            _ => format!("{b}.{field}"),
        })
    }

    /// `<svc>.<T> { field: v, ... }` (every field given, the checker's rule): the header's struct, zeroed, `size` set,
    /// each field written; the values are read in the order written.
    pub(super) fn service_struct_lit(&mut self, fx: &mut Fx, e: &Expr, args: &[(ast::Ident, Result<Expr, String>)], service: &str, name: &str) -> Ex {
        let m = fx.module;
        if self.service_struct(service, name).is_none() && self.text_struct_fields(service, name).is_some() {
            return self.text_struct_lit(fx, e, args, service, name);
        }
        let Some((c, fields)) = self.service_struct(service, name) else { return self.not_yet(fx, e, "a literal of this service struct") };
        let mut sets = Vec::new();
        for (ident, value) in args {
            let fty = fields.iter().find(|(n, _)| *n == ident.name).map(|f| f.1.clone()).ok_or_else(lost)?;
            let to = service_sty(service, &fty);
            let v = match value {
                Ok(x) => {
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    self.coerce(fx, &v, &from, &to, x.at)?
                }
                Err(place) => place.clone(),
            };
            let ct = self.c_type(&to).ok_or_else(lost)?;
            let v = fx.temp(&ct, &v);
            sets.push((ident.name.clone(), fty, v));
        }
        let o = fx.fresh(&c);
        if self.service_sized(&c) {
            fx.emit(format!("{o}.size = (uint32_t)sizeof({c});"));
        }
        for (f, ty, v) in sets {
            let line = match ty {
                STy::Vec(n) => (0..n as usize).map(|i| format!("{o}.{f}[{i}] = {v}.{};", ["x", "y", "z", "w"][i])).collect::<Vec<_>>().join(" "),
                STy::Color => format!("{o}.{f} = lumart_color_rgba8({v});"),
                STy::Bool => format!("{o}.{f} = {v} != 0;"),
                _ => format!("{o}.{f} = {v};"),
            };
            fx.emit(line);
        }
        Ok(o)
    }

    /// A literal of a text struct ([`Self::text_struct`]): the script's copy, each field its value (an actor kept as
    /// its handle); the values are read in the order written.
    fn text_struct_lit(&mut self, fx: &mut Fx, e: &Expr, args: &[(ast::Ident, Result<Expr, String>)], service: &str, name: &str) -> Ex {
        let m = fx.module;
        let (script, _, fields) = self.text_struct(service, name).ok_or_else(lost)?;
        let mut sets = Vec::new();
        for (ident, value) in args {
            let fty = fields.iter().find(|(n, _)| *n == ident.name).map(|f| f.1.clone()).ok_or_else(lost)?;
            let to = service_sty(service, &fty);
            let v = match value {
                Ok(x) => {
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    self.coerce(fx, &v, &from, &to, x.at)?
                }
                Err(place) => place.clone(),
            };
            let ct = self.c_type(&to).ok_or_else(lost)?;
            let v = fx.temp(&ct, &v);
            sets.push((ident.name.clone(), fty, v, ct));
        }
        let o = fx.fresh(&script);
        if super::counted(&script) {
            fx.emit(format!("lm_rel_{script}(&{o});"));
        }
        for (f, ty, v, ct) in sets {
            if ty == STy::Actor {
                let nameobj = self.class_slot("NameObj");
                fx.fail_if(&format!("{v} != 0 && lumart_ref_make(&{o}.{f}, {v}, {nameobj}, {}, {}u) != 0", c_string(&fx.file), e.at.line));
            } else {
                fx.set(&format!("{o}.{f}"), &v, &ct);
            }
        }
        Ok(o)
    }

    /// The trampoline a callback's service calls: `void lm_cb_<m>_<n>(void* user, args...)`, which reads each
    /// argument as the script's value and calls the `fn` value `user` (a script error in it ends that call only).
    fn callback_trampoline(&mut self, m: usize, service: &str, args: &[STy], c_args: &[CT]) -> Option<String> {
        let tys: Vec<Ty> = args.iter().map(|a| service_sty(service, a)).collect();
        let cast = self.fn_cast(&tys, &Ty::Unit)?;
        let n = self.closure_n;
        self.closure_n += 1;
        let name = format!("lm_cb_{m}_{n}");
        let mut head = vec!["void* user".to_string()];
        let mut pass = vec!["f".to_string()];
        for (i, (a, c)) in args.iter().zip(c_args).enumerate() {
            head.push(format!("{} a{i}", c_text(c)));
            pass.push(match a {
                STy::Struct(_) | STy::Shared(_) if c.ptrs == 1 => format!("(a{i} != 0 ? *a{i} : ({}){{0}})", c.base),
                STy::Color => format!("lumart_color_of_rgba8(a{i})"),
                STy::Bool => format!("((uint8_t)(a{i} != 0))"),
                STy::Enum(_) => format!("((int32_t)a{i})"),
                STy::Handle(_) | STy::HandleOf(..) => format!("((uint64_t)a{i})"),
                _ => format!("a{i}"),
            });
        }
        let head = format!("void {name}({})", head.join(", "));
        self.protos.push(format!("{head};"));
        self.closures.push_str(&format!("{head} {{\n    lumart_fn f = (lumart_fn)user;\n    (void)(({cast})(f->code))({});\n}}\n", pass.join(", ")));
        Some(name)
    }

    /// The table slot of a service, made the first time it is called.
    pub(super) fn service_slot(&mut self, service: &str) -> Option<String> {
        if let Some(s) = self.svc_slots.get(service) {
            return Some(s.slot.clone());
        }
        let face = self.game.services.faces.get(service)?;
        if face.header.is_empty() || face.version.is_empty() {
            return None;
        }
        let slot = format!("lm_svc_{}", self.svc_slots.len());
        self.svc_slots.insert(
            service.to_string(),
            SvcSlot { slot: slot.clone(), table: format!("Luma{}Service", camel(service)), header: face.header.clone(), version: face.version.clone() },
        );
        Some(slot)
    }

    /// `<svc>.<f>(args)`, a request included: with its closure it gives its handle `<svc>.<K>?`
    /// (none when refused); waited, a pending of its value.
    pub(super) fn call_service(&mut self, fx: &mut Fx, e: &Expr, service: &str, name: &str, waited: bool, args: &[ast::Arg]) -> Ex {
        let m = fx.module;
        let callee = format!("{service}.{name}");
        let Some(f) = self.game.services.faces.get(service).and_then(|face| face.function(name)).cloned() else { return Err(lost()) };
        let Shape::Callable { params, gives } = &f.shape else { return Err(lost()) };
        let request = match gives {
            Gives::Request { kind, .. } => Some(kind.clone()),
            _ => None,
        };
        let stream = matches!(gives, Gives::Stream { .. });
        let closure = params.iter().find_map(|p| match p {
            SParam::Closure { optional } => Some(*optional),
            _ => None,
        });
        let optional_closure = closure == Some(true);
        if request.is_none() && (waited || (!stream && closure.is_some())) {
            return Err(lost());
        }
        // The C parameters after `mod`, as the face reading took them (svc::Face::make_fn).
        let mut passes = Vec::new();
        let ps = f.c.params.get(1..).ok_or_else(lost)?;
        let mut values = params.iter();
        let members: Vec<crate::svc::FnPtr> = self.game.services.faces.get(service).map(|face| face.functions.iter().map(|s| s.c.clone()).collect()).unwrap_or_default();
        // Each C parameter's first pass, for a pass that reads an argument before it (Items' handle).
        let mut pass_of = Vec::new();
        let mut i = 0;
        while i < ps.len() {
            pass_of.push((i, passes.len()));
            let t = &ps[i].ty;
            let next_is_size = matches!(ps.get(i + 1), Some(d) if d.ty.base == "uint64_t" && d.ty.ptrs == 0);
            let buffer = matches!(t.base.as_str(), "char" | "void" | "uint8_t") && t.ptrs == 1;
            let value = |values: &mut std::slice::Iter<'_, SParam>| match values.next() {
                Some(SParam::Value { ty, .. }) => Ok(ty.clone()),
                _ => Err(lost()),
            };
            if let (0, Some(SParam::Callback { c_fn, args, c_args, .. })) = (t.ptrs, values.clone().next()) {
                if *c_fn == t.base {
                    passes.push(Pass::Callback { args: args.clone(), c_args: c_args.clone(), c_fn: c_fn.clone() });
                    values.next();
                    i += 2;
                    continue;
                }
            }
            // The twin of make_fn's `LumaEventFn fn, void* user`.
            if t.base == "LumaEventFn" && t.ptrs == 0 {
                if !matches!(values.next(), Some(SParam::Closure { .. })) {
                    return Err(lost());
                }
                passes.push(Pass::Event);
                i += 2;
                continue;
            }
            // The twin of make_fn's fixed arrays.
            if ps[i].array.is_some() {
                passes.push(Pass::Arg(value(&mut values)?, ArgForm::Array(t.base.clone())));
                i += 1;
                continue;
            }
            if buffer && t.konst && pixels_in(ps, i) {
                passes.push(Pass::Arg(value(&mut values)?, ArgForm::Pixels));
                i += 1;
                continue;
            }
            if buffer && t.konst && sized_in_32(ps, i) {
                passes.push(Pass::Arg(value(&mut values)?, ArgForm::Sized32));
                i += 2;
                continue;
            }
            if buffer && t.konst {
                if t.base == "char" && !next_is_size {
                    passes.push(Pass::Arg(value(&mut values)?, ArgForm::CText));
                    i += 1;
                } else {
                    passes.push(Pass::Arg(value(&mut values)?, ArgForm::Sized));
                    i += 2;
                }
                continue;
            }
            // The out at this place among the outs (svc::Face::make_fn pushes buffers and outs in parameter order).
            let out_ty = |passes: &[Pass]| match gives {
                Gives::Outs(o) | Gives::Answered(o) => o.get(passes.iter().filter(|p| matches!(p, Pass::Out(..) | Pass::Buffer { .. } | Pass::Lent)).count()).cloned().ok_or_else(lost),
                // A request's one out is its handle (make_fn refuses others).
                Gives::Request { handle, .. } | Gives::Stream { handle: Some(handle) } => Ok(handle.clone()),
                _ => Err(lost()),
            };
            if buffer {
                // The twin of make_fn's buffer: a `uint64_t* length` after the size is the buffer's, no out.
                let length = length_out(ps, i);
                let ty = out_ty(&passes)?;
                let narrow = crate::svc::length_out_32(ps, i);
                let form = if length { Form::LengthOut { narrow, nul: t.base == "char" && f.comment.contains("size - 1") } } else { Form::Result { nul: t.base == "char" } };
                passes.push(Pass::Buffer { ty, form });
                i += if length { 3 } else { 2 };
                continue;
            }
            // The twins of make_fn's counted floats, items counted by another member and lent bytes.
            if floats_in(&f.comment, &ps[i]).is_some() {
                passes.push(Pass::Arg(value(&mut values)?, ArgForm::Array("float".into())));
                i += 1;
                continue;
            }
            if let Some((by, h)) = items_by(name, ps, i, &members) {
                let handle = pass_of.iter().find(|(p, _)| *p == h).map(|x| x.1).ok_or_else(lost)?;
                passes.push(Pass::Arg(value(&mut values)?, ArgForm::Items { by, handle, count: ps[i + 1].name.clone() }));
                i += 2;
                continue;
            }
            if lent_out(ps, i) {
                if !matches!(out_ty(&passes)?, STy::Bytes) {
                    return Err(lost());
                }
                passes.push(Pass::Lent);
                i += 2;
                continue;
            }
            // The twin of make_fn's arrays with a count (C1c).
            if t.ptrs == 1 && array_in(ps, i) {
                passes.push(Pass::Arg(value(&mut values)?, ArgForm::List));
                i += 2;
                continue;
            }
            if t.ptrs == 1 && array_out(ps, i) {
                let ty = out_ty(&passes)?;
                passes.push(Pass::Buffer { ty, form: Form::Count });
                i += 3;
                continue;
            }
            if t.ptrs == 1 {
                if t.konst {
                    passes.push(Pass::Arg(value(&mut values)?, ArgForm::Ref));
                    i += 1;
                    continue;
                }
                let ty = out_ty(&passes)?;
                let base = CT { base: t.base.clone(), konst: false, ptrs: 0 };
                passes.push(Pass::Out(ty, c_text(&base)));
                i += 1;
                continue;
            }
            // A content name (`typedef const char* LumaModelName`, C2) goes as the C text of the value's name.
            let ty = value(&mut values)?;
            let form = if matches!(&ty, STy::ContentName(_)) || matches!(&ty, STy::Opt(x) if matches!(**x, STy::ContentName(_))) { ArgForm::CText } else { ArgForm::Value };
            passes.push(Pass::Arg(ty, form));
            i += 1;
        }
        // Every type a pass names must be written.
        if passes.iter().filter(|p| matches!(p, Pass::Buffer { .. })).count() > 1 {
            return self.not_yet(fx, e, "a service function giving two out buffers");
        }
        for p in &passes {
            let tys = match p {
                Pass::Arg(ty, _) | Pass::Out(ty, _) | Pass::Buffer { ty, .. } => std::slice::from_ref(ty),
                Pass::Callback { args, .. } => args.as_slice(),
                Pass::Event | Pass::Lent => &[],
            };
            // A callback reads its struct arguments as C has them: a text struct's copy is not made there.
            let in_callback = matches!(p, Pass::Callback { .. });
            for ty in tys {
                let text_struct = match ty {
                    STy::Struct(n) => self.service_struct(service, n).is_none(),
                    STy::Opt(x) => matches!(&**x, STy::Struct(n) if self.service_struct(service, n).is_none()),
                    _ => false,
                };
                if !self.service_ty_written(service, ty) || (in_callback && text_struct) {
                    let what = format!("a service function taking or giving {}", sty_text(ty));
                    return self.not_yet(fx, e, &what);
                }
            }
        }
        let Some(slot) = self.service_slot(service) else { return self.not_yet(fx, e, "a service its header gives no version") };
        // The arguments, positional then by name; a service's parameters have no defaults. A waited request is given no
        // closure (the checker took it out of the parameters).
        let names: Vec<&str> = params
            .iter()
            .filter(|p| !(waited && matches!(p, SParam::Closure { .. })))
            .map(|p| match p {
                SParam::Value { name, .. } | SParam::Callback { name, .. } => name.as_str(),
                SParam::Closure { .. } => "",
            })
            .collect();
        let mut given: Vec<Option<&Expr>> = vec![None; names.len()];
        let mut next = 0;
        for a in args {
            let k = match &a.name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(nm) => names.iter().position(|n| *n == nm.name).ok_or_else(lost)?,
            };
            *given.get_mut(k).ok_or_else(lost)? = Some(&a.value);
        }
        let mut list = vec!["luma_rt_self".to_string()];
        let mut held = Vec::new();
        // Each argument's temporary by its pass; the status of a member asked before the call (Items), which the call
        // is made only on.
        let mut arg_at: Vec<(usize, String)> = Vec::new();
        let mut before: Option<String> = None;
        // Lent bytes: the pointer and length the service fills, and the script's copy made of them for LUMA_OK.
        let mut lent: Vec<(String, String, String)> = Vec::new();
        let mut outs = Vec::new();
        // A waited request's box, frame type, step and task, set up just before the call (after every argument that
        // may end the statement).
        let mut wait_box: Option<(String, String, String, String, String)> = None;
        let mut k = 0;
        for (pi, p) in passes.iter().enumerate() {
            match p {
                // A stream's closure takes the service's Event.
                Pass::Event if stream => {
                    let x = given.get(k).copied().flatten().ok_or_else(lost)?;
                    k += 1;
                    let mut to = Ty::Fn(vec![Ty::Service { service: service.to_string(), name: "Event".into() }], Box::new(Ty::Unit));
                    if optional_closure {
                        to = Ty::Opt(Box::new(to));
                    }
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    let v = self.coerce(fx, &v, &from, &to, x.at)?;
                    let Some(tramp) = self.stream_trampoline(m, service, &slot, &fx.file.clone(), x.at.line) else {
                        return self.not_yet(fx, e, "a stream whose events' fields or takes are not written yet");
                    };
                    let (file, line) = (c_string(&fx.file), x.at.line);
                    let keep = |f: &str| format!("lumart_fn_keep({f}, {file}, {line}u) != 0");
                    if optional_closure {
                        // `none`: a null fn (no events, or the watch stopped).
                        let ct = self.c_type(&to).ok_or_else(lost)?;
                        let o = fx.temp(&ct, &v);
                        fx.fail_if(&format!("{o}.some && {}", keep(&format!("{o}.value"))));
                        list.push(format!("({o}.some ? (LumaEventFn){tramp} : (LumaEventFn)0)"));
                        list.push(format!("({o}.some ? (void*){o}.value : (void*)0)"));
                    } else {
                        let f = fx.temp("lumart_fn", &v);
                        fx.fail_if(&keep(&f));
                        list.push(format!("((LumaEventFn){tramp})"));
                        list.push(format!("(void*){f}"));
                    }
                }
                Pass::Event => {
                    let kind = request.clone().ok_or_else(lost)?;
                    if waited {
                        let data = self.event_kind(service, &format!("{}Event", case_name(&kind.to_ascii_uppercase()))).ok_or_else(lost)?.1;
                        let r = data.map_or(Ty::Int(crate::game::Scalar::U64), |d| service_sty(service, &d));
                        let Some((bt, ft, step)) = self.waited_parts(&Ty::Opt(Box::new(r))) else { return self.not_yet(fx, e, "a waited request of this value") };
                        let Some(tramp) = self.waited_trampoline(m, service, &kind, &slot, &bt, &fx.file.clone(), e.at.line) else {
                            return self.not_yet(fx, e, "a request whose event's take its value does not size");
                        };
                        let bx = fx.fresh("lumart_bytes");
                        let task = fx.fresh("lumart_task");
                        list.push(format!("((LumaEventFn){tramp})"));
                        list.push(format!("(void*){bx}"));
                        wait_box = Some((bx, task, bt, ft, step));
                        continue;
                    }
                    let x = given.get(k).copied().flatten().ok_or_else(lost)?;
                    k += 1;
                    let record = Ty::Service { service: service.to_string(), name: format!("{}Event", case_name(&kind.to_ascii_uppercase())) };
                    let to = Ty::Fn(vec![record], Box::new(Ty::Unit));
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    let v = self.coerce(fx, &v, &from, &to, x.at)?;
                    let f = fx.temp("lumart_fn", &v);
                    let Some(tramp) = self.event_trampoline(m, service, &kind, &slot, &fx.file.clone(), x.at.line) else {
                        return self.not_yet(fx, e, "a request whose event's take its value does not size");
                    };
                    let cond = format!("lumart_fn_keep({f}, {}, {}u) != 0", c_string(&fx.file), x.at.line);
                    fx.fail_if(&cond);
                    list.push(format!("((LumaEventFn){tramp})"));
                    list.push(format!("(void*){f}"));
                }
                Pass::Arg(ty, form) => {
                    let x = given.get(k).copied().flatten().ok_or_else(lost)?;
                    let param = names[k];
                    k += 1;
                    let to = service_sty(service, ty);
                    let ct = self.c_type(&to).ok_or_else(lost)?;
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    let v = self.coerce(fx, &v, &from, &to, x.at)?;
                    let t = fx.temp(&ct, &v);
                    arg_at.push((pi, t.clone()));
                    // The script's copy of a text struct is written as the header's struct for the call (lm_sw_).
                    if let Some((c, write, opt)) = self.text_struct_arg(service, ty) {
                        let cs = fx.fresh(&c);
                        let (file, line) = (c_string(&fx.file), x.at.line);
                        let (value, there) = if opt { (format!("{t}.value"), format!("{t}.some && ")) } else { (t.clone(), String::new()) };
                        fx.fail_if(&format!("{there}{write}(&{cs}, &{value}, {file}, {line}u) != 0"));
                        list.push(match (form, opt) {
                            (ArgForm::Ref, true) => format!("({t}.some ? &{cs} : 0)"),
                            (ArgForm::Ref, false) => format!("&{cs}"),
                            (ArgForm::Value, false) => cs,
                            _ => return self.not_yet(fx, e, "a text struct passed this way"),
                        });
                        continue;
                    }
                    match form {
                        ArgForm::Value if *ty == STy::Color => list.push(format!("lumart_color_rgba8({t})")),
                        ArgForm::Value => list.push(t),
                        ArgForm::Ref => match ty {
                            STy::Opt(_) => list.push(format!("({t}.some ? &{t}.value : 0)")),
                            _ => list.push(format!("&{t}")),
                        },
                        ArgForm::CText => {
                            held.push((t.clone(), ct.clone()));
                            let c = fx.fresh("const char*");
                            let (file, line, function, param) = (c_string(&fx.file), x.at.line, c_string(&callee), c_string(param));
                            let read = |text: &str| format!("({c} = lumart_text_cstr({text}, {file}, {line}u, {function}, {param})) == 0");
                            match ty {
                                STy::Opt(_) => {
                                    let cond = format!("{t}.some && {}", read(&format!("{t}.value")));
                                    fx.fail_if(&cond);
                                }
                                _ => {
                                    let cond = read(&t);
                                    fx.fail_if(&cond);
                                }
                            }
                            list.push(c);
                        }
                        ArgForm::List => {
                            // The elements as C has them (a list of a service struct is stored as the header's struct);
                            // a struct with `size` gets it set in each element, in the list made this call's own.
                            held.push((t.clone(), ct.clone()));
                            let STy::List(elem) = ty else { return Err(lost()) };
                            // Items that hold text or bytes (assets.AssetFile): each written as the header's struct
                            // into a block of this call (lm_sw_), its pointers into the script's values.
                            if let Some((ec, write)) = self.list_text_struct(service, elem) {
                                let (file, line) = (c_string(&fx.file), x.at.line);
                                let (a, k) = (fx.fresh(&format!("{ec}*")), fx.fresh("uint64_t"));
                                fx.emit(format!("if (lumart_list_n({t}) > 0) {{"));
                                fx.depth += 1;
                                fx.fail_if(&format!(
                                    "lumart_list_n({t}) > 0xFFFFFFFFu && lumart_overflow({file}, {line}u, \"u32\", {}) != 0",
                                    c_string(param)
                                ));
                                fx.fail_if(&format!("({a} = ({ec}*)lumart_scratch_zeroed(lumart_list_n({t}) * sizeof({ec}), {file}, {line}u)) == 0"));
                                fx.emit(format!("for ({k} = 0; {k} < lumart_list_n({t}); {k}++) {{"));
                                fx.depth += 1;
                                let es = self.c_type(&service_sty(service, elem)).ok_or_else(lost)?;
                                fx.fail_if(&format!("{write}(&{a}[{k}], (const {es}*)lumart_list_at({t}, {k}), {file}, {line}u) != 0"));
                                fx.depth -= 1;
                                fx.emit("}".into());
                                fx.depth -= 1;
                                fx.emit("}".into());
                                list.push(format!("(const {ec}*){a}"));
                                list.push(format!("(uint32_t)lumart_list_n({t})"));
                                continue;
                            }
                            let ety = service_sty(service, elem);
                            let ec = self.c_type(&ety).ok_or_else(lost)?;
                            if self.service_sized(&ec) {
                                let d = self.desc(&ety).ok_or_else(lost)?;
                                let k = fx.fresh("uint64_t");
                                let (file, line) = (c_string(&fx.file), x.at.line);
                                fx.emit(format!("if (lumart_list_n({t}) > 0) {{"));
                                fx.depth += 1;
                                fx.fail_if(&format!("lumart_list_own(&{t}, {d}, 0, {file}, {line}u) != 0"));
                                fx.emit(format!("for ({k} = 0; {k} < lumart_list_n({t}); {k}++) (({ec}*)lumart_list_at({t}, {k}))->size = (uint32_t)sizeof({ec});"));
                                fx.depth -= 1;
                                fx.emit("}".into());
                            }
                            list.push(format!("(lumart_list_n({t}) > 0 ? (const {ec}*)lumart_list_at({t}, 0) : 0)"));
                            list.push(format!("(uint32_t)lumart_list_n({t})"));
                        }
                        // The w and h just passed: the bytes must be w x h x 4 long (a script error before the call).
                        ArgForm::Pixels => {
                            held.push((t.clone(), ct.clone()));
                            let (Some(h), Some(w)) = (list.last().cloned(), list.get(list.len().wrapping_sub(2)).cloned()) else { return Err(lost()) };
                            let (file, line) = (c_string(&fx.file), x.at.line);
                            let fname = c_string(&format!("{service}.{name}"));
                            fx.fail_if(&format!(
                                "(uint64_t)lumart_list_n({t}) != (uint64_t){w} * (uint64_t){h} * 4u && lumart_wrong_call({file}, {line}u, {fname}, {}, \"bytes not w x h x 4 long\", \"w x h x 4 bytes (RGBA, rows top first)\") != 0",
                                c_string(param)
                            ));
                            list.push(format!("({t} != 0 ? (const void*)lumart_bytes_p({t}) : (const void*)0)"));
                        }
                        ArgForm::Sized32 => {
                            held.push((t.clone(), ct.clone()));
                            let (file, line) = (c_string(&fx.file), x.at.line);
                            fx.fail_if(&format!(
                                "lumart_list_n({t}) > 0xFFFFFFFFu && lumart_overflow({file}, {line}u, \"u32\", {}) != 0",
                                c_string(param)
                            ));
                            list.push(format!("({t} != 0 ? (const void*)lumart_bytes_p({t}) : (const void*)0)"));
                            list.push(format!("(uint32_t)lumart_list_n({t})"));
                        }
                        // The member that says how many numbers an item is, asked for the handle's argument; refused
                        // (it reported), the call is not made and gives its status.
                        ArgForm::Items { by, handle, count } => {
                            held.push((t.clone(), ct.clone()));
                            if before.is_some() {
                                return Err(lost());
                            }
                            let h = arg_at.iter().find(|(p, _)| p == handle).map(|x| x.1.clone()).ok_or_else(lost)?;
                            let STy::List(elem) = ty else { return Err(lost()) };
                            let ec = self.c_type(&service_sty(service, elem)).ok_or_else(lost)?;
                            let (file, line) = (c_string(&fx.file), x.at.line);
                            let k = fx.fresh("uint32_t");
                            let st = fx.temp("int32_t", &format!("{slot}->{by}(luma_rt_self, {h}, &{k})"));
                            fx.fail_if(&format!(
                                "{st} == LUMA_OK && {k} != 0 && lumart_list_n({t}) % {k} != 0 && lumart_wrong_multiple({file}, {line}u, {}, {}, lumart_list_n({t}), {}, {k}) != 0",
                                c_string(&callee),
                                c_string(&format!("{param}.len()")),
                                c_string(count)
                            ));
                            list.push(format!("(lumart_list_n({t}) > 0 ? (const {ec}*)lumart_list_at({t}, 0) : (const {ec}*)0)"));
                            list.push(format!("({st} == LUMA_OK && {k} != 0 ? lumart_list_n({t}) / {k} : 0u)"));
                            before = Some(st);
                        }
                        ArgForm::Sized => {
                            held.push((t.clone(), ct.clone()));
                            match ty {
                                STy::Text => {
                                    list.push(format!("lumart_text_p({t})"));
                                    list.push(format!("lumart_text_n({t})"));
                                }
                                // Bytes in (C1d): the block's bytes, null for an empty one, and its length.
                                STy::Bytes => {
                                    list.push(format!("({t} != 0 ? (const void*)lumart_bytes_p({t}) : (const void*)0)"));
                                    list.push(format!("lumart_list_n({t})"));
                                }
                                _ => return self.not_yet(fx, e, "a service function taking bytes"),
                            }
                        }
                        // A compound literal: the array lives to the end of the block the call is in.
                        ArgForm::Array(c) => {
                            let items: Vec<String> = match ty {
                                STy::Vec(n) => ["x", "y", "z", "w"][..usize::from(*n)].iter().map(|f| format!("{t}.{f}")).collect(),
                                STy::Fixed(x, n) => (0..usize::from(*n))
                                    .map(|i| match &**x {
                                        STy::Color => format!("lumart_color_rgba8({t}._{i})"),
                                        _ => format!("{t}._{i}"),
                                    })
                                    .collect(),
                                _ => return Err(lost()),
                            };
                            list.push(format!("((const {c}[{}]){{{}}})", items.len(), items.join(", ")));
                        }
                    }
                }
                Pass::Out(ty, c) => {
                    let o = fx.fresh(c);
                    if self.service_sized(c) {
                        fx.emit(format!("{o}.size = (uint32_t)sizeof({c});"));
                    }
                    list.push(format!("&{o}"));
                    outs.push((o, ty.clone()));
                }
                Pass::Callback { args: cargs, c_args, c_fn } => {
                    let x = given.get(k).copied().flatten().ok_or_else(lost)?;
                    k += 1;
                    let to = Ty::Fn(cargs.iter().map(|a| service_sty(service, a)).collect(), Box::new(Ty::Unit));
                    let v = self.expr(fx, x)?;
                    let from = self.ty(m, x);
                    let v = self.coerce(fx, &v, &from, &to, x.at)?;
                    let f = fx.temp("lumart_fn", &v);
                    let tramp = self.callback_trampoline(m, service, cargs, c_args).ok_or_else(lost)?;
                    let cond = format!("lumart_fn_keep({f}, {}, {}u) != 0", c_string(&fx.file), x.at.line);
                    fx.fail_if(&cond);
                    list.push(format!("(({c_fn}){tramp})"));
                    list.push(format!("(void*){f}"));
                }
                Pass::Buffer { ty, .. } => {
                    list.push(BUFFER_ARGS.to_string());
                    // Its value takes its place among the outs once the call is written (below).
                    outs.push((BUFFER_ARGS.to_string(), ty.clone()));
                }
                Pass::Lent => {
                    let (lp, ln, lb) = (fx.fresh("const void*"), fx.fresh("uint64_t"), fx.fresh("lumart_bytes"));
                    list.push(format!("&{lp}"));
                    list.push(format!("&{ln}"));
                    outs.push((lb.clone(), STy::Bytes));
                    lent.push((lp, ln, lb));
                }
            }
        }
        // The waited request's box (the frame's count, then the trampoline's) and its task, which parks until the event.
        if let Some((bx, task, bt, ft, step)) = &wait_box {
            let (file, line) = (c_string(&fx.file), e.at.line);
            fx.fail_if(&format!("lumart_record_new(&{bx}, (int32_t)sizeof({bt}), {file}, {line}u) != 0"));
            let fr = fx.fresh("void*");
            fx.emit(format!("{fr} = lumart_task_new(sizeof({ft}), {step}, &{task}, {file}, {line}u);"));
            fx.fail_if(&format!("{fr} == 0"));
            fx.emit(format!("lm_cp_lumart_bytes(&(({ft}*){fr})->box, {bx});"));
            fx.emit(format!("lm_ret_lumart_bytes(&{bx});"));
        }
        let args = list.join(", ");
        let r = &f.c.result;
        let status = r.base == "int32_t" && r.ptrs == 0;
        let buffer = passes.iter().find_map(|p| match p {
            Pass::Buffer { ty, form } => Some((ty.clone(), *form)),
            _ => None,
        });
        // A member asked before the call refused: the call is not made and gives that status.
        let guarded = |call: String| match &before {
            Some(g) if r.base == "void" && r.ptrs == 0 => format!("if ({g} == LUMA_OK) {call}"),
            Some(g) => format!("({g} == LUMA_OK ? {call} : ({})({g}))", c_text(r)),
            None => call,
        };
        if before.is_some() && buffer.is_some() {
            return Err(lost());
        }
        let st = if let Some((ty, form)) = buffer {
            let (st, v) = self.buffer_call(fx, e, service, &slot, name, &args, &ty, form, r)?;
            for o in outs.iter_mut().filter(|o| o.0 == BUFFER_ARGS) {
                o.0 = v.clone();
            }
            Some(st)
        } else {
            let call = format!("{slot}->{name}({args})");
            if status {
                Some(fx.temp("int32_t", &guarded(call)))
            } else if matches!(gives, Gives::Number(_)) {
                None
            } else if before.is_some() && r.base == "void" && r.ptrs == 0 {
                fx.emit(format!("{};", guarded(call)));
                None
            } else {
                fx.emit(format!("(void){};", guarded(call)));
                None
            }
        };
        let call = guarded(format!("{slot}->{name}({args})"));
        // Lent bytes copied as the script's own, for LUMA_OK only.
        if !lent.is_empty() {
            let Some(st) = &st else { return Err(lost()) };
            let (file, line) = (c_string(&fx.file), e.at.line);
            for (lp, ln, lb) in &lent {
                fx.fail_if(&format!("{st} == LUMA_OK && lumart_bytes_from(&{lb}, {lp}, {ln}, {file}, {line}u) != 0"));
            }
        }
        // A request refused (no event will come): the box is done with nothing in it, the trampoline's count given back.
        if let (Some(st), Some((bx, _, bt, _, _))) = (&st, &wait_box) {
            fx.emit(format!("if ({st} != LUMA_PENDING && {st} != LUMA_OK) {{ (({bt}*)lumart_bytes_p({bx}))->done = 1; {{ lumart_bytes lm_k = {bx}; lm_rel_lumart_bytes(&lm_k); }} }}"));
        }
        // a mistake gives zeros. A service that wrote part of an out before failing leaves nothing of it. An
        // out buffer's value is made only for LUMA_OK (an empty one else), so it is never zeroed by bytes.
        if let (Some(st), Gives::Outs(_)) = (&st, gives) {
            for (o, ty) in &outs {
                if !matches!(ty, STy::Text | STy::Bytes | STy::List(_)) {
                    fx.emit(format!("if ({st} != LUMA_OK) __builtin_memset(&{o}, 0, sizeof {o});"));
                }
            }
        }
        // The outs as the script's values.
        let mut vals = Vec::new();
        for (o, ty) in &outs {
            let to = service_sty(service, ty);
            let ct = self.c_type(&to).ok_or_else(lost)?;
            let v = match ty {
                STy::Struct(n) if self.service_struct(service, n).is_none() => {
                    let (script, make, _) = self.text_struct(service, n).ok_or_else(lost)?;
                    let s = fx.fresh(&script);
                    let (file, line) = (c_string(&fx.file), e.at.line);
                    fx.fail_if(&format!("{make}(&{s}, &{o}, {file}, {line}u) != 0"));
                    s
                }
                STy::Struct(_) | STy::Shared(_) => o.clone(),
                STy::Color => format!("lumart_color_of_rgba8({o})"),
                _ => format!("({ct}){o}"),
            };
            vals.push((fx.temp(&ct, &v), to));
        }
        let value = |gen: &mut Self, fx: &mut Fx, vals: Vec<(String, Ty)>| -> Ex {
            Ok(match vals.len() {
                0 => String::new(),
                1 => vals[0].0.clone(),
                _ => {
                    let tt = Ty::Tuple(vals.iter().map(|v| v.1.clone()).collect());
                    let ct = gen.c_type(&tt).ok_or_else(lost)?;
                    let t = fx.fresh(&ct);
                    // A counted member (a text out buffer beside a number) takes its count, as every copy does.
                    for (i, (v, vt)) in vals.iter().enumerate() {
                        let ect = gen.c_type(vt).ok_or_else(lost)?;
                        fx.emit(super::assign_line(&format!("{t}._{i}"), v, &ect));
                    }
                    t
                }
            })
        };
        let out = match gives {
            Gives::Number(c) => fx.temp(c, &call),
            Gives::Outs(_) => value(self, fx, vals)?,
            Gives::Answered(o) if o.is_empty() => {
                let st = st.ok_or_else(lost)?;
                format!("((uint8_t)({st} == LUMA_OK))")
            }
            Gives::Answered(o) => {
                let st = st.ok_or_else(lost)?;
                let inner = if o.len() == 1 { vals[0].1.clone() } else { Ty::Tuple(vals.iter().map(|v| v.1.clone()).collect()) };
                let v = value(self, fx, vals)?;
                let opt = Ty::Opt(Box::new(inner));
                let ct = self.c_type(&opt).ok_or_else(lost)?;
                if ct == "void*" {
                    format!("({st} == LUMA_OK ? {v} : (void*)0)")
                } else {
                    // The value takes its count (a text or bytes out buffer is counted): the optional is released
                    // at the statement's end as the value's own temporary is.
                    let inner_ct = match &opt { Ty::Opt(x) => self.c_type(x), _ => None }.ok_or_else(lost)?;
                    let t = fx.fresh(&ct);
                    let set = super::assign_line(&format!("{t}.value"), &v, &inner_ct);
                    fx.emit(format!("if ({st} == LUMA_OK) {{ {t}.some = 1; {set} }}"));
                    t
                }
            }
            // Waited: the task's pending (wait_pending reads it as any task's).
            Gives::Request { .. } if waited => {
                let (_, task, ..) = wait_box.as_ref().ok_or_else(lost)?;
                fx.temp("lumart_pending", &format!("((lumart_pending){{{task}, &lumart_pending_own}})"))
            }
            // A watch (a stream with no handle): as a call with no outs.
            Gives::Stream { handle: None } if f.answers.is_empty() => String::new(),
            Gives::Stream { handle: None } => {
                let st = st.ok_or_else(lost)?;
                format!("((uint8_t)({st} == LUMA_OK))")
            }
            // With its closure (a stream's too): the handle, none when the call was refused.
            Gives::Request { .. } | Gives::Stream { .. } => {
                let st = st.ok_or_else(lost)?;
                let (h, ht) = vals.first().cloned().ok_or_else(lost)?;
                let ct = self.c_type(&Ty::Opt(Box::new(ht))).ok_or_else(lost)?;
                let t = fx.fresh(&ct);
                fx.emit(format!("if ({st} == LUMA_PENDING || {st} == LUMA_OK) {{ {t}.some = 1; {t}.value = {h}; }}"));
                t
            }
        };
        for (t, ct) in held {
            fx.emit(format!("lm_rel_{ct}(&{t});"));
        }
        Ok(out)
    }

    /// An out buffer's call, written twice (the whole length first, with a null buffer of size 0, then the fill)
    /// in place of the one call: `args` holds [`BUFFER_ARGS`] where the buffer's arguments go. Gives the call's status
    /// (`int32_t`: LUMA_OK, or the answer or mistake) and the buffer as the script's value (`lumart_text`,
    /// `lumart_bytes` or a list of texts), which is made only for LUMA_OK and is empty else.
    /// - `int64_t` result (`mods.option`, `save.get`, `net.local.address`): the length, or a negative status. A text
    ///   the service ends with a NUL gets a byte of room more than its length.
    /// - `int32_t` result with `uint64_t* length` (`mods.kit_names`, `kit_read`): the status; LUMA_E_TOO_LARGE on the
    ///   first call (size 0 below the length) says the length is known, as LUMA_OK does.
    /// - an array (`T* out, uint32_t capacity, uint32_t* count`, `int32_t` status): a capacity of 0 first, then a
    ///   list of that many elements, zeroed, `size` set in each when T has one.
    /// A length that grew between the two calls (it cannot on the game thread) is cut to the room given.
    #[allow(clippy::too_many_arguments)]
    fn buffer_call(&mut self, fx: &mut Fx, e: &Expr, service: &str, slot: &str, name: &str, args: &str, ty: &STy, form: Form, result: &CT) -> Result<(String, String), super::NotWritten> {
        let call = |buf: &str| format!("{slot}->{name}({})", args.replace(BUFFER_ARGS, buf));
        let (file, line) = (c_string(&fx.file), e.at.line);
        if form == Form::Count {
            let STy::List(elem) = ty else { return Err(lost()) };
            if !(result.base == "int32_t" && result.ptrs == 0) {
                return Err(lost());
            }
            let ety = service_sty(service, elem);
            let d = self.desc(&ety).ok_or_else(lost)?;
            // Items that hold text (content.model.PlayerPart, input.DeviceInfo): filled as the header's structs in a
            // block of this call, then each made the script's own (lm_sc_) into the list.
            if let Some((hc, _)) = self.list_text_struct(service, elem) {
                let (owner, n) = match &**elem {
                    STy::StructOf(o, n) => (o.clone(), n.clone()),
                    STy::Struct(n) => (service.to_string(), n.clone()),
                    _ => return Err(lost()),
                };
                let (es, make, _) = self.text_struct(&owner, &n).ok_or_else(lost)?;
                let (count, room, k) = (fx.fresh("uint32_t"), fx.fresh("uint32_t"), fx.fresh("uint32_t"));
                let a = fx.fresh(&format!("{hc}*"));
                let l = fx.fresh("lumart_list");
                let st = fx.temp("int32_t", &call(&format!("0, 0, &{count}")));
                fx.emit(format!("if ({st} == LUMA_OK && {count} > 0) {{"));
                fx.depth += 1;
                fx.emit(format!("{room} = {count};"));
                fx.fail_if(&format!("({a} = ({hc}*)lumart_scratch_zeroed((uint64_t){room} * sizeof({hc}), {file}, {line}u)) == 0"));
                if self.service_sized(&hc) {
                    fx.emit(format!("for ({k} = 0; {k} < {room}; {k}++) {a}[{k}].size = (uint32_t)sizeof({hc});"));
                }
                fx.emit(format!("{st} = {};", call(&format!("{a}, {room}, &{count}"))));
                fx.emit(format!("if ({st} == LUMA_OK) {{"));
                fx.depth += 1;
                fx.emit(format!("for ({k} = 0; {k} < {count} && {k} < {room}; {k}++) {{"));
                fx.depth += 1;
                let one = fx.fresh(&es);
                fx.fail_if(&format!("{make}(&{one}, &{a}[{k}], {file}, {line}u) != 0"));
                fx.fail_if(&format!("lumart_list_push(&{l}, {d}, &{one}, {file}, {line}u) != 0"));
                if super::counted(&es) {
                    fx.emit(format!("lm_rel_{es}(&{one});"));
                }
                fx.depth -= 1;
                fx.emit("}".into());
                fx.depth -= 1;
                fx.emit("}".into());
                fx.depth -= 1;
                fx.emit("}".into());
                return Ok((st, l));
            }
            let ec = self.c_type(&ety).ok_or_else(lost)?;
            let (count, room, k) = (fx.fresh("uint32_t"), fx.fresh("uint32_t"), fx.fresh("uint32_t"));
            let l = fx.fresh("lumart_list");
            let st = fx.temp("int32_t", &call(&format!("0, 0, &{count}")));
            fx.emit(format!("if ({st} == LUMA_OK && {count} > 0) {{"));
            fx.depth += 1;
            fx.emit(format!("{room} = {count};"));
            fx.fail_if(&format!("lumart_list_own(&{l}, {d}, {room}, {file}, {line}u) != 0"));
            fx.emit(format!("__builtin_memset(lumart_list_at({l}, 0), 0, (uint64_t){room} * sizeof({ec}));"));
            if self.service_sized(&ec) {
                fx.emit(format!("for ({k} = 0; {k} < {room}; {k}++) (({ec}*)lumart_list_at({l}, {k}))->size = (uint32_t)sizeof({ec});"));
            }
            fx.emit(format!("{st} = {};", call(&format!("({ec}*)lumart_list_at({l}, 0), {room}, &{count}"))));
            fx.emit(format!("if ({st} == LUMA_OK) {l}->len = {count} < {room} ? {count} : {room};"));
            fx.depth -= 1;
            fx.emit("}".into());
            return Ok((st, l));
        }
        let (length, nul, narrow) = match form {
            Form::Result { nul } => (false, nul, false),
            Form::LengthOut { narrow, nul } => (true, nul, narrow),
            Form::Count => return Err(lost()),
        };
        let b = fx.fresh("lumart_bytes");
        let width = if narrow { "uint32_t" } else { "uint64_t" };
        let room = fx.fresh(width);
        let n = fx.fresh(width);
        let st = if length {
            if !(result.base == "int32_t" && result.ptrs == 0) {
                return Err(lost());
            }
            let st = fx.temp("int32_t", &call(&format!("0, 0, &{n}")));
            fx.emit(format!("if ({st} == LUMA_E_TOO_LARGE) {st} = LUMA_OK;"));
            fx.emit(format!("if ({st} == LUMA_OK && {n} > 0) {{"));
            fx.depth += 1;
            // A size that counts the 0 byte: one more, so the whole length fits beside it.
            fx.emit(format!("{room} = {n}{};", if nul { " + 1u" } else { "" }));
            fx.fail_if(&format!("lumart_list_own(&{b}, &lumart_u8, {room}, {file}, {line}u) != 0"));
            fx.emit(format!("{st} = {};", call(&format!("lumart_bytes_p({b}), {room}, &{n}"))));
            fx.emit(format!("if ({n} > {room}) {n} = {room};"));
            fx.depth -= 1;
            fx.emit("}".into());
            st
        } else {
            if !(result.base == "int64_t" && result.ptrs == 0) {
                return Err(lost());
            }
            let len = fx.temp("int64_t", &call("0, 0"));
            fx.emit(format!("if ({len} > 0) {{"));
            fx.depth += 1;
            fx.emit(format!("{n} = (uint64_t){len};"));
            fx.emit(format!("{room} = {n} + {}u;", u8::from(nul)));
            fx.fail_if(&format!("lumart_list_own(&{b}, &lumart_u8, {room}, {file}, {line}u) != 0"));
            fx.emit(format!("{len} = {};", call(&format!("lumart_bytes_p({b}), {room}"))));
            fx.emit(format!("if ({len} >= 0 && (uint64_t){len} < {n}) {n} = (uint64_t){len};"));
            fx.depth -= 1;
            fx.emit("}".into());
            fx.temp("int32_t", &format!("({len} < 0 ? (int32_t){len} : LUMA_OK)"))
        };
        // The bytes the service wrote, as the script's value.
        let p = format!("({b} != 0 ? (const char*)lumart_bytes_p({b}) : \"\")");
        let v = match ty {
            STy::Bytes => {
                fx.emit(format!("if ({st} == LUMA_OK && {b} != 0) {b}->len = {n};"));
                b
            }
            STy::Text => {
                if nul {
                    // A NUL-ended name whose length counted the 0 byte (assets.entry's needed): the text ends at it.
                    fx.emit(format!("if ({st} == LUMA_OK && {b} != 0) {{ {width} z = 0; while (z < {n} && {p}[z] != 0) z++; {n} = z; }}"));
                }
                let t = fx.fresh("lumart_text");
                fx.fail_if(&format!("{st} == LUMA_OK && lumart_text_make(&{t}, {p}, {n}, {file}, {line}u) != 0"));
                t
            }
            STy::List(x) if **x == STy::Text => {
                // Names each followed by a 0 byte (a last one without it is kept too).
                let d = self.desc(&Ty::Text).ok_or_else(lost)?;
                let l = fx.fresh("lumart_list");
                let (at, from, one) = (fx.fresh("uint64_t"), fx.fresh("uint64_t"), fx.fresh("lumart_text"));
                fx.emit(format!("if ({st} == LUMA_OK) {{"));
                fx.depth += 1;
                fx.emit(format!("for ({at} = 0, {from} = 0; {at} <= {n}; {at}++) {{"));
                fx.depth += 1;
                fx.emit(format!("if ({at} < {n} && {p}[{at}] != 0) continue;"));
                fx.emit(format!("if ({at} == {n} && {at} == {from}) break;"));
                fx.fail_if(&format!("lumart_text_make(&{one}, {p} + {from}, {at} - {from}, {file}, {line}u) != 0"));
                fx.fail_if(&format!("lumart_list_push(&{l}, {d}, &{one}, {file}, {line}u) != 0"));
                fx.emit(format!("{from} = {at} + 1;"));
                fx.depth -= 1;
                fx.emit("}".into());
                fx.depth -= 1;
                fx.emit("}".into());
                l
            }
            _ => return Err(lost()),
        };
        Ok((st, v))
    }

    /// A list item that is a struct holding text, bytes or a list (this service's or another's of the same header):
    /// the header's C struct and its lm_sw_ writer. None for a struct of plain fields (stored as the header's).
    fn list_text_struct(&mut self, service: &str, elem: &STy) -> Option<(String, String)> {
        let (owner, n) = match elem {
            STy::Struct(n) => (service.to_string(), n.clone()),
            STy::StructOf(o, n) => (o.clone(), n.clone()),
            _ => return None,
        };
        if self.service_struct(&owner, &n).is_some() {
            return None;
        }
        self.text_struct_write(&owner, &n)
    }

    /// A face type the call writing above handles.
    fn service_ty_written(&self, service: &str, ty: &STy) -> bool {
        match ty {
            // An out buffer's names, or an array of values the call writing passes as C has them (C1c).
            STy::List(x) => match &**x {
                STy::Text => true,
                // Elements as C has them (a struct of plain fields), or a struct holding text written item by item.
                STy::Struct(n) => self.service_struct(service, n).is_some() || self.text_struct_fields(service, n).is_some(),
                STy::StructOf(o, n) => self.service_struct(o, n).is_some() || self.text_struct_fields(o, n).is_some(),
                STy::Bool | STy::Number(_) | STy::Enum(_) | STy::Handle(_) | STy::HandleOf(..) | STy::Shared(_) => self.service_ty_written(service, x),
                _ => false,
            },
            STy::Bool | STy::Number(_) | STy::Text | STy::Bytes | STy::Actor | STy::Color | STy::ContentName(_) | STy::Vec(_) => true,
            STy::Enum(n) | STy::Handle(n) => self.service_c_type(service, n).is_some(),
            STy::HandleOf(owner, n) => self.service_c_type(owner, n).is_some(),
            STy::StructOf(owner, n) => self.service_struct(owner, n).is_some() || self.text_struct_fields(owner, n).is_some(),
            // A tuple of values C has as the script does (a struct of plain fields stored as the header's).
            STy::Fixed(x, _) => match &**x {
                STy::Struct(n) => self.service_struct(service, n).is_some(),
                STy::Bool | STy::Number(_) | STy::Color | STy::Enum(_) | STy::Handle(_) | STy::HandleOf(..) | STy::Shared(_) => self.service_ty_written(service, x),
                _ => false,
            },
            STy::Struct(n) => self.service_struct(service, n).is_some() || self.text_struct_fields(service, n).is_some(),
            STy::Shared(c) => self.service_struct(SHARED_SERVICE, c.strip_prefix("Luma").unwrap_or(c)).is_some(),
            STy::Opt(x) => match &**x {
                STy::Text | STy::Actor | STy::ContentName(_) => true,
                inner @ (STy::Struct(_) | STy::Shared(_)) => self.service_ty_written(service, inner),
                _ => false,
            },
            _ => false,
        }
    }

    /// The services imported LUMA_OPTIONAL: every module's `use` of each is `use optional svc.<name>` (one plain
    /// `use` anywhere makes it needed). The mod loads without them, their slots null.
    pub(super) fn optional_services(&self) -> std::collections::BTreeSet<String> {
        let (mut optional, mut needed) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        for src in self.sources {
            for item in &src.module.items {
                if let ast::ItemKind::Use(ast::Use::Module { optional: o, local: false, path, .. }) = &item.kind {
                    if path.len() >= 2 && path[0].name == "svc" {
                        let service = path[1..].iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(".");
                        if *o { optional.insert(service) } else { needed.insert(service) };
                    }
                }
            }
        }
        optional.retain(|s| !needed.contains(s));
        optional
    }

    /// `#include`s, slot declarations and definitions, and import rows of the services called.
    pub(super) fn service_files(&self) -> (String, String, Vec<(String, String, u32, String)>) {
        let (mut h, mut c, mut rows) = (String::new(), String::new(), Vec::new());
        for header in &self.svc_type_headers {
            if !self.svc_slots.values().any(|s| &s.header == header) {
                h.push_str(&format!("#include \"{header}\"\n"));
            }
        }
        let optional = self.optional_services();
        for (name, s) in &self.svc_slots {
            h.push_str(&format!("#include \"{}\"\n", s.header));
            c.push_str(&format!("const {}* {};\n", s.table, s.slot));
            let row = if optional.contains(name) {
                format!("{{LUMA_IMPORT_SERVICE, LUMA_OPTIONAL, {}, {}, &{}, 0, 0, 0}}", c_string(name), c_string(&s.version), s.slot)
            } else {
                format!("LUMA_SERVICE({}, {}, {})", s.slot, c_string(name), c_string(&s.version))
            };
            rows.push((name.clone(), s.version.clone(), 105, row));
        }
        for s in self.svc_slots.values() {
            h.push_str(&format!("extern const {}* {}; /* the service's table */\n", s.table, s.slot));
        }
        (h, c, rows)
    }
}

/// A service called: its slot, table type, header and version.
pub(super) struct SvcSlot {
    pub slot: String,
    pub table: String,
    pub header: String,
    pub version: String,
}

fn c_text(t: &CT) -> String {
    let mut s = String::new();
    if t.konst {
        s.push_str("const ");
    }
    s.push_str(&t.base);
    for _ in 0..t.ptrs {
        s.push('*');
    }
    s
}

fn sty_text(t: &STy) -> String {
    match t {
        STy::Opt(x) => format!("{}?", sty_text(x)),
        STy::Struct(n) | STy::Enum(n) | STy::Handle(n) => n.clone(),
        STy::HandleOf(owner, n) | STy::StructOf(owner, n) => format!("{owner}.{n}"),
        STy::Fixed(x, n) => format!("({})", vec![sty_text(x); usize::from(*n)].join(", ")),
        STy::Shared(c) => c.clone(),
        STy::Vec(n) => format!("vec{n}"),
        STy::ContentName(k) => format!("a {k} name"),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}
