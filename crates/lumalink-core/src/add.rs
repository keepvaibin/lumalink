//! `lumalink add`: every part of a mod by command, so no modder writes `luma.json` or places a file by
//! hand. A part is an entry of `luma.json` (a need, a conflict, the load order, a permission, an option, a layer, an
//! author, a language's name) or a file at its place (a game file, an asset, a texture, the icon, a picture, a save,
//! a module of source). The mod is checked before and after with the same inputs: a change that brings a new error is
//! taken back whole, and those errors are what the refusal names. `luma.json` is written in the format's key order
//! (docs/modding/format/luma.md, section 4), one key a line, as `lumalink new` writes it. `add character` is `jobs::add_character`.

use crate::jobs::{validate, Inputs};
use crate::say;
use anyhow::{bail, Context, Result};
use luma_validate::{Finding, Level, Report};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// luma.json's keys in the format's order (docs/modding/format/luma.md, section 4; the validator's TOP_KEYS); any other key keeps its place
/// after them, and the check names it.
const ORDER: [&str; 26] = [
    "format", "id", "version", "name", "summary", "text", "authors", "license", "links", "icon", "pictures", "game",
    "nebula", "needs", "conflicts", "load", "takes_over", "options", "permissions", "renderer", "layers", "code",
    "saves", "kit", "build", "pre_standard",
];

/// One part to add.
#[derive(Clone, Debug)]
pub enum Part {
    /// `needs`: another mod by id and a semver range.
    Need { id: String, version: String },
    /// `conflicts`: a mod that must not be on with this one, and why (shown to the player).
    Conflict { id: String, why: String },
    /// `load.after` or `load.before`.
    Load { id: String, before: bool },
    /// `takes_over`: an earlier id of this mod.
    TakesOver { id: String },
    /// `authors`.
    Author { name: String },
    /// `text.<language>`: the mod's name and summary in a language.
    Text { language: String, name: Option<String>, summary: Option<String> },
    /// `permissions`: a permission with why; `hosts` for net.internet, `roots` for the file exchange.
    Permission { name: String, why: String, hosts: Vec<String>, roots: Vec<String> },
    /// `options`: one the player sets on the Mods screen.
    Option(OptionPart),
    /// `layers`: a layer and when it applies; its folder is made.
    Layer(LayerPart),
    /// A whole file at a game path in a layer (`layers/<layer>/files/<at>`).
    File { from: PathBuf, at: String, layer: String },
    /// An asset of the mod's own (`layers/<layer>/assets/<at>`).
    Asset { from: PathBuf, at: String, layer: String },
    /// A game texture replaced by name (`layers/<layer>/textures/<name>`).
    Texture { from: PathBuf, name: Option<String>, layer: String },
    /// `icon`: the PNG copied to media/.
    Icon { from: PathBuf },
    /// `pictures`: a PNG or JPEG copied to media/.
    Picture { from: PathBuf },
    /// `saves`: a save file for a region, with why (shown when the player is asked).
    Save { from: PathBuf, region: String, why: String },
    /// A LumaScript module, `source/<name>.lum`, empty.
    Script { name: String },
}

#[derive(Clone, Debug, Default)]
pub struct OptionPart {
    pub key: String,
    /// `choice`, `number`, `toggle`, `text` or `key`.
    pub kind: String,
    /// As typed: a number for `number`, true or false for `toggle`, else text.
    pub default: String,
    pub name: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// A choice's values, in order, each optionally `value=Name shown`.
    pub values: Vec<String>,
    pub max_length: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct LayerPart {
    pub name: String,
    /// `key=value`: the layer applies when that option has that value (a key twice: any of its values).
    pub options: Vec<String>,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub platforms: Vec<String>,
    /// The mod's code chooses it before content loads.
    pub code: bool,
}

impl Part {
    /// What the part is, for the lines that say what was done: the command's word.
    pub fn word(&self) -> &'static str {
        match self {
            Part::Need { .. } => "need",
            Part::Conflict { .. } => "conflict",
            Part::Load { before: false, .. } => "after",
            Part::Load { before: true, .. } => "before",
            Part::TakesOver { .. } => "takes-over",
            Part::Author { .. } => "author",
            Part::Text { .. } => "text",
            Part::Permission { .. } => "permission",
            Part::Option(_) => "option",
            Part::Layer(_) => "layer",
            Part::File { .. } => "file",
            Part::Asset { .. } => "asset",
            Part::Texture { .. } => "texture",
            Part::Icon { .. } => "icon",
            Part::Picture { .. } => "picture",
            Part::Save { .. } => "save",
            Part::Script { .. } => "script",
        }
    }

    /// The words `add` takes, for its refusal of any other.
    pub const WORDS: [&'static str; 18] = [
        "character", "need", "conflict", "after", "before", "takes-over", "author", "text", "permission", "option",
        "layer", "file", "asset", "texture", "icon", "picture", "save", "script",
    ];
}

/// What came of an `add`.
pub enum Added {
    /// Kept: what was done, one line each, and the mod's check after.
    Done { lines: Vec<String>, report: Report },
    /// Taken back: the errors the change brought, and nothing of it is left.
    Refused { errors: Vec<Finding> },
}

/// A change before it is made: luma.json's new text and the files to write.
struct Change {
    manifest: Map<String, Value>,
    files: Vec<(PathBuf, Vec<u8>)>,
    folders: Vec<PathBuf>,
    lines: Vec<String>,
}

/// Adds `part` to the mod in `folder`, checked with `check` before and after (resolve the inputs first).
pub fn add(folder: &Path, part: &Part, check: &Inputs) -> Result<Added> {
    let json = folder.join("luma.json");
    if !json.is_file() {
        bail!(say!("LumaLink.Add.NoMod", folder = folder.display()));
    }
    let old = std::fs::read(&json).with_context(|| say!("LumaLink.CannotRead", path = json.display()))?;
    let manifest = match serde_json::from_slice::<Value>(&old) {
        Ok(Value::Object(m)) => m,
        Ok(_) => bail!(say!("LumaLink.Add.BadManifest", path = json.display(), why = say!("LumaLink.Add.ManifestShape"))),
        Err(e) => bail!(say!("LumaLink.Add.BadManifest", path = json.display(), why = e)),
    };
    let before = validate(folder, check)?;
    let mut change = Change { manifest, files: Vec::new(), folders: Vec::new(), lines: Vec::new() };
    apply(folder, part, &mut change)?;
    for (path, _) in &change.files {
        if path.exists() {
            bail!(say!("LumaLink.AlreadyThere", path = path.display()));
        }
    }
    // Made in order; on a refusal or a failure every made file and folder goes, and luma.json is put back.
    let mut made_files: Vec<PathBuf> = Vec::new();
    let mut made_folders: Vec<PathBuf> = Vec::new();
    let result = (|| -> Result<Report> {
        let wanted: Vec<PathBuf> = change.folders.iter().cloned().chain(change.files.iter().filter_map(|(p, _)| p.parent().map(Path::to_path_buf))).collect();
        for f in wanted {
            make_folder(&f, &mut made_folders)?;
        }
        for (path, bytes) in &change.files {
            std::fs::write(path, bytes).with_context(|| say!("LumaLink.CannotWrite", path = path.display()))?;
            made_files.push(path.clone());
            change.lines.push(say!("LumaLink.Add.Wrote", path = path.display()));
        }
        let text = manifest_text(&change.manifest);
        if text.as_bytes() != old.as_slice() {
            std::fs::write(&json, text).with_context(|| say!("LumaLink.CannotWrite", path = json.display()))?;
        }
        validate(folder, check)
    })();
    let undo = |made_files: &[PathBuf], made_folders: &[PathBuf]| {
        let _ = std::fs::write(&json, &old);
        for f in made_files.iter().rev() {
            let _ = std::fs::remove_file(f);
        }
        for d in made_folders.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
    };
    let after = match result {
        Ok(r) => r,
        Err(e) => {
            undo(&made_files, &made_folders);
            return Err(e);
        }
    };
    let errors = new_errors(&before, &after);
    if !errors.is_empty() {
        undo(&made_files, &made_folders);
        return Ok(Added::Refused { errors });
    }
    Ok(Added::Done { lines: change.lines, report: after })
}

/// Makes `folder` and the folders above it that are missing, noting each made (outermost first).
fn make_folder(folder: &Path, made: &mut Vec<PathBuf>) -> Result<()> {
    let mut missing = Vec::new();
    let mut at = Some(folder);
    while let Some(f) = at {
        if f.as_os_str().is_empty() || f.exists() {
            break;
        }
        missing.push(f.to_path_buf());
        at = f.parent();
    }
    for f in missing.into_iter().rev() {
        std::fs::create_dir(&f).with_context(|| say!("LumaLink.CannotMake", path = f.display()))?;
        made.push(f);
    }
    Ok(())
}

/// The errors of `after` that `before` did not have. Places are left out of the comparison: luma.json is written
/// again, so its lines move.
fn new_errors(before: &Report, after: &Report) -> Vec<Finding> {
    let key = |f: &Finding| format!("{}\u{1}{:?}\u{1}{}\u{1}{:?}", f.code, f.mod_id, f.file, f.args);
    let mut had: BTreeMap<String, usize> = BTreeMap::new();
    for f in before.findings.iter().filter(|f| f.level == Level::Error) {
        *had.entry(key(f)).or_default() += 1;
    }
    let mut out = Vec::new();
    for f in after.findings.iter().filter(|f| f.level == Level::Error) {
        match had.get_mut(&key(f)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => out.push(f.clone()),
        }
    }
    out
}

/// luma.json's text: the format's key order, one key a line, two-space indents, LF, one line end at the end.
pub fn manifest_text(m: &Map<String, Value>) -> String {
    let mut keys: Vec<&String> = ORDER.iter().filter_map(|k| m.get_key_value(*k).map(|(k, _)| k)).collect();
    keys.extend(m.keys().filter(|k| !ORDER.contains(&k.as_str())));
    let lines: Vec<String> = keys
        .iter()
        .map(|k| {
            let value = serde_json::to_string_pretty(&m[k.as_str()]).expect("JSON").replace('\n', "\n  ");
            format!("  {}: {value}", serde_json::to_string(k).expect("JSON"))
        })
        .collect();
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

/// A path inside the mod as typed (`/` between parts): relative, no empty part, no `.` or `..`, no `\` or `:`.
fn inner_path(text: &str) -> Result<PathBuf> {
    let good = !text.is_empty()
        && !text.starts_with('/')
        && !text.contains(['\\', ':'])
        && text.split('/').all(|p| !p.is_empty() && p != "." && p != "..");
    if !good {
        bail!(say!("LumaLink.Add.BadPath", path = text));
    }
    Ok(text.split('/').collect())
}

/// A name as an id's characters: lowercase letters, digits, `-`, `_`, `.`; starts with a letter.
fn check_name(what: &str, name: &str) -> Result<()> {
    let good = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        && name.len() <= 64;
    if !good {
        bail!(say!("LumaLink.Add.BadName", what = what, name = name));
    }
    Ok(())
}

fn read(from: &Path) -> Result<Vec<u8>> {
    std::fs::read(from).with_context(|| say!("LumaLink.CannotRead", path = from.display()))
}

fn file_name(from: &Path) -> Result<String> {
    match from.file_name().and_then(|n| n.to_str()) {
        Some(n) => Ok(n.to_string()),
        None => bail!(say!("LumaLink.Add.BadPath", path = from.display())),
    }
}

/// The list under `key` (made when missing); refused when the key holds something else.
fn list<'a>(m: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>> {
    let v = m.entry(key.to_string()).or_insert_with(|| json!([]));
    match v {
        Value::Array(a) => Ok(a),
        _ => bail!(say!("LumaLink.Add.NotList", key = key)),
    }
}

/// The object under `key` (made when missing); refused when the key holds something else.
fn object<'a>(m: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Map<String, Value>> {
    let v = m.entry(key.to_string()).or_insert_with(|| json!({}));
    match v {
        Value::Object(o) => Ok(o),
        _ => bail!(say!("LumaLink.Add.NotObject", key = key)),
    }
}

/// Puts `entry` in the list: in place of the one whose `field` is `name` (said as changed), else at the end.
fn put(items: &mut Vec<Value>, field: &str, name: &str, entry: Value, what: &str, lines: &mut Vec<String>) {
    match items.iter().position(|x| x.get(field).and_then(Value::as_str) == Some(name)) {
        Some(i) => {
            items[i] = entry;
            lines.push(say!("LumaLink.Add.Changed", what = what, name = name));
        }
        None => {
            items.push(entry);
            lines.push(say!("LumaLink.Add.Added", what = what, name = name));
        }
    }
}

/// Puts a string in a list of strings once.
fn put_text(items: &mut Vec<Value>, name: &str, what: &str, lines: &mut Vec<String>) {
    if items.iter().any(|x| x.as_str() == Some(name)) {
        lines.push(say!("LumaLink.Add.There", what = what, name = name));
    } else {
        items.push(name.into());
        lines.push(say!("LumaLink.Add.Added", what = what, name = name));
    }
}

/// A number as JSON: whole numbers without a fraction.
fn number(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
    }
}

/// The layers list, with `base` first when it was the default (no `layers` key and a `layers/base` folder).
fn layers<'a>(folder: &Path, m: &'a mut Map<String, Value>) -> Result<&'a mut Vec<Value>> {
    let default_base = !m.contains_key("layers") && folder.join("layers/base").is_dir();
    let items = list(m, "layers")?;
    if default_base {
        items.push(json!({ "name": "base" }));
    }
    Ok(items)
}

/// A layer the mod lists, or `base` with its folder: a file may go into it.
fn known_layer(folder: &Path, m: &Map<String, Value>, layer: &str) -> Result<()> {
    check_name("layer", layer)?;
    let listed = match m.get("layers") {
        Some(Value::Array(items)) => items.iter().any(|x| x.get("name").and_then(Value::as_str) == Some(layer)),
        _ => layer == "base",
    };
    if !listed {
        bail!(say!("LumaLink.Add.NoLayer", layer = layer, folder = folder.display()));
    }
    Ok(())
}

fn apply(folder: &Path, part: &Part, c: &mut Change) -> Result<()> {
    let what = part.word();
    match part {
        Part::Need { id, version } => {
            check_name("need", id)?;
            if luma_validate::semver::Range::parse(version).is_none() {
                bail!(say!("LumaLink.Add.BadRange", value = version));
            }
            put(list(&mut c.manifest, "needs")?, "id", id, json!({ "id": id, "version": version }), what, &mut c.lines);
        }
        Part::Conflict { id, why } => {
            check_name("conflict", id)?;
            put(list(&mut c.manifest, "conflicts")?, "id", id, json!({ "id": id, "why": why }), what, &mut c.lines);
        }
        Part::Load { id, before } => {
            check_name(what, id)?;
            let load = object(&mut c.manifest, "load")?;
            let key = if *before { "before" } else { "after" };
            let items = match load.entry(key.to_string()).or_insert_with(|| json!([])) {
                Value::Array(a) => a,
                _ => bail!(say!("LumaLink.Add.NotList", key = format!("load.{key}"))),
            };
            put_text(items, id, what, &mut c.lines);
        }
        Part::TakesOver { id } => {
            check_name(what, id)?;
            put_text(list(&mut c.manifest, "takes_over")?, id, what, &mut c.lines);
        }
        Part::Author { name } => put_text(list(&mut c.manifest, "authors")?, name, what, &mut c.lines),
        Part::Text { language, name, summary } => {
            if !luma_validate::LANGUAGES.contains(&language.as_str()) {
                bail!(say!("LumaLink.Add.BadLanguage", value = language, list = luma_validate::LANGUAGES.join(", ")));
            }
            if name.is_none() && summary.is_none() {
                bail!(say!("LumaLink.Add.TextEmpty"));
            }
            let text = object(&mut c.manifest, "text")?;
            let had = text.contains_key(language);
            let entry = match text.entry(language.clone()).or_insert_with(|| json!({})) {
                Value::Object(o) => o,
                _ => bail!(say!("LumaLink.Add.NotObject", key = format!("text.{language}"))),
            };
            if let Some(n) = name {
                entry.insert("name".into(), n.as_str().into());
            }
            if let Some(s) = summary {
                entry.insert("summary".into(), s.as_str().into());
            }
            let row = if had { "LumaLink.Add.Changed" } else { "LumaLink.Add.Added" };
            c.lines.push(crate::text::say(row, &[("what", &what), ("name", language)]));
        }
        Part::Permission { name, why, hosts, roots } => {
            let mut entry = json!({ "name": name, "why": why });
            if !hosts.is_empty() {
                entry["hosts"] = json!(hosts);
            }
            if !roots.is_empty() {
                entry["roots"] = json!(roots);
            }
            put(list(&mut c.manifest, "permissions")?, "name", name, entry, what, &mut c.lines);
        }
        Part::Option(o) => {
            check_name(what, &o.key)?;
            let default = match o.kind.as_str() {
                "number" => match o.default.parse::<f64>() {
                    Ok(x) if x.is_finite() => number(x),
                    _ => bail!(say!("LumaLink.Add.BadDefault", value = o.default, kind = o.kind)),
                },
                "toggle" => match o.default.as_str() {
                    "true" => json!(true),
                    "false" => json!(false),
                    _ => bail!(say!("LumaLink.Add.BadDefault", value = o.default, kind = o.kind)),
                },
                "choice" | "text" | "key" => json!(o.default),
                _ => bail!(say!("LumaLink.Add.BadKind", value = o.kind)),
            };
            let mut entry = json!({ "key": o.key, "kind": o.kind, "default": default, "name": o.name });
            match o.kind.as_str() {
                "number" => {
                    for (k, v) in [("min", o.min), ("max", o.max), ("step", o.step)] {
                        if let Some(x) = v {
                            entry[k] = number(x);
                        }
                    }
                }
                "choice" => {
                    let mut values = Vec::new();
                    let mut names = Map::new();
                    for v in &o.values {
                        match v.split_once('=') {
                            Some((value, shown)) => {
                                values.push(json!(value));
                                names.insert(value.to_string(), shown.into());
                            }
                            None => values.push(json!(v)),
                        }
                    }
                    entry["values"] = Value::Array(values);
                    if !names.is_empty() {
                        entry["value_names"] = Value::Object(names);
                    }
                }
                "text" => {
                    if let Some(n) = o.max_length {
                        entry["max_length"] = json!(n);
                    }
                }
                _ => {}
            }
            // Arguments of another kind are not quietly dropped.
            let stray = match o.kind.as_str() {
                "number" => !o.values.is_empty() || o.max_length.is_some(),
                "choice" => o.min.is_some() || o.max.is_some() || o.step.is_some() || o.max_length.is_some(),
                "text" => !o.values.is_empty() || o.min.is_some() || o.max.is_some() || o.step.is_some(),
                _ => !o.values.is_empty() || o.min.is_some() || o.max.is_some() || o.step.is_some() || o.max_length.is_some(),
            };
            if stray {
                bail!(say!("LumaLink.Add.KindArgs", kind = o.kind));
            }
            put(list(&mut c.manifest, "options")?, "key", &o.key, entry, what, &mut c.lines);
        }
        Part::Layer(l) => {
            check_name(what, &l.name)?;
            let mut when = Map::new();
            if !l.options.is_empty() {
                let mut by_key: Vec<(String, Vec<Value>)> = Vec::new();
                for o in &l.options {
                    let Some((key, value)) = o.split_once('=') else { bail!(say!("LumaLink.Add.BadWhen", value = o)) };
                    let value = match value {
                        "true" => json!(true),
                        "false" => json!(false),
                        v => v.parse::<f64>().ok().filter(|x| x.is_finite()).map_or_else(|| json!(v), number),
                    };
                    match by_key.iter_mut().find(|(k, _)| k == key) {
                        Some((_, values)) => values.push(value),
                        None => by_key.push((key.to_string(), vec![value])),
                    }
                }
                let mut option = Map::new();
                for (k, mut values) in by_key {
                    option.insert(k, if values.len() == 1 { values.remove(0) } else { Value::Array(values) });
                }
                when.insert("option".into(), Value::Object(option));
            }
            for (k, items) in [("region", &l.regions), ("language", &l.languages), ("platform", &l.platforms)] {
                if !items.is_empty() {
                    when.insert(k.into(), json!(items));
                }
            }
            if l.code {
                when.insert("code".into(), json!(true));
            }
            let mut entry = json!({ "name": l.name });
            if !when.is_empty() {
                entry["when"] = Value::Object(when);
            }
            put(layers(folder, &mut c.manifest)?, "name", &l.name, entry, what, &mut c.lines);
            c.folders.push(folder.join("layers").join(&l.name));
        }
        Part::File { from, at, layer } | Part::Asset { from, at, layer } => {
            known_layer(folder, &c.manifest, layer)?;
            let kind = if matches!(part, Part::File { .. }) { "files" } else { "assets" };
            let target = folder.join("layers").join(layer).join(kind).join(inner_path(at)?);
            c.files.push((target, read(from)?));
        }
        Part::Texture { from, name, layer } => {
            known_layer(folder, &c.manifest, layer)?;
            let name = match name {
                Some(n) => n.clone(),
                None => file_name(from)?,
            };
            let target = folder.join("layers").join(layer).join("textures").join(inner_path(&name)?);
            c.files.push((target, read(from)?));
        }
        Part::Icon { from } => {
            let name = file_name(from)?;
            inner_path(&name)?;
            if c.manifest.get("icon").and_then(Value::as_str).is_some_and(|old| old != format!("media/{name}")) {
                bail!(say!("LumaLink.Add.IconThere", path = folder.join("luma.json").display()));
            }
            c.files.push((folder.join("media").join(&name), read(from)?));
            c.manifest.insert("icon".into(), format!("media/{name}").into());
            c.lines.push(say!("LumaLink.Add.Added", what = what, name = format!("media/{name}")));
        }
        Part::Picture { from } => {
            let name = file_name(from)?;
            inner_path(&name)?;
            c.files.push((folder.join("media").join(&name), read(from)?));
            put_text(list(&mut c.manifest, "pictures")?, &format!("media/{name}"), what, &mut c.lines);
        }
        Part::Save { from, region, why } => {
            if !luma_validate::REGIONS.contains(&region.as_str()) {
                bail!(say!("LumaLink.Add.BadRegion", value = region, list = luma_validate::REGIONS.join(", ")));
            }
            let name = file_name(from)?;
            inner_path(&name)?;
            let file = format!("saves/{region}/{name}");
            c.files.push((folder.join("saves").join(region).join(&name), read(from)?));
            let entry = json!({ "region": region, "file": file, "why": why });
            put(list(&mut c.manifest, "saves")?, "file", &file, entry, what, &mut c.lines);
        }
        Part::Script { name } => {
            check_name(what, name)?;
            c.files.push((folder.join("source").join(format!("{name}.lum")), Vec::new()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_mod() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("m");
        match crate::jobs::new_mod(&folder, "test.add", "Add", "Adds parts.", &["keepvaibin".to_string()]).unwrap() {
            crate::jobs::NewMod::Made(_) => {}
            crate::jobs::NewMod::Refused(r) => panic!("{:?}", r.findings),
        }
        (dir, folder)
    }

    fn manifest(folder: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(folder.join("luma.json")).unwrap()).unwrap()
    }

    fn done(folder: &Path, part: Part) -> Vec<String> {
        match add(folder, &part, &Inputs::none()).unwrap() {
            Added::Done { lines, report } => {
                assert!(report.passed(), "{:?}", report.findings);
                lines
            }
            Added::Refused { errors } => panic!("{part:?}: {errors:?}"),
        }
    }

    #[test]
    fn each_manifest_part_is_written_in_the_format_order_and_checked() {
        let (_dir, folder) = new_mod();
        done(&folder, Part::Need { id: "lumakit".into(), version: ">=1.0.0".into() });
        done(&folder, Part::Need { id: "lumakit".into(), version: ">=1.2.0".into() });
        done(&folder, Part::Conflict { id: "other-flower".into(), why: "Both change the Fire Flower.".into() });
        done(&folder, Part::Load { id: "lumakit".into(), before: false });
        done(&folder, Part::Author { name: "a friend".into() });
        done(&folder, Part::Text { language: "JpJapanese".into(), name: Some("追加".into()), summary: None });
        let o = OptionPart { key: "balls".into(), kind: "number".into(), default: "3".into(), name: "Balls at once".into(), min: Some(1.0), max: Some(10.0), step: Some(1.0), ..Default::default() };
        done(&folder, Part::Option(o));
        let look = OptionPart { key: "look".into(), kind: "choice".into(), default: "shiny".into(), name: "Look".into(), values: vec!["shiny=Shiny".into(), "matte=Matte".into()], ..Default::default() };
        done(&folder, Part::Option(look));
        let lines = done(&folder, Part::Layer(LayerPart { name: "matte".into(), options: vec!["look=matte".into()], ..Default::default() }));
        assert!(folder.join("layers/matte").is_dir(), "{lines:?}");
        let m = manifest(&folder);
        assert_eq!(m["needs"], json!([{ "id": "lumakit", "version": ">=1.2.0" }]));
        assert_eq!(m["load"], json!({ "after": ["lumakit"] }));
        assert_eq!(m["options"][0], json!({ "key": "balls", "kind": "number", "default": 3, "name": "Balls at once", "min": 1, "max": 10, "step": 1 }));
        assert_eq!(m["options"][1]["value_names"], json!({ "shiny": "Shiny", "matte": "Matte" }));
        // base, the default before, stays first.
        assert_eq!(m["layers"], json!([{ "name": "base" }, { "name": "matte", "when": { "option": { "look": "matte" } } }]));
        let text = std::fs::read_to_string(folder.join("luma.json")).unwrap();
        let at = |k: &str| text.find(&format!("\n  \"{k}\"")).unwrap();
        assert!(at("format") < at("id") && at("authors") < at("needs") && at("needs") < at("conflicts") && at("options") < at("layers"));
        assert_eq!(manifest_text(m.as_object().unwrap()), text);
    }

    #[test]
    fn a_change_that_brings_an_error_is_taken_back_whole() {
        let (_dir, folder) = new_mod();
        let before = std::fs::read(folder.join("luma.json")).unwrap();
        // A default outside its own range: the check's error, and nothing kept.
        let o = OptionPart { key: "balls".into(), kind: "number".into(), default: "30".into(), name: "Balls".into(), min: Some(1.0), max: Some(10.0), ..Default::default() };
        match add(&folder, &Part::Option(o), &Inputs::none()).unwrap() {
            Added::Refused { errors } => assert!(!errors.is_empty()),
            Added::Done { .. } => panic!("kept"),
        }
        assert_eq!(std::fs::read(folder.join("luma.json")).unwrap(), before);
        // A layer the check refuses goes with its folder.
        let bad = LayerPart { name: "pal".into(), regions: vec!["RMGX01".into()], ..Default::default() };
        assert!(matches!(add(&folder, &Part::Layer(bad), &Inputs::none()).unwrap(), Added::Refused { .. }));
        assert!(!folder.join("layers/pal").exists());
        assert_eq!(std::fs::read(folder.join("luma.json")).unwrap(), before);
    }

    #[test]
    fn files_go_to_their_places_and_bad_arguments_are_named() {
        let (dir, folder) = new_mod();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, b"not checked here").unwrap();
        let script = done(&folder, Part::Script { name: "main".into() });
        assert!(folder.join("source/main.lum").is_file(), "{script:?}");
        let asset = dir.path().join("ball.bin");
        std::fs::write(&asset, b"x").unwrap();
        add(&folder, &Part::Asset { from: asset.clone(), at: "models/ball.bin".into(), layer: "base".into() }, &Inputs::none()).unwrap();
        assert!(folder.join("layers/base/assets/models/ball.bin").is_file());
        // Refused before anything is written.
        for (part, row) in [
            (Part::Asset { from: asset.clone(), at: "../out.bin".into(), layer: "base".into() }, "LumaLink.Add.BadPath"),
            (Part::File { from: asset.clone(), at: "a.arc".into(), layer: "nope".into() }, "LumaLink.Add.NoLayer"),
            (Part::Need { id: "lumakit".into(), version: "soon".into() }, "LumaLink.Add.BadRange"),
            (Part::Need { id: "Kit".into(), version: ">=1.0.0".into() }, "LumaLink.Add.BadName"),
            (Part::Save { from: png.clone(), region: "RMGX01".into(), why: "w".into() }, "LumaLink.Add.BadRegion"),
            (Part::Text { language: "Klingon".into(), name: Some("x".into()), summary: None }, "LumaLink.Add.BadLanguage"),
        ] {
            let e = add(&folder, &part, &Inputs::none()).err().unwrap_or_else(|| panic!("{part:?} passed"));
            // The row's longest words outside its placeholders are in the refusal.
            let row_text = crate::text::template(0, row).unwrap_or_else(|| panic!("{row} is no row")).to_string();
            let words = row_text.split(['{', '}']).step_by(2).map(str::trim).max_by_key(|s| s.len()).unwrap_or("").to_string();
            assert!(!words.is_empty() && e.to_string().contains(&words), "{part:?}: {e}");
        }
        assert!(!folder.join("out.bin").exists() && !dir.path().join("out.bin").exists());
        // The same place twice is refused.
        let again = add(&folder, &Part::Script { name: "main".into() }, &Inputs::none()).err().unwrap();
        assert!(again.to_string().contains("main.lum"), "{again}");
    }
}
