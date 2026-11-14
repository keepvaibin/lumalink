//! `lumalink lsp`: the Language Server Protocol over stdin and stdout, for any editor that speaks it
//! (VS Code, Neovim, Helix, Zed...). Each message is `Content-Length: <n>` then a blank line then n bytes of JSON-RPC.
//! What it answers is [`crate::editor`]'s: each module's diagnostics as `lumalink build` gives them (their S-codes and
//! texts in the chosen language), completion, hover, go to definition (into the mod, a needed mod's source or
//! interface, an SDK header, or the game's reference page), references, rename, signature help, a module's outline,
//! a search of the mod's and the game's names, and formatting in the one style.

use crate::editor::{self, Analysis, GameView, Kind, Place, RenameRefused};
use lumascript::ast::Pos;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// The most names a workspace search gives.
const MOST_FOUND: usize = 200;

/// Reads one message; None at the end of the input.
pub fn read_message(input: &mut impl BufRead) -> std::io::Result<Option<Value>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                length = value.trim().parse().ok();
            }
        }
    }
    let mut body = vec![0u8; length.unwrap_or(0)];
    input.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

/// Writes one message.
pub fn write_message(output: &mut impl Write, message: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

/// A `file:` URI's path (percent-decoded; `file:///C:/x` is `C:/x` on Windows).
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&raw[i + 1..i + 3]).ok()?, 16) {
                bytes.push(b);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let text = String::from_utf8(bytes).ok()?;
    // `/C:/x` on Windows is the drive's path.
    let text = if text.len() > 2 && text.as_bytes()[0] == b'/' && text.as_bytes()[2] == b':' { text[1..].to_string() } else { text };
    Some(PathBuf::from(text))
}

/// A path as a `file:` URI.
pub fn path_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from(if text.starts_with('/') { "file://" } else { "file:///" });
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The protocol's place (line from 0, character in UTF-16 units) as the compiler's (both from 1, in characters).
pub fn to_pos(text: &str, line: u64, character: u64) -> Pos {
    let l = text.split('\n').nth(line as usize).unwrap_or("");
    let (mut units, mut chars) = (0u64, 0u32);
    for c in l.chars() {
        if units >= character {
            break;
        }
        units += c.len_utf16() as u64;
        chars += 1;
    }
    Pos { line: line as u32 + 1, col: chars + 1 }
}

/// The compiler's place as the protocol's.
pub fn from_pos(text: &str, at: Pos) -> Value {
    let l = text.split('\n').nth(at.line.saturating_sub(1) as usize).unwrap_or("").trim_end_matches('\r');
    let units: usize = l.chars().take(at.col.saturating_sub(1) as usize).map(char::len_utf16).sum();
    json!({ "line": at.line.saturating_sub(1), "character": units })
}

fn range(text: &str, from: Pos, to: Pos) -> Value {
    json!({ "start": from_pos(text, from), "end": from_pos(text, to) })
}

/// CompletionItemKind and SymbolKind number them differently.
fn kind_number(kind: Kind, completion: bool) -> u32 {
    match (kind, completion) {
        (Kind::Keyword, true) => 14,
        (Kind::Function, true) => 3,
        (Kind::Variable, true) => 6,
        (Kind::Constant, true) => 21,
        (Kind::Struct, true) => 22,
        (Kind::Enum, true) => 13,
        (Kind::Case, true) => 20,
        (Kind::Class, true) => 7,
        (Kind::Field, true) => 5,
        (Kind::Module, true) => 9,
        (Kind::Event, true) => 23,
        (Kind::Keyword, false) => 20,
        (Kind::Function, false) => 12,
        (Kind::Variable, false) => 13,
        (Kind::Constant, false) => 14,
        (Kind::Struct, false) => 23,
        (Kind::Enum, false) => 10,
        (Kind::Case, false) => 22,
        (Kind::Class, false) => 5,
        (Kind::Field, false) => 8,
        (Kind::Module, false) => 2,
        (Kind::Event, false) => 24,
    }
}

/// The server: the files the editor has open, each mod's last reading, and the game.
pub struct Server {
    open: HashMap<PathBuf, String>,
    analyses: HashMap<PathBuf, Analysis>,
    game: GameView,
    /// The index of the chosen language in the game's 14.
    language: usize,
    /// The files each mod last had diagnostics for, so a file that has none now is cleared.
    said: HashMap<PathBuf, Vec<PathBuf>>,
    shutdown: bool,
}

impl Server {
    pub fn new(game: GameView, language: usize) -> Server {
        Server { open: HashMap::new(), analyses: HashMap::new(), game, language, said: HashMap::new(), shutdown: false }
    }

    /// Serves until `exit` or the end of the input. Returns the process's exit status (0 after `shutdown`).
    pub fn serve(&mut self, input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<i32> {
        while let Some(message) = read_message(input)? {
            let method = message.get("method").and_then(Value::as_str).unwrap_or("").to_string();
            let id = message.get("id").cloned();
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            if method == "exit" {
                return Ok(if self.shutdown { 0 } else { 1 });
            }
            // A reply to a request of ours (none is sent): nothing to do.
            if method.is_empty() {
                continue;
            }
            let mut notes = Vec::new();
            let result = self.handle(&method, &params, &mut notes);
            for note in notes {
                write_message(output, &note)?;
            }
            if let Some(id) = id {
                let reply = match result {
                    Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
                    Err((code, text)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } }),
                };
                write_message(output, &reply)?;
            }
        }
        Ok(if self.shutdown { 0 } else { 1 })
    }

    fn say(&self, id: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
        crate::text::say_in(self.language, id, args)
    }

    /// One request or notification; notifications to send go in `notes`.
    pub fn handle(&mut self, method: &str, params: &Value, notes: &mut Vec<Value>) -> Result<Value, (i64, String)> {
        let uri = params.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or("");
        let file = uri_path(uri);
        let at = || {
            let line = params.pointer("/position/line").and_then(Value::as_u64).unwrap_or(0);
            let character = params.pointer("/position/character").and_then(Value::as_u64).unwrap_or(0);
            (line, character)
        };
        // A question about a file of a mod not read yet: read it first.
        if method.starts_with("textDocument/") && !method.starts_with("textDocument/did") {
            if let Some(f) = &file {
                if editor::mod_root(f).is_some_and(|r| !self.analyses.contains_key(&r)) {
                    self.refresh(f, notes);
                }
            }
        }
        match method {
            "initialize" => Ok(json!({
                "capabilities": {
                    "positionEncoding": "utf-16",
                    "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
                    "completionProvider": { "triggerCharacters": [".", ":", "`", "/"] },
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "referencesProvider": true,
                    "renameProvider": { "prepareProvider": true },
                    "signatureHelpProvider": { "triggerCharacters": ["(", ","], "retriggerCharacters": [","] },
                    "documentSymbolProvider": true,
                    "workspaceSymbolProvider": true,
                    "documentFormattingProvider": true,
                },
                "serverInfo": { "name": "lumalink", "version": env!("CARGO_PKG_VERSION") },
            })),
            "initialized" | "$/cancelRequest" | "$/setTrace" | "workspace/didChangeConfiguration" => Ok(Value::Null),
            "shutdown" => {
                self.shutdown = true;
                Ok(Value::Null)
            }
            "textDocument/didOpen" | "textDocument/didChange" => {
                let Some(file) = file else { return Ok(Value::Null) };
                let text = if method == "textDocument/didOpen" {
                    params.pointer("/textDocument/text").and_then(Value::as_str).unwrap_or("").to_string()
                } else {
                    // Full sync: the last change is the whole text.
                    params.pointer("/contentChanges").and_then(Value::as_array).and_then(|c| c.last()).and_then(|c| c.get("text")).and_then(Value::as_str).unwrap_or("").to_string()
                };
                self.open.insert(file.clone(), text);
                self.refresh(&file, notes);
                Ok(Value::Null)
            }
            "textDocument/didSave" => {
                if let Some(file) = file {
                    self.refresh(&file, notes);
                }
                Ok(Value::Null)
            }
            "textDocument/didClose" => {
                if let Some(file) = file {
                    self.open.remove(&file);
                    self.refresh(&file, notes);
                }
                Ok(Value::Null)
            }
            // A file of a mod changed, was made or was removed outside the editor (a module, luma.json, a needed
            // mod's interface): read again every mod read so far that it touches.
            "workspace/didChangeWatchedFiles" => {
                let changed: Vec<PathBuf> = params.get("changes").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("uri").and_then(Value::as_str)).filter_map(uri_path).collect();
                let roots: Vec<PathBuf> = self.analyses.keys().cloned().collect();
                for root in roots {
                    let touched = changed.iter().any(|f| f.starts_with(&root) || self.analyses[&root].libraries.iter().any(|l| f.starts_with(l.lumi.parent().and_then(Path::parent).unwrap_or(&l.lumi))));
                    if touched {
                        let any = root.join("luma.json");
                        self.refresh(&any, notes);
                    }
                }
                Ok(Value::Null)
            }
            "textDocument/hover" => {
                let Some((a, m, pos, _)) = self.place(file.as_deref(), at()) else { return Ok(Value::Null) };
                Ok(match a.hover(m, pos, &self.game) {
                    Some(h) => {
                        let mut parts = vec![format!("```lumascript\n{}\n```", h.code)];
                        if !h.doc.is_empty() {
                            parts.push(h.doc.clone());
                        }
                        if h.others > 0 {
                            parts.push(self.say("LumaLink.Editor.OtherSignatures", &[("count", &h.others)]));
                        }
                        if let Some(page) = &h.page {
                            parts.push(format!("[{}]({})", self.say("LumaLink.Editor.Reference", &[]), path_uri(page)));
                        }
                        json!({ "contents": { "kind": "markdown", "value": parts.join("\n\n") } })
                    }
                    None => Value::Null,
                })
            }
            "textDocument/definition" => {
                let Some((a, m, pos, _)) = self.place(file.as_deref(), at()) else { return Ok(Value::Null) };
                let Some(target) = a.at(m, pos) else { return Ok(Value::Null) };
                Ok(a.definition(&target, &self.game).map_or(Value::Null, |p| self.location(&p)))
            }
            "textDocument/references" => {
                let Some((a, m, pos, _)) = self.place(file.as_deref(), at()) else { return Ok(json!([])) };
                let Some(target) = a.at(m, pos) else { return Ok(json!([])) };
                let with_definition = params.pointer("/context/includeDeclaration").and_then(Value::as_bool).unwrap_or(true);
                let mut places = a.references(&target, &self.game);
                if !with_definition {
                    if let Some(def) = a.definition(&target, &self.game) {
                        places.retain(|p| *p != def);
                    }
                }
                Ok(Value::Array(places.iter().map(|p| self.location(p)).collect()))
            }
            "textDocument/prepareRename" => {
                let Some((a, m, pos, text)) = self.place(file.as_deref(), at()) else { return Ok(Value::Null) };
                // A name the mod made: its range here; else no rename.
                match a.rename(m, pos, "x", &self.game) {
                    Ok(_) => {
                        let o = a.occurrences.iter().find(|o| o.module == m && o.at.line == pos.line && o.at.col <= pos.col && pos.col < o.at.col + o.len.max(1));
                        let start = o.map(|o| o.at).unwrap_or_else(|| word_start(&text, pos));
                        Ok(range(&text, start, Pos { line: start.line, col: start.col + editor::name_len(&text, start) }))
                    }
                    Err(why) => Err((-32602, self.refusal(why))),
                }
            }
            "textDocument/rename" => {
                let new = params.get("newName").and_then(Value::as_str).unwrap_or("");
                let Some((a, m, pos, _)) = self.place(file.as_deref(), at()) else { return Ok(Value::Null) };
                let places = a.rename(m, pos, new, &self.game).map_err(|why| (-32602, self.refusal(why)))?;
                let mut changes: serde_json::Map<String, Value> = serde_json::Map::new();
                for p in &places {
                    let text = self.text_of(&p.file);
                    let edit = json!({ "range": range(&text, p.at, p.end), "newText": new });
                    changes.entry(path_uri(&p.file)).or_insert_with(|| json!([])).as_array_mut().expect("a list").push(edit);
                }
                Ok(json!({ "changes": changes }))
            }
            "textDocument/completion" => {
                let Some((a, m, pos, text)) = self.place(file.as_deref(), at()) else { return Ok(json!({ "isIncomplete": false, "items": [] })) };
                let answer = a.complete(m, pos, &self.game);
                let items: Vec<Value> = answer
                    .items
                    .into_iter()
                    .map(|c| {
                        let mut item = json!({ "label": c.label, "kind": kind_number(c.kind, true), "sortText": format!("{}{}", c.group, c.label) });
                        if !c.detail.is_empty() {
                            item["detail"] = json!(c.detail);
                        }
                        if !c.doc.is_empty() {
                            item["documentation"] = json!({ "kind": "markdown", "value": c.doc });
                        }
                        if let Some((from, to)) = c.replace {
                            item["filterText"] = json!(c.label);
                            item["textEdit"] = json!({ "range": range(&text, from, to), "newText": c.label });
                        }
                        item
                    })
                    .collect();
                Ok(json!({ "isIncomplete": answer.incomplete, "items": items }))
            }
            "textDocument/signatureHelp" => {
                let Some((a, m, pos, _)) = self.place(file.as_deref(), at()) else { return Ok(Value::Null) };
                Ok(match a.signatures(m, pos, &self.game) {
                    Some(s) => json!({
                        "signatures": s.list.iter().map(|sig| {
                            let mut v = json!({ "label": sig.label, "parameters": sig.params.iter().map(|p| json!({ "label": p })).collect::<Vec<_>>() });
                            if !sig.doc.is_empty() {
                                v["documentation"] = json!({ "kind": "markdown", "value": sig.doc });
                            }
                            v
                        }).collect::<Vec<_>>(),
                        "activeSignature": s.active,
                        "activeParameter": s.param,
                    }),
                    None => Value::Null,
                })
            }
            "textDocument/documentSymbol" => {
                let Some((a, m, _, text)) = self.place(file.as_deref(), (0, 0)) else { return Ok(json!([])) };
                Ok(Value::Array(a.outline(m).iter().map(|s| symbol(&text, s)).collect()))
            }
            "workspace/symbol" => {
                let query = params.get("query").and_then(Value::as_str).unwrap_or("");
                let mut out = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                for a in self.analyses.values() {
                    for f in a.search(query, &self.game, MOST_FOUND) {
                        if seen.insert((f.place.file.clone(), f.place.at, f.name.clone())) && out.len() < MOST_FOUND {
                            out.push(json!({ "name": f.name, "kind": kind_number(f.kind, false), "containerName": f.container, "location": self.location(&f.place) }));
                        }
                    }
                }
                // With no mod read, the game's names alone.
                if self.analyses.is_empty() {
                    let empty = Analysis::new(Path::new(""), Vec::new(), &self.game);
                    for f in empty.search(query, &self.game, MOST_FOUND) {
                        out.push(json!({ "name": f.name, "kind": kind_number(f.kind, false), "containerName": f.container, "location": self.location(&f.place) }));
                    }
                }
                Ok(Value::Array(out))
            }
            "textDocument/formatting" => {
                // The one style (lumascript::fmt), whatever the editor's tab size: one edit over the whole text.
                let Some(file) = file else { return Ok(json!([])) };
                let text = self.text_of(&file);
                let styled = lumascript::fmt::format(&text);
                if styled == text {
                    return Ok(json!([]));
                }
                let lines = text.split('\n').count() - 1;
                let last = text.rsplit('\n').next().unwrap_or("").encode_utf16().count();
                Ok(json!([{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": lines, "character": last } }, "newText": styled }]))
            }
            _ if method.starts_with("$/") => Ok(Value::Null),
            _ => Err((-32601, format!("{method}: not offered"))),
        }
    }

    fn refusal(&self, why: RenameRefused) -> String {
        let row = match why {
            RenameRefused::NotAName => "LumaLink.Editor.Rename.NotAName",
            RenameRefused::NoName => "LumaLink.Editor.Rename.NoName",
            RenameRefused::NotOwn => "LumaLink.Editor.Rename.NotOwn",
        };
        self.say(row, &[])
    }

    fn text_of(&self, file: &Path) -> String {
        self.open.get(file).cloned().unwrap_or_else(|| std::fs::read_to_string(file).unwrap_or_default())
    }

    fn location(&self, p: &Place) -> Value {
        let text = self.text_of(&p.file);
        json!({ "uri": path_uri(&p.file), "range": range(&text, p.at, p.end) })
    }

    /// The mod's reading, the module and the compiler's place for a request at (line, character) in `file`; None for a
    /// file that is no module of a mod (no luma.json above it, or not under its `source/`).
    fn place(&self, file: Option<&Path>, (line, character): (u64, u64)) -> Option<(&Analysis, usize, Pos, String)> {
        let file = file?;
        let a = self.analyses.get(&editor::mod_root(file)?)?;
        let m = a.module_of(file)?;
        let text = a.modules[m].text.clone();
        Some((a, m, to_pos(&text, line, character), text))
    }

    /// Reads the mod holding `file` again and says each module's diagnostics, and luma.json's for what is of no
    /// module (and clears those that have none now).
    fn refresh(&mut self, file: &Path, notes: &mut Vec<Value>) {
        let Some(root) = editor::mod_root(file) else { return };
        let modules = editor::modules(&root, &self.open);
        let analysis = Analysis::new(&root, modules, &self.game);
        let mut now = Vec::new();
        let diagnostic = |text: &str, d: &lumascript::Diagnostic| {
            let len = editor::name_len(text, d.at);
            json!({
                "range": range(text, d.at, Pos { line: d.at.line, col: d.at.col + len }),
                "severity": if lumascript::diag::is_note(d.code) { 3 } else { 1 },
                "code": format!("S{:04}", d.code),
                "source": "lumalink",
                "message": crate::script::message_in(self.language, d),
            })
        };
        for (m, module) in analysis.modules.iter().enumerate() {
            let diagnostics: Vec<Value> = analysis.diags[m].iter().map(|d| diagnostic(&module.text, d)).collect();
            notes.push(json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": path_uri(&module.file), "diagnostics": diagnostics } }));
            now.push(module.file.clone());
        }
        let manifest = root.join("luma.json");
        let manifest_text = self.text_of(&manifest);
        let outside: Vec<Value> = analysis
            .outside
            .iter()
            .map(|(what, d)| {
                let mut v = diagnostic(&manifest_text, d);
                v["message"] = json!(format!("{what}: {}", v["message"].as_str().unwrap_or("")));
                v
            })
            .collect();
        if !outside.is_empty() || self.said.get(&root).is_some_and(|s| s.contains(&manifest)) {
            notes.push(json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": path_uri(&manifest), "diagnostics": outside } }));
        }
        if !analysis.outside.is_empty() {
            now.push(manifest);
        }
        for gone in self.said.get(&root).into_iter().flatten().filter(|f| !now.contains(f)) {
            notes.push(json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": path_uri(gone), "diagnostics": [] } }));
        }
        self.said.insert(root.clone(), now);
        self.analyses.insert(root, analysis);
    }
}

/// The start of the word at `at`.
fn word_start(text: &str, at: Pos) -> Pos {
    let line: Vec<char> = text.split('\n').nth(at.line as usize - 1).unwrap_or("").chars().collect();
    let mut col = (at.col as usize - 1).min(line.len());
    while col > 0 && (line[col - 1].is_alphanumeric() || line[col - 1] == '_') {
        col -= 1;
    }
    Pos { line: at.line, col: col as u32 + 1 }
}

fn symbol(text: &str, s: &editor::OutlineItem) -> Value {
    let selection = range(text, s.name_at, Pos { line: s.name_at.line, col: s.name_at.col + s.name_len });
    let whole = range(text, s.start.min(s.name_at), s.end.max(Pos { line: s.name_at.line, col: s.name_at.col + s.name_len }));
    let mut v = json!({ "name": s.name, "kind": kind_number(s.kind, false), "range": whole, "selectionRange": selection });
    if !s.detail.is_empty() {
        v["detail"] = json!(s.detail);
    }
    if !s.children.is_empty() {
        v["children"] = Value::Array(s.children.iter().map(|c| symbol(text, c)).collect());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumascript::game::Game;
    use std::collections::HashMap as Map;

    fn framed(messages: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for m in messages {
            write_message(&mut out, m).unwrap();
        }
        out
    }

    /// The messages the server writes for these, in order.
    fn session(game: GameView, messages: &[Value]) -> Vec<Value> {
        let mut output = Vec::new();
        Server::new(game, 0).serve(&mut std::io::Cursor::new(framed(messages)), &mut output).unwrap();
        let mut reader = std::io::Cursor::new(output);
        let mut said = Vec::new();
        while let Some(m) = read_message(&mut reader).unwrap() {
            said.push(m);
        }
        said
    }

    fn answer(said: &[Value], id: i64) -> Value {
        said.iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("no answer {id} in {said:?}"))["result"].clone()
    }

    fn request(id: i64, method: &str, uri: &str, (line, character): (u64, u64)) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } } })
    }

    /// The protocol's place of the `nth` (from 0) `needle` in `text`, `after` characters into it (ASCII text).
    fn spot(text: &str, needle: &str, nth: usize, after: usize) -> (u64, u64) {
        let at = text.match_indices(needle).nth(nth).unwrap_or_else(|| panic!("{needle} #{nth}")).0 + after;
        let line = text[..at].matches('\n').count();
        let character = at - text[..at].rfind('\n').map_or(0, |n| n + 1);
        (line as u64, character as u64)
    }

    const TYPES: &str = r#"{
  "types": [
    {"kind": "class", "name": "NameObj", "size": 8, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [], "reachable": [], "fields": []},
    {"kind": "class", "name": "Actor", "size": 64, "align": 8, "polymorphic": true, "abstract": false, "trivial_copy": false, "trivial_destroy": false, "no_copy": false, "bases": [{"name": "NameObj", "offset": 0, "virtual": false}], "reachable": [], "fields": [
      {"name": "mCount", "type": "s32", "offset": 8, "count": 1, "extents": null, "access": "public", "bits": null}]},
    {"kind": "alias", "name": "s32", "of": "int", "record": null},
    {"kind": "alias", "name": "f32", "of": "float", "record": null}
  ],
  "globals": [],
  "left_out": []
}"#;

    /// A small game: an actor record, a function giving one, and `MR::say` with two signatures.
    fn game() -> GameView {
        let mut functions: Map<String, Vec<(String, bool)>> = Map::new();
        for (name, sig) in [("MR::actor", "Actor *()"), ("MR::say", "void(const char *)"), ("MR::say", "void(const char *, s32)")] {
            functions.entry(name.to_string()).or_default().push((sig.to_string(), false));
        }
        let types = luma_validate::names::Types::from_json(TYPES.as_bytes()).unwrap();
        GameView { game: Game::new(functions, Some(types)), ..GameView::default() }
    }

    const KIT: &str = "/// Pushes by `speed`.\npub fn push(speed: f32) -> f32 {\n    return speed\n}\n";

    const MAIN: &str = "use toolbox.actor\n\n/// A ball.\nstruct Ball {\n    size: f32\n    fn grow(self) -> f32 {\n        return self.size\n    }\n}\n\nfn roll(b: Ball) {\n    let n = actor.push(b.size)\n    MR::say(\"hi\", 2)\n    // b.\n    log \"{n}\"\n}\n";

    /// A mod `test.mod` needing the library `toolbox` beside it (its interface and its source), in a fresh folder.
    fn two_mods() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let toolbox = dir.path().join("toolbox");
        std::fs::create_dir_all(toolbox.join("code")).unwrap();
        std::fs::create_dir_all(toolbox.join("source")).unwrap();
        std::fs::write(toolbox.join("luma.json"), r#"{"format": 1, "id": "toolbox", "version": "1.0.0"}"#).unwrap();
        std::fs::write(toolbox.join("source").join("actor.lum"), KIT).unwrap();
        let module = lumascript::on_compiler_stack(|| lumascript::parse_module(KIT).0).unwrap();
        let lumi = lumascript::lumi::write("toolbox", "1.0.0", &[("actor", KIT, &module)], &|_, _| Some("lumi1:0011223344556677".to_string()));
        std::fs::write(toolbox.join("code").join("toolbox.lumi"), lumi).unwrap();
        let main = dir.path().join("test.mod");
        std::fs::create_dir_all(main.join("source")).unwrap();
        std::fs::write(main.join("luma.json"), r#"{"format": 1, "id": "test.mod", "version": "1.0.0", "needs": [{"id": "toolbox", "version": "1.0.0"}]}"#).unwrap();
        let file = main.join("source").join("main.lum");
        std::fs::write(&file, MAIN).unwrap();
        (dir, file, toolbox.join("source").join("actor.lum"))
    }

    #[test]
    fn places_count_utf16_units_on_the_wire_and_characters_in_the_compiler() {
        let text = "let a = \"😀\" + b\n";
        // `b` is the 15th character; 😀 is two UTF-16 units.
        assert_eq!(to_pos(text, 0, 15), Pos { line: 1, col: 15 });
        assert_eq!(from_pos(text, Pos { line: 1, col: 15 }), json!({ "line": 0, "character": 15 }));
        assert_eq!(from_pos(text, Pos { line: 1, col: 10 }), json!({ "line": 0, "character": 9 }));
    }

    #[test]
    fn uris_and_paths_go_both_ways() {
        assert_eq!(uri_path("file:///C:/mods/a%20b/main.lum"), Some(PathBuf::from("C:/mods/a b/main.lum")));
        assert_eq!(uri_path("file:///home/x/main.lum"), Some(PathBuf::from("/home/x/main.lum")));
        assert_eq!(path_uri(Path::new("/home/x/a b.lum")), "file:///home/x/a%20b.lum");
    }

    #[test]
    fn a_session_opens_a_module_says_its_errors_and_ends() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("luma.json"), r#"{"id":"test.mod","version":"1.0.0"}"#).unwrap();
        std::fs::create_dir_all(dir.path().join("source")).unwrap();
        let file = dir.path().join("source").join("main.lum");
        std::fs::write(&file, "").unwrap();
        let uri = path_uri(&file);
        let said = session(GameView::default(), &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "lumascript", "version": 1, "text": "fn f() {\n    let = 1\n}\n" } } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        assert_eq!(said[0]["id"], 1);
        let caps = &said[0]["result"]["capabilities"];
        for c in ["hoverProvider", "definitionProvider", "referencesProvider", "documentSymbolProvider", "workspaceSymbolProvider", "documentFormattingProvider"] {
            assert_eq!(caps[c], true, "{c}");
        }
        // Nothing is offered that is not answered.
        for c in ["codeActionProvider", "semanticTokensProvider", "documentRangeFormattingProvider"] {
            assert!(caps[c].is_null(), "{c}");
        }
        let diags = said.iter().find(|m| m["method"] == "textDocument/publishDiagnostics").expect("diagnostics");
        let list = diags["params"]["diagnostics"].as_array().unwrap();
        assert!(!list.is_empty());
        assert!(list[0]["code"].as_str().unwrap().starts_with('S'));
        assert_eq!(list[0]["range"]["start"]["line"], 1);
        assert!(said.iter().any(|m| m["id"] == 2 && m["result"].is_null()));
    }

    #[test]
    fn formatting_gives_the_one_style_as_one_edit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.lum");
        let uri = path_uri(&file);
        let said = session(GameView::default(), &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "lumascript", "version": 1, "text": "fn f() {\r\nlog \"\u{1F600}\"   \r\n}" } } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/formatting", "params": { "textDocument": { "uri": uri }, "options": { "tabSize": 2, "insertSpaces": false } } }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": { "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "fn f() {\n    x\n}\n" }] } }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/formatting", "params": { "textDocument": { "uri": uri }, "options": { "tabSize": 4, "insertSpaces": true } } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown" }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        assert_eq!(said[0]["result"]["capabilities"]["documentFormattingProvider"], true);
        // Whatever the editor's tab size: four spaces, LF, no spaces at a line's end, one line end at the end.
        let edits = answer(&said, 2);
        let edits = edits.as_array().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["newText"], "fn f() {\n    log \"\u{1F600}\"\n}\n");
        // The whole old text: to line 2, after its one character.
        assert_eq!(edits[0]["range"]["start"], json!({ "line": 0, "character": 0 }));
        assert_eq!(edits[0]["range"]["end"], json!({ "line": 2, "character": 1 }));
        // In the style already: no edit.
        assert_eq!(answer(&said, 3), json!([]));
    }

    #[test]
    fn a_file_of_no_mod_gets_no_answer_and_no_error() {
        let dir = tempfile::tempdir().unwrap();
        let uri = path_uri(&dir.path().join("loose.lum"));
        let said = session(GameView::default(), &[
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "lumascript", "version": 1, "text": "fn f() {}\n" } } }),
            request(1, "textDocument/hover", &uri, (0, 3)),
            request(2, "textDocument/definition", &uri, (0, 3)),
            request(3, "textDocument/completion", &uri, (0, 3)),
            request(4, "textDocument/documentSymbol", &uri, (0, 0)),
            request(5, "textDocument/signatureHelp", &uri, (0, 3)),
        ]);
        assert!(said.iter().all(|m| m.get("error").is_none()), "{said:?}");
        assert!(answer(&said, 1).is_null());
        assert!(answer(&said, 2).is_null());
        assert_eq!(answer(&said, 3)["items"], json!([]));
        assert_eq!(answer(&said, 4), json!([]));
        assert!(answer(&said, 5).is_null());
    }

    /// One session over a mod that needs a library, with the game's names: what each question gets.
    #[test]
    fn the_server_answers_from_the_checker_with_the_library_and_the_game() {
        let (_dir, file, kit_source) = two_mods();
        let uri = path_uri(&file);
        let rename = |id: i64, at: (u64, u64), name: &str| json!({ "jsonrpc": "2.0", "id": id, "method": "textDocument/rename", "params": { "textDocument": { "uri": uri }, "position": { "line": at.0, "character": at.1 }, "newName": name } });
        let said = session(game(), &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "lumascript", "version": 1, "text": MAIN } } }),
            request(2, "textDocument/hover", &uri, spot(MAIN, "push", 0, 1)),
            request(3, "textDocument/definition", &uri, spot(MAIN, "push", 0, 1)),
            request(4, "textDocument/hover", &uri, spot(MAIN, "Ball", 1, 1)),
            request(5, "textDocument/completion", &uri, spot(MAIN, "b.size", 0, 2)),
            request(6, "textDocument/completion", &uri, spot(MAIN, "MR::say", 0, 4)),
            request(7, "textDocument/completion", &uri, spot(MAIN, "// b.", 0, 5)),
            request(8, "textDocument/signatureHelp", &uri, spot(MAIN, ", 2)", 0, 2)),
            request(9, "textDocument/documentSymbol", &uri, (0, 0)),
            json!({ "jsonrpc": "2.0", "id": 10, "method": "workspace/symbol", "params": { "query": "gro" } }),
            rename(11, spot(MAIN, "size", 1, 1), "width"),
            rename(12, spot(MAIN, "push", 0, 1), "shove"),
            request(13, "textDocument/hover", &uri, spot(MAIN, "say", 0, 1)),
            json!({ "jsonrpc": "2.0", "id": 14, "method": "textDocument/references", "params": { "textDocument": { "uri": uri }, "position": { "line": spot(MAIN, "size", 0, 1).0, "character": spot(MAIN, "size", 0, 1).1 }, "context": { "includeDeclaration": false } } }),
        ]);
        // The diagnostics are the build's: the mod reads with its library and the game, so none.
        let diags = said.iter().find(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri).expect("diagnostics");
        assert_eq!(diags["params"]["diagnostics"], json!([]), "{diags}");

        // A library function: its signature and its comment, from the interface.
        let hover = answer(&said, 2)["contents"]["value"].as_str().unwrap_or_default().to_string();
        assert!(hover.contains("fn push(speed: f32) -> f32"), "{hover}");
        assert!(hover.contains("Pushes by `speed`."), "{hover}");
        // ...made in the library's own source beside the mod.
        let def = answer(&said, 3);
        assert_eq!(uri_path(def["uri"].as_str().unwrap()).unwrap(), kit_source);
        assert_eq!(def["range"]["start"]["line"], 1);

        // A type written in a parameter: the struct, with its comment.
        let hover = answer(&said, 4)["contents"]["value"].as_str().unwrap_or_default().to_string();
        assert!(hover.contains("struct Ball") && hover.contains("A ball."), "{hover}");

        // After `b.`: the struct's fields and functions, nothing else.
        let labels = |id: i64| -> Vec<String> { answer(&said, id)["items"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect() };
        let members = labels(5);
        assert!(members.contains(&"size".to_string()) && members.contains(&"grow".to_string()), "{members:?}");
        assert!(!members.contains(&"let".to_string()), "{members:?}");
        // After `MR::`: the game's functions under it.
        let game_names = labels(6);
        assert!(game_names.contains(&"say".to_string()) && game_names.contains(&"actor".to_string()), "{game_names:?}");
        // In a comment: nothing.
        assert!(labels(7).is_empty());

        // Both signatures of `MR::say`; the one the checker chose is the active one, at its second parameter.
        let sig = answer(&said, 8);
        let list = sig["signatures"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{sig}");
        let active = &list[sig["activeSignature"].as_u64().unwrap() as usize];
        assert_eq!(active["parameters"].as_array().unwrap().len(), 2, "{sig}");
        assert_eq!(sig["activeParameter"], 1);

        // The outline: Ball with its members, then roll.
        let outline = answer(&said, 9);
        let names: Vec<&str> = outline.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Ball") && names.contains(&"roll"), "{outline}");
        let ball = outline.as_array().unwrap().iter().find(|s| s["name"] == "Ball").unwrap();
        let children: Vec<&str> = ball["children"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(children, vec!["size", "grow"]);

        // A search of the mod's names.
        let found = answer(&said, 10);
        assert!(found.as_array().unwrap().iter().any(|s| s["name"] == "grow"), "{found}");

        // The mod's field: every place it is written.
        let edits = &answer(&said, 11)["changes"][uri.as_str()];
        assert_eq!(edits.as_array().map_or(0, Vec::len), 3, "{edits}");
        // A library's function is not the mod's to rename: refused, in words.
        let refused = said.iter().find(|m| m["id"] == 12).unwrap();
        assert_eq!(refused["error"]["message"], crate::text::say_in(0, "LumaLink.Editor.Rename.NotOwn", &[]));

        // A game function with two signatures: the chosen one, and how many others.
        let hover = answer(&said, 13)["contents"]["value"].as_str().unwrap_or_default().to_string();
        assert!(hover.contains("s32") || hover.contains("i32"), "{hover}");
        assert!(hover.contains(&crate::text::say_in(0, "LumaLink.Editor.OtherSignatures", &[("count", &1)])), "{hover}");

        // References without the declaration: the two uses.
        assert_eq!(answer(&said, 14).as_array().unwrap().len(), 2);
    }
}
