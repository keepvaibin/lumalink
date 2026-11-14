//! A published mod (a Riivolution pack, a folder laid out as the disc, loose files, a texture pack, a Gecko
//! list...) written as a mod folder in the one form: `luma.json`, each layer's
//! `files/<game path>`, a texture pack's textures in `layers/base/textures/` (decision 3), and what the previous engine
//! read from its own rows under `pre_standard` (OLD-ROWS: the game reads them there until their replacements pass).
//! The plan (plan.rs) decides where every file goes; this writes it. Each file is first cooked as the game cooks it, so
//! one the game cannot use is named with its reason and left out only when the maker accepts it.

use crate::game_store::GameStore;
use crate::layout::Layout;
use crate::plan::{Layer, Level, Note, Plan, PlannedFile, When};
use crate::say;
use anyhow::{bail, Context, Result};
use nebula_import::cook::overlay::{self, Change};
use nebula_import::cook::overlay_patches::PlanPatches;
use nebula_import::cook::{refuse, Stats};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// What the maker says that the mod itself does not.
#[derive(Clone, Debug, Default)]
pub struct Given {
    /// The mod's name, in place of the pack's title or the file's name.
    pub name: Option<String>,
    /// Who made it, in place of the pack's authors.
    pub authors: Vec<String>,
    /// Write the mod without the files the game cannot use.
    pub accept_losses: bool,
}

/// What came of a conversion.
#[derive(Clone, Debug, Default)]
pub struct Converted {
    pub id: String,
    /// Files written under the layers' `files/`.
    pub files: usize,
    /// Textures written under `layers/base/textures/`.
    pub textures: usize,
    /// Files the game cannot use, each with why: left out.
    pub left_out: Vec<(String, String)>,
    /// What the plan and the writing found to say.
    pub notes: Vec<Note>,
}

/// A conversion that would leave files out, refused until the maker accepts it (`Given::accept_losses`).
#[derive(Debug)]
pub struct Losses(pub Vec<(String, String)>);

impl std::fmt::Display for Losses {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", say!("LumaLink.Convert.Losses", count = self.0.len()))
    }
}

impl std::error::Error for Losses {}

/// The form's key (manifest key_ok: 1 to 64 of a-z, 0-9, '-', '.', '_', starting with a letter) for a name of the
/// previous engine's (an option's or a choice's id, a layer's name).
pub fn key_of(raw: &str) -> String {
    let mut key: String = raw
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '_') { c } else { '-' })
        .collect();
    if !key.starts_with(|c: char| c.is_ascii_lowercase()) {
        key.insert_str(0, "o-");
    }
    key.truncate(64);
    key
}

/// The form's mod id (3 to 64 of a-z, 0-9, '-', '.', starting with a letter) from the plan's (`plan::mod_id`).
pub fn id_of(name: &str) -> String {
    let mut id = crate::plan::mod_id(name).replace('_', "-");
    if !id.starts_with(|c: char| c.is_ascii_lowercase()) {
        id.insert_str(0, "mod-");
    }
    if id.len() < 3 {
        id.push_str("-mod");
    }
    id.truncate(64);
    id.trim_end_matches('-').to_string()
}

/// A key not yet in `taken`: `key`, else `key-2`, `key-3`...
fn unused(key: String, taken: &BTreeSet<String>) -> String {
    if !taken.contains(&key) {
        return key;
    }
    (2..).map(|n| format!("{}-{n}", &key[..key.len().min(60)])).find(|k| !taken.contains(k)).unwrap_or(key)
}

/// The releases a version letter of the previous engine stands for (the disc id's fourth letter). Korean discs are
/// none of the form's regions (luma_validate::REGIONS).
fn region_of(letter: char) -> Option<&'static str> {
    match letter.to_ascii_uppercase() {
        'E' => Some("RMGE01"),
        'P' => Some("RMGP01"),
        'J' => Some("RMGJ01"),
        _ => None,
    }
}

/// The plan's options and choices under the form's keys, and the map from the plan's ids to them.
struct Keys {
    options: BTreeMap<String, (String, BTreeMap<String, String>)>,
}

impl Keys {
    fn of(plan: &Plan) -> Keys {
        let mut options = BTreeMap::new();
        let mut taken = BTreeSet::new();
        for option in &plan.options {
            let key = unused(key_of(&option.id), &taken);
            taken.insert(key.clone());
            let mut values = BTreeMap::new();
            let mut used = BTreeSet::new();
            for choice in &option.choices {
                let value = unused(key_of(&choice.id), &used);
                used.insert(value.clone());
                values.insert(choice.id.clone(), value);
            }
            options.insert(option.id.clone(), (key, values));
        }
        Keys { options }
    }

    fn options(&self, plan: &Plan) -> Vec<Value> {
        plan.options
            .iter()
            .map(|option| {
                let (key, values) = &self.options[&option.id];
                let list: Vec<&String> = option.choices.iter().map(|c| &values[&c.id]).collect();
                let names: Map<String, Value> = option.choices.iter().map(|c| (values[&c.id].clone(), Value::from(line(&c.name, &c.id)))).collect();
                let default = values.get(&option.default).or_else(|| list.first().copied()).cloned().unwrap_or_default();
                json!({ "key": key, "kind": "choice", "name": line(&option.name, &option.id), "default": default, "values": list, "value_names": names })
            })
            .collect()
    }

    /// The layer's `when` in the form; Err with the note when it stands for nothing the form can choose.
    fn when(&self, layer: &str, when: &When) -> std::result::Result<Option<Value>, Note> {
        let mut out = Map::new();
        if !when.version.is_empty() {
            let regions: BTreeSet<&str> = when.version.iter().filter_map(|l| region_of(*l)).collect();
            if regions.is_empty() {
                return Err(Note { level: Level::Lost, text: say!("LumaLink.Convert.RegionNone", layer = layer) });
            }
            if regions.len() < luma_validate::REGIONS.len() {
                out.insert("region".into(), json!(regions.into_iter().collect::<Vec<_>>()));
            }
        }
        if !when.option.is_empty() {
            let mut options = Map::new();
            for (id, choices) in &when.option {
                let Some((key, values)) = self.options.get(id) else { continue };
                let chosen: Vec<&String> = choices.iter().filter_map(|c| values.get(c)).collect();
                options.insert(key.clone(), json!(chosen));
            }
            out.insert("option".into(), Value::Object(options));
        }
        Ok((!out.is_empty()).then_some(Value::Object(out)))
    }
}

/// One line of 1 to 64 characters players read (manifest line_ok): the name, else the id it stands for.
fn line(name: &str, fallback: &str) -> String {
    let one: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(64).collect();
    if one.is_empty() {
        fallback.chars().take(64).collect()
    } else {
        one
    }
}

/// A layer's recognised patches as the game's numbers its code sets.
fn layer_numbers(switches: &BTreeMap<u32, u32>) -> Vec<(&'static str, f64)> {
    switches.iter().filter_map(|(id, number)| crate::known_patches::number(*id, *number)).collect()
}

/// The previous engine's rows of a layer, as it read them (OLD-ROWS, per layer); empty when it has none.
fn layer_rows(layer: &Layer) -> Map<String, Value> {
    let mut rows = Map::new();
    if !layer.object_changes.is_empty() {
        rows.insert("object_changes".into(), json!(layer.object_changes));
    }
    if !layer.comet_limits.is_empty() {
        rows.insert("comet_time_limits".into(), json!(layer.comet_limits));
    }
    if !layer.player_constants.is_empty() {
        rows.insert("player_constants".into(), json!(layer.player_constants));
    }
    rows
}

/// A layer as written: its form name, its `when`, its files and the previous engine's rows.
type OutLayer<'a> = (String, Option<Value>, &'a BTreeMap<String, PlannedFile>, Map<String, Value>);

fn changes(files: &BTreeMap<String, PlannedFile>) -> Vec<Change> {
    files.values().map(|f| Change { path: f.disc.trim_start_matches('/').to_string(), source: f.host.clone(), merge: false }).collect()
}

/// Cooks every layer as the game will, to find the files it cannot use: (file, why) pairs.
fn cook_check(layers: &[(String, Vec<Change>)], game_id: &str, store: Option<&GameStore>, scratch: &Path) -> Result<Vec<(String, String)>> {
    // One at a time: the cook's notes and facts are the whole program's.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // What the cook notes and records of each file is taken here: the files it refuses are the list it returns.
    let _reading = refuse::reading();
    let mut failed = Vec::new();
    // A whole file at a path the player's game does not have, of a kind the game finds only at its own paths (a
    // Korean or Chinese disc's language folder, which no release Nebula takes has): the validator refuses it (L302),
    // so it is named and left out here, as a file the cook cannot use is. A source form is the cook's to judge.
    if let Some(store) = store {
        use luma_validate::Store as _;
        let pak = nebula_import::store_view::PakStore::open(&store.path).with_context(|| say!("LumaLink.CannotRead", path = store.path.display()))?;
        for (_, changes) in layers {
            for c in changes {
                let lower = c.path.trim_start_matches('/').to_ascii_lowercase();
                let source = luma_validate::SOURCE_ENDINGS.iter().any(|e| lower.ends_with(e));
                if !source && pak.path(&lower).is_none() && !pak.may_be_new(&lower) {
                    let path = c.path.trim_start_matches('/');
                    failed.push((path.to_string(), say!("LumaLink.Convert.NotInGame", path = path)));
                }
            }
        }
    }
    fs::create_dir_all(scratch)?;
    for (name, changes) in layers.iter().filter(|(_, c)| !c.is_empty()) {
        let mut stats = Stats::default();
        let mut facts = Vec::new();
        let out = scratch.join(format!("{name}.pak"));
        match overlay::write_overlay(game_id, changes, &PlanPatches::default(), store.map(|s| s.path.as_path()), &out, &mut facts, &mut stats) {
            Ok(bad) => failed.extend(bad),
            Err(error) => {
                let why = say!("LumaLink.Convert.LayerFailed", why = format!("{error:#}"));
                failed.extend(changes.iter().map(|c| (c.path.clone(), why.clone())));
            }
        }
        refuse::take_notes();
        refuse::take_facts();
    }
    let _ = fs::remove_dir_all(scratch);
    // A path named twice (left out above, then by the cook too) is said once, with its first reason.
    let mut seen = BTreeSet::new();
    failed.retain(|(path, _)| seen.insert(path.clone()));
    Ok(failed)
}

/// A layer's whole game tables as patches by key (or whole rows, for a table named by place) in `patches` (L303: the
/// one form does not replace a table that has rows); the paths written so (or the game's own, left out), which are not
/// copied as files. A file that reads as no table stays a file, for the validator to name.
fn table_patches(files: &BTreeMap<String, PlannedFile>, failed: &BTreeSet<String>, store: &GameStore, patches: &Path, notes: &mut Vec<Note>) -> Result<BTreeSet<String>> {
    use crate::table_patch::{patch_of, read_table};
    let mut done = BTreeSet::new();
    let mut pak = None;
    for file in files.values() {
        let relative = file.disc.trim_start_matches('/');
        if failed.contains(relative) {
            continue;
        }
        if pak.is_none() {
            pak = Some(nebula_import::store_view::PakStore::open(&store.path).with_context(|| say!("LumaLink.CannotRead", path = store.path.display()))?);
        }
        // A damaged table of the game stops the conversion (named), never passes as "no table".
        let Some(game) = pak.as_ref().map(|p| p.table_ir(relative)).transpose()?.flatten() else { continue };
        let bytes = fs::read(&file.host).with_context(|| say!("LumaLink.CannotRead", path = file.host.display()))?;
        let Ok(mods) = read_table(&bytes) else { continue };
        let patched = match patch_of(relative, &game, &mods, &crate::inspect::ColumnNames::none()) {
            Ok(p) => p,
            Err(why) => {
                notes.push(Note { level: Level::Lost, text: say!("LumaLink.Convert.TableNotPatched", path = relative, why = why) });
                done.insert(relative.to_string());
                continue;
            }
        };
        done.insert(relative.to_string());
        let Some(patch) = &patched.patch else {
            notes.push(Note { level: Level::Info, text: say!("LumaLink.Convert.TableSame", path = relative) });
            continue;
        };
        let Some(safe) = crate::source::safe_relative(relative) else {
            bail!(say!("LumaLink.Convert.NotAPlace", path = relative));
        };
        let target = patches.join(format!("{}.json", safe.display()));
        fs::create_dir_all(target.parent().unwrap_or(patches))?;
        fs::write(&target, serde_json::to_vec_pretty(patch)?).with_context(|| say!("LumaLink.CannotMake", path = target.display()))?;
        let text = if patch.get("rows").is_some() {
            say!("LumaLink.Convert.TableRows", path = relative)
        } else {
            say!("LumaLink.Convert.TablePatched", path = relative, set = patched.set, added = patched.added, removed = patched.removed)
        };
        notes.push(Note { level: Level::Info, text });
        if !patched.lost_columns.is_empty() {
            notes.push(Note { level: Level::Lost, text: say!("LumaLink.Convert.TableColumnsLost", path = relative, columns = patched.lost_columns.join(", ")) });
        }
        if !patched.repeated.is_empty() {
            notes.push(Note { level: Level::Lost, text: say!("LumaLink.Convert.TableRepeated", path = relative, keys = patched.repeated.join("; ")) });
        }
    }
    Ok(done)
}

/// Copies a layer's files the game can use to `<folder>/layers/<layer>/files/<game path>`; the count written.
fn copy_files(files: &BTreeMap<String, PlannedFile>, failed: &BTreeSet<String>, folder: &Path, layer: &str) -> Result<usize> {
    let mut written = 0;
    for file in files.values() {
        let relative = file.disc.trim_start_matches('/');
        if failed.contains(relative) {
            continue;
        }
        let Some(safe) = crate::source::safe_relative(relative) else {
            bail!(say!("LumaLink.Convert.NotAPlace", path = relative));
        };
        let target = folder.join("layers").join(layer).join("files").join(safe);
        fs::create_dir_all(target.parent().unwrap_or(folder))?;
        fs::copy(&file.host, &target).with_context(|| say!("LumaLink.CannotRead", path = file.host.display()))?;
        written += 1;
    }
    Ok(written)
}

/// A texture of a pack the game can take: a `.png` or `.dds`, not a note, not in a folder of another game's.
fn texture_wanted(path: &Path, root: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if !(lower.ends_with(".png") || lower.ends_with(".dds")) || path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("logo.png")) {
        return false;
    }
    let relative = path.strip_prefix(root).unwrap_or(path);
    !relative.components().any(|part| crate::layout::is_other_game_folder(&part.as_os_str().to_string_lossy()))
}

/// luma.json's text: the form's keys in luma-format 4's order, each value as JSON.
fn manifest_text(fields: &[(&str, Value)]) -> String {
    let lines: Vec<String> = fields
        .iter()
        .map(|(k, v)| {
            let value = serde_json::to_string_pretty(v).expect("JSON").replace('\n', "\n  ");
            format!("  \"{k}\": {value}")
        })
        .collect();
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

/// Writes the mod at `input` as a new mod folder `folder` in the one form.
pub fn convert(input: &Path, folder: &Path, store: Option<&GameStore>, given: &Given) -> Result<Converted> {
    if folder.exists() {
        bail!(say!("LumaLink.AlreadyThere", path = folder.display()));
    }
    let game_id = store.map_or(crate::plan::ANY_VERSION, |s| s.game_id.as_str());
    let work = tempfile::tempdir()?;
    let root = crate::source::open(input, work.path())?;
    let layout = Layout::of(&root);
    let plan = crate::plan::build(&layout, game_id, store)?;
    let staging = tempfile::tempdir_in(folder.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
    let mut out = write(input, &layout, &plan, store, staging.path(), given)?;
    fs::rename(staging.path(), folder).or_else(|_| copy_tree(staging.path(), folder)).with_context(|| say!("LumaLink.CannotMake", path = folder.display()))?;
    out.notes.splice(0..0, plan.notes.iter().cloned());
    Ok(out)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(from) {
        let entry = entry?;
        let target = to.join(entry.path().strip_prefix(from)?);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// The folder's contents, written into `building` (empty).
pub fn write(input: &Path, layout: &Layout, plan: &Plan, store: Option<&GameStore>, building: &Path, given: &Given) -> Result<Converted> {
    let mut out = Converted::default();
    let keys = Keys::of(plan);
    // The cook's store names a disc: the player's, or the first release's when no game is given (nothing is read from it).
    let game_id = store.map_or("RMGE01", |s| s.game_id.as_str());

    // Layers: base, then the plan's, under the form's names.
    let mut names = BTreeSet::from(["base".to_string()]);
    let mut layers: Vec<OutLayer> = Vec::new();
    let mut numbers: Vec<crate::numbers_code::LayerNumbers> = Vec::new();
    let base = Layer { object_changes: plan.object_changes.clone(), comet_limits: plan.comet_limits.clone(), player_constants: plan.player_constants.clone(), ..Layer::default() };
    layers.push(("base".into(), None, &plan.files, layer_rows(&base)));
    numbers.push(crate::numbers_code::LayerNumbers { layer: "base".into(), when: None, numbers: layer_numbers(&plan.native_patches) });
    for layer in &plan.layers {
        let name = unused(key_of(&layer.name), &names);
        match keys.when(&name, &layer.when) {
            Ok(when) => {
                names.insert(name.clone());
                numbers.push(crate::numbers_code::LayerNumbers { layer: name.clone(), when: when.clone(), numbers: layer_numbers(&layer.switches) });
                layers.push((name, when, &layer.files, layer_rows(layer)));
            }
            Err(note) => out.notes.push(note),
        }
    }

    // The files the game cannot use, found by cooking each layer as it will.
    let jobs: Vec<(String, Vec<Change>)> = layers.iter().map(|(n, _, files, _)| (n.clone(), changes(files))).collect();
    let failed = cook_check(&jobs, game_id, store, &building.join(".checking"))?;
    if !failed.is_empty() && !given.accept_losses {
        return Err(Losses(failed).into());
    }
    let failed_paths: BTreeSet<String> = failed.iter().map(|(p, _)| p.clone()).collect();
    out.left_out = failed;
    for (name, _, files, _) in &layers {
        let mut skip = failed_paths.clone();
        if let Some(store) = store {
            skip.extend(table_patches(files, &failed_paths, store, &building.join("layers").join(name).join("patches"), &mut out.notes)?);
        }
        out.files += copy_files(files, &skip, building, name)?;
    }

    // A texture pack: its textures under the game's names, in the base layer (decision 3).
    let pack = layout.pack.as_ref();
    if let Some(pack) = pack {
        let names = store.map(|s| crate::texture_pack::store_names(&s.path)).unwrap_or_default();
        let root = pack.folder.clone();
        let made = crate::texture_pack::convert_into(&pack.folder, &names, &building.join("layers/base/textures"), move |p| texture_wanted(p, &root))?;
        out.textures = made.written;
        if !made.kept.is_empty() {
            out.notes.push(Note { level: Level::Info, text: say!("LumaLink.Convert.TexturesKept", count = made.kept.len()) });
        }
    }

    // The previous engine's rows: per layer, and the whole mod's.
    let mut pre = Map::new();
    if !plan.event_flags.is_empty() {
        pre.insert("event_flags".into(), json!(plan.event_flags));
    }
    if !plan.comet_schedule.is_empty() {
        pre.insert("comet_schedule".into(), Value::Array(plan.comet_schedule.iter().map(|(dome, comets)| json!({ "dome": dome, "comets": comets })).collect()));
    }
    if !plan.new_galaxies.is_empty() {
        pre.insert("galaxies".into(), Value::Array(plan.new_galaxies.iter().map(|name| json!({ "name": name })).collect()));
    }
    if plan.save_folder.is_some() || plan.adds_galaxy {
        pre.insert("save".into(), json!({ "mode": if plan.save_clone { "copy" } else { "own" } }));
        if plan.save_folder.as_ref().is_some_and(|s| s.is_dir()) {
            out.notes.push(Note { level: Level::Lost, text: say!("LumaLink.Convert.SaveFiles") });
        }
    }
    let rows: Map<String, Value> = layers.iter().filter(|(_, _, _, r)| !r.is_empty()).map(|(n, _, _, r)| (n.clone(), Value::Object(r.clone()))).collect();
    if !rows.is_empty() {
        pre.insert("layers".into(), Value::Object(rows));
    }

    // A layer with nothing in it is left out of the list (the validator wants each listed layer's folder or rows).
    let listed: Vec<Value> = layers
        .iter()
        .filter(|(n, _, _, r)| !r.is_empty() || building.join("layers").join(n).is_dir())
        .map(|(n, when, _, _)| match when {
            Some(when) => json!({ "name": n, "when": when }),
            None => json!({ "name": n }),
        })
        .collect();
    if listed.is_empty() && pre.is_empty() {
        // A Riivolution pack for another disc, or a texture pack of another game's names only.
        let other = plan.for_another_game.clone().or_else(|| pack.filter(|p| p.textures == 0).and_then(|p| p.other_games.keys().next().cloned()));
        if let Some(other) = other {
            bail!(say!("LumaLink.Convert.OtherGame", path = input.display(), game = other));
        }
        // Every file was left out: each is said with what left it out (a layer the cook could not write, say), not
        // only that nothing remains.
        let mut said = say!("LumaLink.Convert.Nothing", path = input.display());
        for (path, why) in &out.left_out {
            said.push_str("\n  ");
            said.push_str(&say!("LumaLink.Convert.LeftOut", path = path, why = why));
        }
        bail!(said);
    }

    // About the mod.
    let file_name = crate::source::mod_name(input);
    let shown = given.name.clone().or_else(|| pack.map(|p| p.title.clone()).filter(|t| !t.trim().is_empty())).unwrap_or(file_name);
    let name = line(&shown, "mod");
    out.id = id_of(&name);
    // The code patches with no setting: each with its game function and a hook to start from, for the modder.
    if let Some((text, count)) = crate::code_stubs::text(&name, &plan.unknown_code) {
        let path = building.join(crate::code_stubs::FILE);
        fs::create_dir_all(building.join("source"))?;
        fs::write(&path, text).with_context(|| say!("LumaLink.CannotMake", path = path.display()))?;
        out.notes.push(Note { level: Level::Info, text: say!("LumaLink.Convert.CodeStubs", count = count, file = crate::code_stubs::FILE) });
    }
    // The code patches that are the game's numbers: the mod's code sets them.
    let made = crate::numbers_code::text(&name, &numbers);
    for layer in &made.untested {
        out.notes.push(Note { level: Level::Lost, text: say!("LumaLink.Convert.NumbersUntested", layer = layer) });
    }
    if let Some(program) = &made.program {
        let path = building.join(crate::numbers_code::FILE);
        fs::create_dir_all(building.join("source"))?;
        fs::write(&path, program).with_context(|| say!("LumaLink.CannotMake", path = path.display()))?;
        out.notes.push(Note { level: Level::Info, text: say!("LumaLink.Convert.NumbersCode", count = made.count, file = crate::numbers_code::FILE) });
    }
    let summary = pack
        .map(|p| p.description.lines().next().unwrap_or("").trim().chars().take(200).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name.clone());
    let mut authors: Vec<String> = if given.authors.is_empty() { pack.map(|p| p.authors.clone()).unwrap_or_default() } else { given.authors.clone() };
    authors.retain(|a| !a.trim().is_empty());
    authors.truncate(16);
    if authors.is_empty() {
        authors.push(say!("LumaLink.Convert.AuthorUnknown"));
        out.notes.push(Note { level: Level::Info, text: say!("LumaLink.Convert.NoAuthors") });
    }
    let version = pack.map(|p| p.version.trim().to_string()).filter(|v| luma_validate::semver::Version::parse(v).is_some()).unwrap_or_else(|| "1.0.0".to_string());
    let mut fields: Vec<(&str, Value)> = vec![
        ("format", format!("{}.{}", luma_validate::FORMAT.0, luma_validate::FORMAT.1).into()),
        ("id", out.id.clone().into()),
        ("version", version.into()),
        ("name", name.into()),
        ("summary", summary.into()),
        ("authors", authors.into()),
        ("game", json!({ "id": "RMG" })),
    ];
    if let Some(home) = pack.map(|p| p.homepage.clone()).filter(|h| h.starts_with("https://")) {
        fields.push(("links", json!({ "home": home })));
    }
    if let Some(logo) = pack.and_then(|p| p.logo.as_ref()) {
        fs::create_dir_all(building.join("media"))?;
        if fs::copy(logo, building.join("media/icon.png")).is_ok() {
            fields.push(("icon", "media/icon.png".into()));
        }
    }
    if !plan.options.is_empty() {
        fields.push(("options", Value::Array(keys.options(plan))));
    }
    fields.push(("layers", Value::Array(listed)));
    if !pre.is_empty() {
        fields.push(("pre_standard", Value::Object(pre)));
    }
    fs::write(building.join("luma.json"), manifest_text(&fields))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn keys_and_ids_are_the_forms() {
        assert_eq!(key_of("Hard Mode!"), "hard-mode-");
        assert_eq!(key_of("2x"), "o-2x");
        assert_eq!(id_of("HD"), "hd-mod");
        assert_eq!(id_of("2D Mario"), "mod-2d-mario");
        let key = key_of("Ünïcode");
        assert!(key.starts_with(|c: char| c.is_ascii_lowercase()) && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._".contains(&b)), "{key}");
    }

    #[test]
    fn a_riivolution_pack_becomes_the_one_form_and_the_validator_passes_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Pack");
        put(&source, "riivolution/pack.xml", br#"<wiidisc version="1"><id game="RMG"/><options><section name="Pack">
  <option name="Look"><choice name="Night Sky"><patch id="night"/></choice><choice name="Day"><patch id="day"/></choice></option></section></options>
  <patch id="night"><file disc="/StageData/note.txt.bin" external="/Pack/note.txt.bin" create="true"/></patch>
  <patch id="day"><file disc="/StageData/day.txt.bin" external="/Pack/note.txt.bin" create="true"/></patch></wiidisc>"#);
        put(&source, "Pack/note.txt.bin", b"hello");
        let folder = dir.path().join("out");
        let given = Given { authors: vec!["Someone".into()], accept_losses: true, ..Given::default() };
        let done = convert(&source, &folder, None, &given).unwrap();
        let text = fs::read_to_string(folder.join("luma.json")).unwrap();
        let manifest: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest["format"], "1.0", "{text}");
        assert_eq!(manifest["authors"], json!(["Someone"]));
        assert_eq!(manifest["options"][0]["kind"], "choice", "{text}");
        let report = luma_validate::validate(&luma_validate::Input {
            source: luma_validate::Source::Folder(&folder),
            game: None,
            installed: &[],
            platforms: &luma_validate::PLATFORMS,
            options: None,
            position: None,
        });
        let errors: Vec<_> = report.findings.iter().filter(|f| f.level == luma_validate::Level::Error).collect();
        assert!(errors.is_empty(), "{errors:?}\n{text}\n{:?}", done.left_out);
    }

    #[test]
    fn a_file_the_cook_refuses_is_left_out_and_what_the_cook_said_of_it_is_taken() {
        // An empty wave archive (as the corpus's Peach and Rosalina mods ship one): the cook refuses it and records the
        // refusal; the check takes what it records, so nothing is left on the thread for a later cook to find.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Pack");
        put(&source, "riivolution/pack.xml", br#"<wiidisc version="1"><id game="RMG"/><options><section name="Pack">
  <option name="On"><choice name="Enabled"><patch id="on"/></choice></option></section></options>
  <patch id="on"><folder external="/Pack/AudioRes" disc="/AudioRes" recursive="true" create="true"/>
  <file disc="/StageData/note.txt.bin" external="/Pack/note.txt.bin" create="true"/></patch></wiidisc>"#);
        put(&source, "Pack/AudioRes/Waves/Empty_0.aw", b"");
        put(&source, "Pack/note.txt.bin", b"hello");
        let folder = dir.path().join("out");
        let given = Given { authors: vec!["Someone".into()], accept_losses: true, ..Given::default() };
        let done = convert(&source, &folder, None, &given).unwrap();
        let left: Vec<&str> = done.left_out.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(left, ["AudioRes/Waves/Empty_0.aw"], "{:?}", done.left_out);
        assert!(!folder.join("layers/base/files/AudioRes/Waves/Empty_0.aw").exists());
        assert!(folder.join("layers/base/files/StageData/note.txt.bin").is_file());
        assert!(refuse::take_facts().is_empty() && refuse::take_notes().is_empty());
        // When that file is all there is, what left it out is said with "nothing can be used".
        fs::remove_file(source.join("Pack/note.txt.bin")).unwrap();
        fs::write(source.join("riivolution/pack.xml"), br#"<wiidisc version="1"><id game="RMG"/><options><section name="Pack">
  <option name="On"><choice name="Enabled"><patch id="on"/></choice></option></section></options>
  <patch id="on"><folder external="/Pack/AudioRes" disc="/AudioRes" recursive="true" create="true"/></patch></wiidisc>"#).unwrap();
        let error = convert(&source, &dir.path().join("none"), None, &given).unwrap_err().to_string();
        assert!(error.contains(&say!("LumaLink.Convert.LeftOut", path = "AudioRes/Waves/Empty_0.aw", why = "")) && error.contains(&done.left_out[0].1), "{error}");
        assert!(!dir.path().join("none").exists());
    }
}
