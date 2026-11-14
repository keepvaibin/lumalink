//! Decides where every file of a mod goes, and says why for each thing it does not use.

use crate::game_store::GameStore;
use crate::known_patches;
use crate::say;
use crate::layout::{self, Layout, PatchFile};
use crate::riivolution::{self, Disc, FilePatch, FolderPatch, MemoryPatch};
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One file the mod sets.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlannedFile {
    pub disc: String,
    pub host: PathBuf,
}

/// How serious a note is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    /// Information only.
    Info,
    /// Part of the mod does not reach the game.
    Lost,
    /// The mod cannot be used.
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Note {
    pub level: Level,
    pub text: String,
}

/// A code patch with no native version.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UnknownCode {
    /// In the first release (RMGE01): where the game's functions are known.
    pub address: u32,
    pub bytes: usize,
    pub kind: &'static str,
    /// The release the patch is for (its disc id's fourth letter), and the address as the patch gives it.
    #[serde(skip)]
    pub release: char,
    #[serde(skip)]
    pub own: u32,
    /// What the mod writes, and what the game has there (the patch's `original`, else the imported game's program).
    #[serde(skip)]
    pub value: Vec<u8>,
    #[serde(skip)]
    pub original: Vec<u8>,
}

/// When a layer is used. Empty: always.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct When {
    pub version: Vec<char>,
    pub option: BTreeMap<String, Vec<String>>,
}

/// A part of a mod used only sometimes.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Layer {
    pub name: String,
    pub when: When,
    pub files: BTreeMap<String, PlannedFile>,
    pub switches: BTreeMap<u32, u32>,
    pub object_changes: Vec<known_patches::ObjectChangeRow>,
    pub comet_limits: Vec<known_patches::CometLimitRow>,
    pub player_constants: crate::player_constants::Constants,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct OptionChoice {
    pub id: String,
    pub name: String,
}

/// Something the player chooses in the game.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PlayerOption {
    pub id: String,
    pub name: String,
    pub default: String,
    pub choices: Vec<OptionChoice>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Plan {
    pub game_id: String,
    /// What is always used, by the lower-case path.
    pub files: BTreeMap<String, PlannedFile>,
    /// The switches always on.
    pub native_patches: BTreeMap<u32, u32>,
    /// Rows of the game's progression a mod's patches set.
    pub event_flags: Vec<known_patches::EventFlagRow>,
    /// Each dome's comets a mod's patches set.
    pub comet_schedule: BTreeMap<u32, Vec<known_patches::CometRow>>,
    pub object_changes: Vec<known_patches::ObjectChangeRow>,
    pub comet_limits: Vec<known_patches::CometLimitRow>,
    pub player_constants: crate::player_constants::Constants,
    /// What is used only for some versions of the game or some of the player's choices.
    pub layers: Vec<Layer>,
    pub options: Vec<PlayerOption>,
    pub notes: Vec<Note>,
    pub known_patches: Vec<String>,
    pub unknown_code: Vec<UnknownCode>,
    pub save_folder: Option<PathBuf>,
    pub save_clone: bool,
    /// The id a patch names when it is for another game (`SB4E01`, or the three letters of one).
    pub for_another_game: Option<String>,
    /// It adds a galaxy: its saves have to be its own, since a save's shape follows the galaxy list.
    pub adds_galaxy: bool,
    /// The galaxies it adds, by their folder names.
    pub new_galaxies: Vec<String>,
}

/// A language's HOME menu layout (`UsEnglish/LayoutData/HomeButton.arc`), which console_only names by its
/// store path.
fn is_language_home_menu(disc: &str) -> bool {
    disc.to_ascii_lowercase().ends_with("/layoutdata/homebutton.arc")
}

impl Plan {
    fn note(&mut self, level: Level, text: impl Into<String>) {
        self.notes.push(Note { level, text: text.into() });
    }

    fn set(&mut self, disc: &str, host: &Path) {
        let disc = if disc.starts_with('/') { disc.to_string() } else { format!("/{disc}") };
        if crate::source::safe_relative(disc.trim_start_matches('/')).is_none() {
            self.note(Level::Lost, say!("LumaLink.Convert.NotAPlace", path = disc));
            return;
        }
        if disc.to_ascii_lowercase().starts_with("/nebuladata/") && !disc.eq_ignore_ascii_case("/NebulaData/GalaxyInfoTable.bcsv") {
            self.note(Level::Lost, say!("LumaLink.Plan.NebulaTables", path = disc));
            return;
        }
        // A console's own files (banner, HOME menu, module list): Nebula has none of those.
        if nebula_import::cook::console_only(disc.trim_start_matches('/')) || is_language_home_menu(&disc) {
            self.note(Level::Info, say!("LumaLink.Plan.ConsoleOnly", path = disc));
            return;
        }
        // Keyed as the store names it: two of the mod's paths may be one place in the game (a zone's archive both at
        // StageData/ and in a galaxy's folder). The game reads the one where its own file is, the shallower.
        let key = format!("/{}", nebula_import::cook::store::store_path(disc.trim_start_matches('/')).to_ascii_lowercase());
        if let Some(have) = self.files.get(&key) {
            if !have.disc.eq_ignore_ascii_case(&disc) {
                let depth = |path: &str| path.matches('/').count();
                let (kept, dropped) = if depth(&have.disc) <= depth(&disc) { (have.disc.clone(), disc.clone()) } else { (disc.clone(), have.disc.clone()) };
                self.note(Level::Info, say!("LumaLink.Plan.SameFile", path = dropped, kept = kept));
                if kept != disc {
                    return;
                }
            }
        }
        self.files.insert(key, PlannedFile { disc, host: host.to_path_buf() });
    }

    /// An empty file where the game's own is empty too (`AudioRes/Waves/B109take_0.aw`, which mods that copy the
    /// whole wave folder carry) changes nothing. `empty_in_game` says, by lower-case path without the leading slash.
    fn leave_out_empty_as_in_game(&mut self, empty_in_game: &dyn Fn(&str) -> bool) {
        let empty = |file: &PlannedFile| fs::metadata(&file.host).is_ok_and(|m| m.is_file() && m.len() == 0) && empty_in_game(&file.disc.trim_start_matches('/').to_ascii_lowercase());
        let mut left = Vec::new();
        self.files.retain(|_, file| {
            let keep = !empty(file);
            if !keep {
                left.push(file.disc.clone());
            }
            keep
        });
        for layer in &mut self.layers {
            layer.files.retain(|_, file| {
                let keep = !empty(file);
                if !keep {
                    left.push(file.disc.clone());
                }
                keep
            });
        }
        left.sort();
        left.dedup();
        for disc in left {
            self.note(Level::Info, say!("LumaLink.Plan.EmptyAsGame", path = disc));
        }
    }

    pub fn blocked(&self) -> bool {
        self.notes.iter().any(|n| n.level == Level::Blocked)
    }

    /// How many different files it can set, across everything.
    pub fn file_count(&self) -> usize {
        let mut all: std::collections::BTreeSet<&String> = self.files.keys().collect();
        for layer in &self.layers {
            all.extend(layer.files.keys());
        }
        all.len()
    }
}

/// A relative path in a mod to a file, in any case. `..` may not leave the root.
fn resolve(sd_root: &Path, patch_root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = relative.replace('\\', "/");
    let start = if relative.starts_with('/') { sd_root } else { patch_root };
    let mut result = start.to_path_buf();
    let mut depth = 0usize;
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth = depth.checked_sub(1)?;
                result.pop();
            }
            part => {
                if part.contains(':') {
                    return None;
                }
                depth += 1;
                let exact = result.join(part);
                if exact.exists() {
                    result = exact;
                    continue;
                }
                let found = fs::read_dir(&result).ok().and_then(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
                        .map(|e| e.path())
                });
                result = found.unwrap_or(exact);
            }
        }
    }
    Some(result)
}

fn join(a: &str, b: &str) -> String {
    let a = a.trim_end_matches('/');
    let b = b.trim_start_matches('/');
    match (a.is_empty(), b.is_empty()) {
        (true, _) => format!("/{b}"),
        (_, true) => a.to_string(),
        _ => format!("{a}/{b}"),
    }
}

/// Every version of the game, by the letter in its id.
const VERSION_LETTERS: [char; 6] = ['E', 'P', 'J', 'K', 'W', 'C'];

/// What the game's id in a package says: this game, whichever version.
pub const ANY_VERSION: &str = "RMG";

/// The version a patch file was written for, when it names only one.
fn version_named(target: &riivolution::Target, game_id: &str) -> String {
    if target.accepts(game_id) {
        return game_id.to_string();
    }
    let letter = target.game.as_deref().and_then(|g| g.chars().nth(3)).or_else(|| target.regions.first().and_then(|r| r.chars().next()));
    letter.map_or_else(|| game_id.to_string(), |l| format!("RMG{l}01"))
}

/// A name for people from a file's name: words apart, and capitals if it has none at all.
pub fn nice_name(raw: &str) -> String {
    let spaced = raw.replace(['_', '-'], " ");
    let mut out = String::new();
    let mut last = ' ';
    for c in spaced.chars() {
        if c.is_uppercase() && last.is_lowercase() {
            out.push(' ');
        }
        out.push(c);
        last = c;
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    if out.chars().any(char::is_uppercase) {
        return words.join(" ");
    }
    words
        .iter()
        .map(|word| {
            if word.chars().any(|c| c.is_ascii_digit()) {
                return (*word).to_string();
            }
            let mut letters = word.chars();
            letters.next().map_or_else(String::new, |first| first.to_uppercase().chain(letters).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A name and the version at its end, if it has one: "Kaizo V2.1" is Kaizo, version 2.1.
pub fn split_version(name: &str) -> (String, Option<String>) {
    if let Some((before, last)) = name.rsplit_once(' ') {
        let digits = last.strip_prefix(['v', 'V']).unwrap_or(last);
        let named = last.len() != digits.len() || digits.contains('.');
        if named && !digits.is_empty() && digits.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())) {
            return (before.trim().to_string(), Some(digits.to_string()));
        }
    }
    (name.to_string(), None)
}

/// A name as an id: lower case letters and digits, with `-` between words.
pub fn slug(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let mut id = id.trim_end_matches('-').to_string();
    id.truncate(58);
    let id = id.trim_end_matches('-').to_string();
    if id.is_empty() {
        "mod".to_string()
    } else if reserved_id(&id) {
        format!("{id}-mod")
    } else {
        id
    }
}

/// A mod's id from its name. Letters outside ASCII make no part of an id, so a name with any gets a mark of the whole
/// name too: two mods named in Japanese are two mods, not one replacing the other.
pub fn mod_id(name: &str) -> String {
    let id = slug(name);
    if !name.chars().any(|c| c.is_alphanumeric() && !c.is_ascii()) {
        return id;
    }
    // the same on every system and in every build.
    let mark = name.bytes().fold(0x811c_9dc5u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193));
    let mut head = id;
    head.truncate(49);
    let head = head.trim_end_matches('-');
    format!("{head}-{mark:08x}")
}

/// `id`, or the first of `id-2`, `id-3`... that `taken` doesn't hold.
fn unused(id: String, taken: impl Fn(&str) -> bool) -> String {
    if !taken(&id) {
        return id;
    }
    (2..).map(|n| format!("{id}-{n}")).find(|candidate| !taken(candidate)).unwrap_or(id)
}

/// Names Windows keeps for itself, which no folder can have.
pub(crate) fn reserved_id(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul") || (id.len() == 4 && (id.starts_with("com") || id.starts_with("lpt")) && id.as_bytes()[3].is_ascii_digit())
}

struct Builder<'a> {
    plan: Plan,
    store: Option<&'a GameStore>,
    spare: known_patches::SpareMemory,
    comet_hook: Option<u32>,
    /// The version of the game the patch being read was written for. Every table here is the first release's: others are read through `region_map`.
    version: char,
}

/// The game's own program, to compare a mod's with: `--base`, or beside the imported game.
static BASE_PROGRAM: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// A program file (main.dol) to compare a mod's with.
pub fn set_base_program(path: Option<std::path::PathBuf>) {
    *BASE_PROGRAM.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

fn base_program(store: Option<&GameStore>) -> Option<Vec<u8>> {
    if let Some(path) = BASE_PROGRAM.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return fs::read(path).ok();
    }
    let folder = store?.path.parent()?.to_path_buf();
    fs::read(folder.join("game-inputs").join("sys").join("main.dol")).ok()
}

/// What the game has where each code patch writes, from the imported game's program when the patch is for its release.
fn game_instructions(unknown: &mut [UnknownCode], store: Option<&GameStore>) {
    let Some(release) = store.and_then(|s| s.game_id.chars().nth(3)) else {
        return;
    };
    if !unknown.iter().any(|u| u.original.is_empty() && !u.value.is_empty() && matches!(u.kind, "memory" | "code file") && u.release == release) {
        return;
    }
    let Some(program) = base_program(store) else {
        return;
    };
    for u in unknown.iter_mut().filter(|u| u.original.is_empty() && !u.value.is_empty() && matches!(u.kind, "memory" | "code file") && u.release == release) {
        if let Some(bytes) = crate::gecko::program_bytes(&program, u.own | 0x8000_0000, u.value.len()) {
            u.original = bytes.to_vec();
        }
    }
}

impl Builder<'_> {
    fn file(&mut self, sd_root: &Path, patch_root: &Path, f: &FilePatch) {
        let Some(host) = resolve(sd_root, patch_root, &f.external).filter(|h| h.is_file()) else {
            self.plan.note(Level::Lost, say!("LumaLink.Plan.NotInMod", path = f.external, disc = f.disc));
            return;
        };
        if f.offset != 0 || f.file_offset != 0 || f.length != 0 || !f.resize {
            self.plan.note(Level::Lost, say!("LumaLink.Plan.PatchedInPart", path = f.disc));
            return;
        }
        if f.disc.eq_ignore_ascii_case("main.dol") || f.disc.eq_ignore_ascii_case("/main.dol") {
            self.changed_program(sd_root, patch_root, &host);
            return;
        }
        if f.disc.starts_with('/') {
            if let Some(store) = self.store {
                if !f.create && !store.has(&f.disc) {
                    self.plan.note(Level::Lost, say!("LumaLink.Plan.NoSuchFile", path = f.disc));
                    return;
                }
            }
            self.plan.set(&f.disc, &host);
            return;
        }
        // A bare name: every file of that name in the game.
        match self.store {
            Some(store) => {
                let paths = store.named(&f.disc);
                if paths.is_empty() {
                    self.plan.note(Level::Lost, say!("LumaLink.Plan.NoFileNamed", name = f.disc));
                }
                for path in paths {
                    self.plan.set(&path, &host);
                }
            }
            None => self.plan.note(Level::Lost, say!("LumaLink.Plan.NoFolder", name = f.disc)),
        }
    }

    fn folder(&mut self, sd_root: &Path, patch_root: &Path, folder: &FolderPatch, disc: &str, external: &str) {
        let Some(directory) = resolve(sd_root, patch_root, external).filter(|d| d.is_dir()) else {
            // Patches often list folders the download doesn't bring. Riivolution skips those too.
            return;
        };
        let Ok(entries) = fs::read_dir(&directory) else {
            self.plan.note(Level::Lost, say!("LumaLink.CannotRead", path = external));
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            // A link is left out, as in an archive: one to a folder above it would never end.
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                self.plan.note(Level::Info, say!("LumaLink.Plan.Link", path = join(external, &name)));
                continue;
            }
            if kind.is_dir() {
                if folder.recursive {
                    self.folder(sd_root, patch_root, folder, &join(disc, &name), &join(external, &name));
                }
                continue;
            }
            self.file(
                sd_root,
                patch_root,
                &FilePatch {
                    disc: join(disc, &name),
                    external: join(external, &name),
                    resize: folder.resize,
                    create: true,
                    length: folder.length,
                    ..Default::default()
                },
            );
        }
    }

    /// A whole main.dol in a mod: what it changes, against the game's own.
    fn changed_program(&mut self, sd_root: &Path, patch_root: &Path, host: &Path) {
        let Some(changed) = fs::read(host).ok() else {
            return;
        };
        let Some(original) = base_program(self.store) else {
            self.plan.note(Level::Lost, say!("LumaLink.Plan.NeedsBase"));
            return;
        };
        match crate::gecko::program_changes(&changed, &original) {
            Some(writes) => self.writes(sd_root, patch_root, writes),
            None => self.plan.note(Level::Lost, say!("LumaLink.Plan.NotAProgram")),
        }
    }

    /// A plain write from anywhere: a code list, or a changed program.
    fn writes(&mut self, sd_root: &Path, patch_root: &Path, writes: Vec<(u32, Vec<u8>)>) {
        for (offset, value) in writes {
            let patch = MemoryPatch { offset, value, ..Default::default() };
            self.memory(sd_root, patch_root, &patch);
        }
    }

    fn memory(&mut self, sd_root: &Path, patch_root: &Path, patch: &MemoryPatch) {
        // Addresses and the pointers in tables, as the first release has them.
        let moved;
        let own = patch.offset;
        let patch = if self.version == 'E' || !matches!(self.version, 'P' | 'J' | 'K') {
            patch
        } else {
            let mut one = patch.clone();
            one.offset = crate::region_map::to_first_release(patch.offset, self.version);
            if known_patches::is_table(one.offset) {
                one.value = crate::region_map::pointers_to_first_release(&patch.value, self.version);
            }
            moved = one;
            &moved
        };
        // A code list in a file: each write in it is read like a patch's.
        if patch.ocarina && !patch.value_file.is_empty() {
            if let Some(bytes) = resolve(sd_root, patch_root, &patch.value_file).filter(|f| f.is_file()).and_then(|f| fs::read(f).ok()) {
                let list = crate::gecko::from_gct(&bytes);
                for _ in 0..list.other {
                    self.plan.unknown_code.push(UnknownCode { kind: "code list", ..Default::default() });
                }
                self.writes(sd_root, patch_root, list.writes);
                return;
            }
        }
        // Tables in the Wii program that Nebula keeps as data.
        if !patch.ocarina && !patch.value_file.is_empty() {
            let file = resolve(sd_root, patch_root, &patch.value_file).filter(|f| f.is_file());
            let address = patch.offset | 0x8000_0000;
            if let Some(file) = file {
                if address == known_patches::GALAXY_TABLE {
                    self.plan.set("/NebulaData/GalaxyInfoTable.bcsv", &file);
                    self.plan.known_patches.push("LumaLink.Patch.GalaxyMap".to_string());
                    return;
                }
                if address == known_patches::EVENT_FLAG_TABLE {
                    if let Some(rows) = fs::read(&file).ok().and_then(|bytes| known_patches::event_flag_table(&bytes)) {
                        self.plan.event_flags = rows;
                        self.plan.known_patches.push("LumaLink.Patch.HungryLumas".to_string());
                        return;
                    }
                }
            }
        }
        // A value kept in a file says the same as one in the patch.
        let from_file = if !patch.ocarina && !patch.search && !patch.value_file.is_empty() {
            resolve(sd_root, patch_root, &patch.value_file).filter(|f| f.is_file()).and_then(|f| fs::read(f).ok())
        } else {
            None
        };
        let value: &[u8] = from_file.as_deref().unwrap_or(&patch.value);
        if !patch.search && !patch.ocarina && !value.is_empty() {
            if let Some(found) = known_patches::setting(patch.offset, value) {
                self.plan.native_patches.insert(found.id, found.value);
                self.said(found.what);
                return;
            }
            if let Some(found) = crate::player_constants::write(patch.offset, value) {
                crate::player_constants::merge(&mut self.plan.player_constants, found);
                self.said("LumaLink.Patch.PlayerMoves");
                return;
            }
            if let Some(changes) = known_patches::object_table_write(patch.offset, value) {
                self.plan.object_changes.extend(changes);
                self.said("LumaLink.Patch.ObjectTable");
                return;
            }
            if let Some(change) = known_patches::treasure_box_write(patch.offset, value) {
                self.plan.object_changes.push(change);
                self.said("LumaLink.Patch.TreasureBoxes");
                return;
            }
            if let Some(change) = known_patches::display_model_write(patch.offset, value) {
                self.plan.object_changes.push(change);
                self.said("LumaLink.Patch.CollectModels");
                return;
            }
            if let Some(target) = known_patches::comet_limit_hook(patch.offset, value) {
                self.comet_hook = Some(target);
                return;
            }
            if known_patches::find(patch.offset, value).is_none() && known_patches::is_spare_memory(patch.offset) {
                self.spare.parts.push((patch.offset | 0x8000_0000, value.to_vec()));
                return;
            }
        }
        if !patch.search && !patch.ocarina && patch.value_file.is_empty() {
            if let Some((dome, rows)) = known_patches::comet_table(patch.offset, &patch.value) {
                self.plan.comet_schedule.insert(dome, rows);
                if !self.plan.known_patches.iter().any(|w| w == "LumaLink.Patch.CometSchedule") {
                    self.plan.known_patches.push("LumaLink.Patch.CometSchedule".to_string());
                }
                return;
            }
        }
        if patch.ocarina || !patch.value_file.is_empty() {
            let kind = if patch.ocarina { "code list" } else { "code file" };
            let value = from_file.unwrap_or_default();
            self.plan.unknown_code.push(UnknownCode { address: patch.offset, bytes: 0, kind, release: self.version, own, value, original: patch.original.clone() });
            return;
        }
        if patch.search {
            let (release, value, original) = (self.version, patch.value.clone(), patch.original.clone());
            self.plan.unknown_code.push(UnknownCode { address: patch.offset, bytes: patch.value.len(), kind: "search", release, own, value, original });
            return;
        }
        match known_patches::find(patch.offset, &patch.value) {
            Some(known) => {
                if known.id != known_patches::id::NATIVE_ALREADY {
                    self.plan.native_patches.insert(known.id, known.parameter);
                }
                if !self.plan.known_patches.iter().any(|w| w == known.what) {
                    self.plan.known_patches.push(known.what.to_string());
                }
            }
            None => {
                let (release, value, original) = (self.version, patch.value.clone(), patch.original.clone());
                self.plan.unknown_code.push(UnknownCode { address: patch.offset, bytes: patch.value.len(), kind: "memory", release, own, value, original });
            }
        }
    }

    fn said(&mut self, what: &str) {
        if !self.plan.known_patches.iter().any(|w| w == what) {
            self.plan.known_patches.push(what.to_string());
        }
    }

    /// What a mod's code in spare memory comes to. Whatever isn't understood stays on the list of code changes.
    fn read_spare_memory(&mut self) {
        let mut left: Vec<(u32, usize)> = self.spare.parts.iter().map(|(at, bytes)| (*at, bytes.len())).collect();
        if let Some(target) = self.comet_hook.take() {
            match self.spare.comet_limits(target) {
                Some((rows, used)) => {
                    self.plan.comet_limits.extend(rows);
                    self.said("LumaLink.Patch.CometLimits");
                    left.retain(|(at, size)| !used.iter().any(|(u, length)| u >= at && *u < at + *size as u32 && *length > 0));
                }
                None => self.plan.unknown_code.push(UnknownCode { address: target, bytes: 4, kind: "memory", release: self.version, own: target, ..Default::default() }),
            }
        }
        for (at, size) in left {
            let value = self.spare.parts.iter().find(|(start, bytes)| *start == at && bytes.len() == size).map(|(_, b)| b.clone()).unwrap_or_default();
            self.plan.unknown_code.push(UnknownCode { address: at, bytes: size, kind: "memory", release: self.version, own: at, value, original: Vec::new() });
        }
    }

    /// What one choice of one option does, on its own.
    fn fragment(&self, disc: &Disc, section: usize, option: usize, choice: usize, file: &PatchFile, game_id: &str) -> Plan {
        let mut alone = disc.clone();
        for each in alone.sections.iter_mut().flat_map(|s| &mut s.options) {
            each.selected = 0;
        }
        alone.sections[section].options[option].selected = choice + 1;
        let mut part = Builder { plan: Plan::default(), store: self.store, spare: known_patches::SpareMemory::default(), comet_hook: None, version: 'E' };
        for patch in alone.active_patches(game_id) {
            let patch_root = if patch.root.is_empty() {
                file.xml.parent().unwrap_or(&file.sd_root).to_path_buf()
            } else {
                resolve(&file.sd_root, file.xml.parent().unwrap_or(&file.sd_root), &patch.root).unwrap_or_else(|| file.sd_root.clone())
            };
            for f in &patch.files {
                part.file(&file.sd_root, &patch_root, f);
            }
            for f in &patch.folders {
                let disc_path = if f.disc.is_empty() { "/".to_string() } else { f.disc.clone() };
                part.folder(&file.sd_root, &patch_root, f, &disc_path, &f.external);
            }
            for memory in &patch.memory {
                part.memory(&file.sd_root, &patch_root, memory);
            }
            for save in &patch.saves {
                if let Some(folder) = resolve(&file.sd_root, &patch_root, &save.external) {
                    part.plan.save_folder = Some(folder);
                    part.plan.save_clone = save.clone;
                }
            }
        }
        part.read_spare_memory();
        part.plan
    }

    /// What a part says about itself goes in the plan; its files and switches go where the caller puts them.
    fn take_notes(&mut self, part: &Plan) {
        for note in &part.notes {
            if !self.plan.notes.contains(note) {
                self.plan.notes.push(note.clone());
            }
        }
        for what in &part.known_patches {
            if !self.plan.known_patches.contains(what) {
                self.plan.known_patches.push(what.clone());
            }
        }
        self.plan.unknown_code.extend(part.unknown_code.iter().cloned());
        if !part.event_flags.is_empty() {
            self.plan.event_flags.clone_from(&part.event_flags);
        }
        for (dome, rows) in &part.comet_schedule {
            self.plan.comet_schedule.insert(*dome, rows.clone());
        }
        if part.save_folder.is_some() {
            self.plan.save_folder.clone_from(&part.save_folder);
            self.plan.save_clone = part.save_clone;
        }
    }

    fn take_always(&mut self, part: Plan) {
        self.take_notes(&part);
        self.plan.files.extend(part.files);
        self.plan.native_patches.extend(part.native_patches);
        crate::player_constants::merge(&mut self.plan.player_constants, part.player_constants);
        self.plan.object_changes.extend(part.object_changes);
        self.plan.comet_limits.extend(part.comet_limits);
    }

    /// A question that only asks which version of the game this is. Nobody needs to answer it: what the versions share is
    /// always used, and what differs goes in layers for those versions. A mod's code patches differ between versions only
    /// in their addresses and Nebula's switches don't depend on the version, so the choice Nebula knows most of says them
    /// all.
    fn region_option(&mut self, disc: &Disc, section: usize, option: usize, file: &PatchFile) {
        let choices = &disc.sections[section].options[option].choices;
        let mut parts: Vec<(char, Plan)> = Vec::new();
        for (i, choice) in choices.iter().enumerate() {
            let letter = riivolution::choice_region(&choice.name).unwrap_or('E');
            let part = self.fragment(disc, section, option, i, file, &format!("RMG{letter}01"));
            parts.push((letter, part));
        }
        let best = parts.iter().max_by_key(|(_, p)| (p.known_patches.len(), std::cmp::Reverse(p.unknown_code.len()))).map(|(_, p)| p.clone());
        if let Some(best) = best {
            self.take_notes(&Plan { known_patches: best.known_patches, unknown_code: best.unknown_code, ..Default::default() });
            self.plan.native_patches.extend(best.native_patches);
            crate::player_constants::merge(&mut self.plan.player_constants, best.player_constants);
            self.plan.object_changes.extend(best.object_changes);
            self.plan.comet_limits.extend(best.comet_limits);
        }
        let same = parts.windows(2).all(|w| w[0].1.files == w[1].1.files);
        let letters: Vec<char> = parts.iter().map(|(l, _)| *l).collect();
        for (index, (letter, mut part)) in parts.into_iter().enumerate() {
            part.known_patches.clear();
            part.unknown_code.clear();
            self.take_notes(&part);
            if same {
                self.plan.files.extend(part.files);
                continue;
            }
            if part.files.is_empty() {
                continue;
            }
            // The first choice also stands for any version the mod didn't think of.
            let mut versions = vec![letter];
            if index == 0 {
                versions.extend(VERSION_LETTERS.iter().copied().filter(|l| !letters.contains(l)));
            }
            self.plan.layers.push(Layer {
                name: format!("version-{}", letter.to_ascii_lowercase()),
                when: When { version: versions, ..Default::default() },
                files: part.files,
                ..Default::default()
            });
        }
    }

    /// An option the player chooses in the game: each choice a layer.
    fn player_option(&mut self, disc: &Disc, section: usize, option: usize, file: &PatchFile, game_id: &str) {
        let entry = &disc.sections[section].options[option];
        let name = entry.name.trim_end_matches(':').trim().to_string();
        let mut id = slug(&name);
        if self.plan.options.iter().any(|o| o.id == id) {
            id = format!("{}-{id}", slug(&disc.sections[section].name));
        }
        // Each option its own id: the game keeps one choice per id, so two sharing one would move together.
        let id = unused(id, |candidate| self.plan.options.iter().any(|o| o.id == candidate));
        let mut choices = vec![OptionChoice { id: "off".to_string(), name: "Off".to_string() }];
        let mut default = "off".to_string();
        // Choices that only differ in where a console keeps its save come to the same here: Nebula keeps saves itself.
        let parts: Vec<Plan> = (0..entry.choices.len()).map(|i| self.fragment(disc, section, option, i, file, game_id)).collect();
        let the_same = parts.len() > 1
            && parts.windows(2).all(|w| {
                w[0].player_constants == w[1].player_constants && w[0].files == w[1].files && w[0].native_patches == w[1].native_patches && w[0].object_changes == w[1].object_changes && w[0].comet_limits == w[1].comet_limits
            });
        for (i, choice) in entry.choices.iter().enumerate() {
            if the_same && i != 0 {
                continue;
            }
            let mut choice_id = slug(&choice.name);
            if choice_id == "off" || choices.iter().any(|c| c.id == choice_id) {
                choice_id = format!("choice-{}", i + 1);
            }
            let mut part = parts[i].clone();
            self.take_notes(&part);
            if entry.selected == i + 1 || (the_same && entry.selected != 0) {
                default.clone_from(&choice_id);
            }
            let when = When { option: BTreeMap::from([(id.clone(), vec![choice_id.clone()])]), ..Default::default() };
            if !part.player_constants.is_empty() || !part.files.is_empty() || !part.native_patches.is_empty() || !part.object_changes.is_empty() || !part.comet_limits.is_empty() {
                self.plan.layers.push(Layer {
                    name: format!("{id}-{choice_id}"),
                    when,
                    files: std::mem::take(&mut part.files),
                    switches: std::mem::take(&mut part.native_patches),
                    object_changes: std::mem::take(&mut part.object_changes),
                    comet_limits: std::mem::take(&mut part.comet_limits),
                    player_constants: std::mem::take(&mut part.player_constants),
                });
            }
            // One choice left: "Off" and the mod's own name for being on.
            let shown = if the_same { "On".to_string() } else { choice.name.clone() };
            choices.push(OptionChoice { id: choice_id, name: shown });
        }
        self.plan.options.push(PlayerOption { id, name, default, choices });
    }

    /// A patch file for one other version of the game, beside one for the version the tables know: only the files it adds, on the same options as the main file.
    fn version_file(&mut self, file: &PatchFile, letter: char) -> Result<()> {
        let game_id = format!("RMG{letter}01");
        let bytes = fs::read(&file.xml)?;
        let Ok(mut disc) = riivolution::parse(&riivolution::decode(&bytes)) else {
            return Ok(());
        };
        let saved = layout::Layout::saved_choices(file, &game_id);
        disc.choose_defaults(&game_id, &saved);
        for section in 0..disc.sections.len() {
            for option in 0..disc.sections[section].options.len() {
                let entry = &disc.sections[section].options[option];
                let first = entry.selected.max(1) - 1;
                if entry.choices.is_empty() {
                    continue;
                }
                let mut part = self.fragment(&disc, section, option, first, file, &game_id);
                // What the main file already brings stays its own; a version's own folders are what's left.
                let main_has = |disc_path: &String| {
                    self.plan.files.contains_key(disc_path) || self.plan.layers.iter().any(|l| l.files.contains_key(disc_path))
                };
                part.files.retain(|path, _| !main_has(path));
                if part.files.is_empty() {
                    continue;
                }
                let name = entry.name.trim_end_matches(':').trim().to_string();
                // The main file's option of the same name: its id may not be the name's own when two names met.
                let mut when = When { version: vec![letter], ..Default::default() };
                if let Some(option) = self.plan.options.iter().find(|o| o.name == name) {
                    let on: Vec<String> = option.choices.iter().filter(|c| c.id != "off").map(|c| c.id.clone()).collect();
                    if !on.is_empty() {
                        when.option.insert(option.id.clone(), on);
                    }
                }
                let layer = unused(format!("version-{}-{}", letter.to_ascii_lowercase(), slug(&name)), |candidate| self.plan.layers.iter().any(|l| l.name == candidate));
                self.plan.layers.push(Layer {
                    name: layer,
                    when,
                    files: part.files,
                    ..Default::default()
                });
            }
        }
        Ok(())
    }

    fn patch_file(&mut self, file: &PatchFile, game_id: &str) -> Result<bool> {
        let name = file.xml.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let bytes = fs::read(&file.xml)?;
        let mut disc: Disc = match riivolution::parse(&riivolution::decode(&bytes)) {
            Ok(disc) => disc,
            Err(error) => {
                // Other XML beside the patches (a tool's settings) is not a patch.
                self.plan.note(Level::Info, say!("LumaLink.Plan.NotUsed", name = name, why = format!("{error:#}")));
                return Ok(false);
            }
        };
        // A mod written with one version in mind still works on the others: their files are the same.
        let wanted = disc.target.game.clone().unwrap_or_default();
        if !wanted.is_empty() && wanted.get(..3).unwrap_or(&wanted) != ANY_VERSION {
            self.plan.for_another_game.get_or_insert(wanted);
            return Ok(false);
        }
        let game_id = version_named(&disc.target, game_id);
        self.version = game_id.chars().nth(3).unwrap_or('E');
        let saved = layout::Layout::saved_choices(file, &game_id);
        disc.choose_defaults(&game_id, &saved);
        let total: usize = disc.sections.iter().map(|s| s.options.len()).sum();
        for section in 0..disc.sections.len() {
            for option in 0..disc.sections[section].options.len() {
                let entry = &disc.sections[section].options[option];
                if entry.asks_for_region() {
                    self.region_option(&disc, section, option, file);
                } else if total == 1 && entry.choices.len() == 1 {
                    // A mod's one switch is the mod itself.
                    let part = self.fragment(&disc, section, option, 0, file, &game_id);
                    self.take_always(part);
                } else {
                    self.player_option(&disc, section, option, file, &game_id);
                }
            }
        }
        Ok(true)
    }

    fn files_root(&mut self, root: &Path, top_level: bool) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        let whole = FolderPatch { resize: true, create: true, recursive: true, ..Default::default() };
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                self.plan.note(Level::Info, say!("LumaLink.Plan.Link", path = format!("/{name}")));
                continue;
            }
            if kind.is_dir() {
                if !top_level || layout::is_disc_folder(&name) {
                    self.folder(root, root, &whole, &format!("/{name}"), &format!("/{name}"));
                }
            } else if !layout::is_note(&entry.path()) {
                self.file(root, root, &FilePatch { disc: format!("/{name}"), external: format!("/{name}"), resize: true, create: true, ..Default::default() });
            }
        }
    }
}

/// Whether a game id (`SB4E01`, or its first three letters) is Super Mario Galaxy 2's.
pub fn is_galaxy_2(id: &str) -> bool {
    matches!(id.get(..3), Some("SB4") | Some("SB5"))
}

/// The plan for a mod on this disc. `store`, when given, is the player's imported game; without it some placements cannot
/// be checked and bare file names cannot be placed.
pub fn build(layout: &Layout, game_id: &str, store: Option<&GameStore>) -> Result<Plan> {
    let mut builder = Builder { plan: Plan { game_id: game_id.to_string(), ..Default::default() }, store, spare: known_patches::SpareMemory::default(), comet_hook: None, version: 'E' };
    let mut used_patch = false;
    // One file per version of the game is one mod: the file for this version says it all, the others add the files only their versions need.
    let one_version = |file: &PatchFile| -> Option<char> {
        let bytes = fs::read(&file.xml).ok()?;
        let disc = riivolution::parse(&riivolution::decode(&bytes)).ok()?;
        let wanted = disc.target.game.clone().unwrap_or_default();
        if wanted.get(..3).unwrap_or(&wanted) != ANY_VERSION || disc.target.regions.len() != 1 {
            return None;
        }
        disc.target.regions[0].chars().next()
    };
    let this_version = game_id.chars().nth(3);
    let has_this_version = layout.patches.iter().any(|f| one_version(f) == this_version);
    let mut other_versions = Vec::new();
    for file in &layout.patches {
        match one_version(file) {
            Some(letter) if has_this_version && Some(letter) != this_version => other_versions.push((file, letter)),
            _ => used_patch |= builder.patch_file(file, game_id)?,
        }
    }
    for (file, letter) in other_versions {
        builder.version_file(file, letter)?;
    }
    if !used_patch && !layout.patches.is_empty() {
        if let Some(wanted) = builder.plan.for_another_game.clone() {
            builder.plan.note(Level::Blocked, if is_galaxy_2(&wanted) { say!("LumaLink.Plan.ForGalaxy2") } else { say!("LumaLink.Plan.ForOtherGame", game = wanted) });
        }
    }
    if !builder.plan.blocked() {
        if let Some(root) = &layout.files_root {
            builder.files_root(root, root == &layout.root);
        }
        for file in &layout.loose {
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match store {
                Some(s) => {
                    let paths = s.named(&name);
                    if paths.is_empty() {
                        builder.plan.note(Level::Lost, say!("LumaLink.Plan.NoFileNamed", name = name));
                    }
                    for path in paths {
                        builder.plan.set(&path, file);
                    }
                }
                None => builder.plan.note(Level::Lost, say!("LumaLink.Plan.LooseFile", name = name)),
            }
        }
        if builder.plan.for_another_game.is_none() {
            for code in &layout.wii_code {
                let name = code.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let lower = name.to_ascii_lowercase();
                // The version a list is for: most are named by the game's id.
                builder.version = name.get(3..4).and_then(|l| l.chars().next()).filter(|l| name.get(..3).is_some_and(|g| g.eq_ignore_ascii_case("RMG")) && matches!(l, 'E' | 'P' | 'J' | 'K')).unwrap_or('E');
                if lower.ends_with(".gct") {
                    if let Ok(bytes) = fs::read(code) {
                        let list = crate::gecko::from_gct(&bytes);
                        if !list.writes.is_empty() {
                            for _ in 0..list.other {
                                builder.plan.unknown_code.push(UnknownCode { kind: "code list", ..Default::default() });
                            }
                            builder.writes(&layout.root, &layout.root, list.writes);
                            continue;
                        }
                    }
                }
                if lower == "main.dol" {
                    builder.changed_program(&layout.root, &layout.root, code);
                    continue;
                }
                if lower.ends_with(".ini") {
                    if let Ok(text) = fs::read_to_string(code) {
                        // A popular code by its name is one of Nebula's own samples: its memory writes mean nothing to the native
                        // game, so they are not read; the player is told which sample does it.
                        let mut writes = Vec::new();
                        for one in crate::gecko::ini_codes(&text).into_iter().filter(|c| c.enabled) {
                            if let Some(known) = crate::gecko::known_code(&one.name) {
                                builder.plan.note(Level::Lost, known.advice(&one.name));
                                continue;
                            }
                            let list = crate::gecko::from_pairs(&one.pairs);
                            for _ in 0..list.other {
                                builder.plan.unknown_code.push(UnknownCode { kind: "code list", ..Default::default() });
                            }
                            writes.extend(list.writes);
                        }
                        builder.writes(&layout.root, &layout.root, writes);
                    }
                    continue;
                }
                builder.plan.note(Level::Lost, say!("LumaLink.Plan.CompiledCode", name = name));
            }
        }
    }
    if let Some(store) = store {
        let adds = |file: &PlannedFile| {
            let lower = file.disc.to_ascii_lowercase();
            lower.starts_with("/stagedata/") && lower.ends_with("scenario.arc") && !store.has(&file.disc)
        };
        let mut new_galaxies = std::collections::BTreeSet::new();
        for file in builder.plan.files.values().chain(builder.plan.layers.iter().flat_map(|l| l.files.values())) {
            if adds(file) {
                // /StageData/<name>/<name>Scenario.arc
                if let Some(name) = file.disc.rsplit('/').next().and_then(|f| f.get(..f.len().saturating_sub("Scenario.arc".len()))) {
                    new_galaxies.insert(name.to_string());
                }
            }
        }
        builder.plan.adds_galaxy = !new_galaxies.is_empty();
        builder.plan.new_galaxies = new_galaxies.into_iter().collect();
        if builder.plan.adds_galaxy {
            builder.plan.note(Level::Info, say!("LumaLink.Plan.OwnSaves"));
        }
    }
    // Options all off and nothing else would be a mod that does nothing when it's turned on.
    if builder.plan.files.is_empty() && builder.plan.native_patches.is_empty() && builder.plan.options.iter().all(|o| o.default == "off") {
        if let Some(first) = builder.plan.options.first_mut() {
            if let Some(choice) = first.choices.get(1) {
                first.default.clone_from(&choice.id);
            }
        }
    }
    let count = builder.plan.unknown_code.len();
    if count != 0 {
        game_instructions(&mut builder.plan.unknown_code, store);
        builder.plan.note(Level::Lost, say!("LumaLink.Plan.CodeChanges", count = count));
    }
    if let Some(pack) = &layout.pack {
        for (game, count) in &pack.other_games {
            if !game_id.starts_with(game.get(..3).unwrap_or(game)) {
                builder.plan.note(Level::Info, say!("LumaLink.Plan.PackOtherGame", count = count, game = game));
            }
        }
    }
    // The game's own sizes: the import's record, else the disc named with --base. Without either an empty file
    // stays, and the package says it is empty.
    if let Ok(original) = crate::source::original_files() {
        builder.plan.leave_out_empty_as_in_game(&|path| original.get(path).is_some_and(|(size, _)| *size == 0));
    }
    if builder.plan.file_count() == 0 && builder.plan.native_patches.is_empty() && builder.plan.player_constants.is_empty() && builder.plan.object_changes.is_empty() && builder.plan.comet_limits.is_empty() && builder.plan.event_flags.is_empty() && layout.pack.as_ref().is_none_or(|p| p.textures == 0) && !builder.plan.blocked() {
        let other = layout.pack.as_ref().and_then(|p| p.other_games.keys().next().cloned());
        match other {
            Some(game) if is_galaxy_2(&game) => builder.plan.note(Level::Blocked, say!("LumaLink.Plan.TexturesGalaxy2")),
            Some(game) => builder.plan.note(Level::Blocked, say!("LumaLink.Plan.TexturesOtherGame", game = game)),
            None => builder.plan.note(Level::Blocked, say!("LumaLink.Plan.NotAMod")),
        }
    }
    Ok(builder.plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/modding/format/luma.md, section 4's rule for an id: 3 to 64 characters, lowercase letters, digits, `-` and `.`, starting with a
    /// letter, no `..` (the validator holds the rule; this is its statement for the tests of a converted mod's id).
    fn valid_id(id: &str) -> bool {
        (3..=64).contains(&id.len())
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
            && !id.contains("..")
    }

    #[test]
    fn a_patch_cannot_name_a_place_outside_the_game() {
        let mut plan = Plan::default();
        for bad in ["/../../../evil.bin", "/StageData/../../evil.bin", "\\\\server\\evil", "C:/evil.bin", "/a:stream", "/CON"] {
            plan.set(bad, Path::new("x"));
        }
        assert!(plan.files.is_empty(), "{:?}", plan.files.keys().collect::<Vec<_>>());
        plan.set("/StageData/Good.arc", Path::new("x"));
        assert_eq!(plan.files.len(), 1);
    }

    #[test]
    fn two_paths_of_one_game_file_keep_the_one_the_game_reads() {
        let mut plan = Plan::default();
        plan.set("/StageData/CubeBubbleExLv2Galaxy/KoopaBattleVs3BossZone.arc", Path::new("a"));
        plan.set("/StageData/KoopaBattleVs3BossZone.arc", Path::new("b"));
        plan.set("/StageData/Other/KoopaBattleVs3BossZone.arc", Path::new("c"));
        assert_eq!(plan.files.len(), 1);
        let only = plan.files.values().next().unwrap();
        assert_eq!((only.disc.as_str(), only.host.as_path()), ("/StageData/KoopaBattleVs3BossZone.arc", Path::new("b")));
        assert_eq!(plan.notes.len(), 2, "{:?}", plan.notes);
    }

    #[test]
    fn console_files_are_left_out_as_nothing_lost() {
        let mut plan = Plan::default();
        plan.set("/UsEnglish/LayoutData/HomeButton.arc", Path::new("a"));
        plan.set("/opening.bnr", Path::new("b"));
        assert!(plan.files.is_empty());
        assert!(plan.notes.iter().all(|n| n.level == Level::Info), "{:?}", plan.notes);
    }

    #[test]
    fn any_name_makes_an_id_the_game_reads() {
        for name in ["Aux", "CON", "com1", "!!!", "", "A Very Long Name That Goes On And On And On And On And On And On And On And On", "Kaizo: Rebalanced!"] {
            assert!(valid_id(&slug(name)), "{name:?} -> {}", slug(name));
            assert!(valid_id(&mod_id(name)), "{name:?} -> {}", mod_id(name));
        }
    }

    #[test]
    fn names_in_other_letters_are_different_mods() {
        let ids: Vec<String> = ["スーパーマリオ", "マリオギャラクシー", "Café Galaxy", "Cafe Galaxy", "Ñandú ".repeat(20).as_str()].iter().map(|n| mod_id(n)).collect();
        for id in &ids {
            assert!(valid_id(id), "{id}");
        }
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[2], ids[3]);
        assert_eq!(ids[3], "cafe-galaxy");
        assert!(ids[0].starts_with("mod-") && ids[2].starts_with("caf-galaxy-"), "{ids:?}");
    }

    /// A mod folder with a link to the folder above it: read once, the link left out, never followed round.
    #[test]
    fn a_link_in_a_mod_folder_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mod");
        write(&source, "StageData/a.arc", "x");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&source, source.join("StageData/loop"));
        // A junction: the link any Windows user can make.
        #[cfg(windows)]
        let linked = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(source.join("StageData").join("loop"))
            .arg(&source)
            .output()
            .map_err(|e| e.to_string())
            .and_then(|out| if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).to_string()) });
        if linked.is_err() {
            return; // this system lets no one make links
        }
        assert!(source.join("StageData/loop/StageData/a.arc").is_file());
        let plan = build(&Layout::of(&source), "RMGE01", None).unwrap();
        assert_eq!(plan.files.len(), 1, "{:?}", plan.files.keys());
        assert!(plan.notes.iter().any(|n| n.text.contains("a link, left out")), "{:?}", plan.notes);
    }

    #[test]
    fn options_never_share_an_id() {
        assert_eq!(unused("a".to_string(), |_| false), "a");
        assert_eq!(unused("a".to_string(), |c| c == "a" || c == "a-2"), "a-3");
    }

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    const XML: &str = r#"<wiidisc version="1"><id game="RMG"/><options><section name="M"><option name="Region">
        <choice name="NTSC"><patch id="p"/></choice><choice name="PAL"><patch id="q"/></choice></option></section></options>
        <patch id="p"><folder disc="/StageData" external="/M/stage" create="true"/>
          <file disc="Kuribo.arc" external="/M/k.arc"/>
          <file disc="/ObjectData/New.arc" external="/M/k.arc"/>
          <file disc="/ObjectData/Part.arc" external="/M/k.arc" offset="16"/>
          <file disc="/ObjectData/Gone.arc" external="/M/none.arc" create="true"/>
          <memory offset="0x8039AA2C" value="60000000"/>
          <memory offset="0x8007679C" value="60000000"/>
          <memory offset="0x80000000" value="00000001"/>
          <memory valuefile="code.gct" ocarina="true"/>
          <savegame external="/save"/>
        </patch><patch id="q"><file disc="/PalOnly" external="/M/k.arc" create="true"/></patch></wiidisc>"#;

    fn store() -> GameStore {
        GameStore::from_paths("RMGE01", &["ObjectData", "ObjectData/Kuribo.arc", "StageData", "UsEnglish"])
    }

    #[test]
    fn a_popular_code_is_said_by_what_it_does() {
        let dir = tempfile::tempdir().unwrap();
        let ini = ["[Gecko]", "$Moon Jump [someone]", "C0000000 00000003", "$Hide HUD", "04001000 00000001", "[Gecko_Enabled]", "$Moon Jump"].join("
");
        write(dir.path(), "RMGE01.ini", &ini);
        let store = store();
        let plan = build(&Layout::of(dir.path()), "RMGE01", Some(&store)).unwrap();
        let text: Vec<_> = plan.notes.iter().map(|n| n.text.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("Moon Jump") && t.contains("rise while jump is held") && t.contains("code/")), "{text:?}");
        assert!(!text.iter().any(|t| t.contains("HUD hidden")), "a code that is off says nothing: {text:?}");
        assert!(plan.unknown_code.is_empty(), "{:?}", plan.unknown_code);
    }

    #[test]
    fn an_empty_file_the_game_has_empty_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.aw", "");
        write(dir.path(), "b.aw", "");
        write(dir.path(), "c.aw", "x");
        let mut plan = Plan::default();
        for name in ["a.aw", "b.aw", "c.aw"] {
            plan.set(&format!("/AudioRes/Waves/{name}"), &dir.path().join(name));
        }
        // a.aw and c.aw are empty in the game; b.aw is not, so an empty one is the mod's change (and a loss later).
        plan.leave_out_empty_as_in_game(&|path| path == "audiores/waves/a.aw" || path == "audiores/waves/c.aw");
        let left: Vec<_> = plan.files.values().map(|f| f.disc.as_str()).collect();
        assert_eq!(left, ["/AudioRes/Waves/b.aw", "/AudioRes/Waves/c.aw"]);
        assert!(plan.notes.iter().any(|n| n.level == Level::Info && n.text.starts_with("/AudioRes/Waves/a.aw: empty")), "{:?}", plan.notes);
    }

    #[test]
    fn a_patch_file_places_everything_or_says_why() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "riivolution/m.xml", XML);
        write(dir.path(), "M/stage/a.arc", "x");
        write(dir.path(), "M/stage/Sub/b.arc", "x");
        write(dir.path(), "M/k.arc", "x");
        write(dir.path(), "save/keep.txt", "x");
        let store = store();
        let plan = build(&Layout::of(dir.path()), "RMGE01", Some(&store)).unwrap();
        // NTSC and PAL set different files: a layer for each, and no question.
        assert!(plan.options.is_empty());
        assert_eq!(plan.layers.len(), 2);
        let ntsc = &plan.layers[0];
        assert!(ntsc.when.version.contains(&'E') && ntsc.when.version.contains(&'J'), "{:?}", ntsc.when);
        let mut files: Vec<_> = ntsc.files.values().map(|f| f.disc.as_str()).collect();
        files.sort();
        assert_eq!(files, ["/ObjectData/Kuribo.arc", "/StageData/Sub/b.arc", "/StageData/a.arc"]);
        assert_eq!(plan.layers[1].when.version, ['P']);
        let text: Vec<_> = plan.notes.iter().map(|n| n.text.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("/ObjectData/New.arc") && t.contains("does not create")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("patched in part")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("/M/none.arc")), "{text:?}");
        assert_eq!(plan.native_patches[&known_patches::id::SKELETAL_FISH_BABY_NO_BUBBLE], 1);
        assert_eq!(plan.known_patches.len(), 2);
        assert_eq!(plan.unknown_code.len(), 2);
        // Each write keeps what it writes and where the patch says, for the list of code changes.
        for u in plan.unknown_code.iter().filter(|u| u.kind == "memory") {
            assert!(u.value.len() == u.bytes && u.bytes != 0 && u.release != '\0' && u.own != 0, "{u:?}");
        }
        assert!(plan.save_folder.is_some());
    }

    const OPTIONS: &str = r#"<wiidisc version="1"><id game="RMG"/><options><section name="M">
        <option name="Lives:"><choice name="Normal"><patch id="a"/></choice><choice name="One health"><patch id="b"/></choice></option>
        <option name="Music"><choice name="New"><patch id="c"/></choice></option></section></options>
        <patch id="a"><file disc="/StageData/A.arc" external="/M/a.arc" create="true"/></patch>
        <patch id="b"><file disc="/StageData/A.arc" external="/M/b.arc" create="true"/></patch>
        <patch id="c"><file disc="/AudioRes/C.arc" external="/M/a.arc" create="true"/></patch></wiidisc>"#;

    #[test]
    fn options_become_the_players_choices() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "riivolution/m.xml", OPTIONS);
        write(dir.path(), "M/a.arc", "a");
        write(dir.path(), "M/b.arc", "b");
        let plan = build(&Layout::of(dir.path()), "RMGP01", None).unwrap();
        assert_eq!(plan.options.len(), 2);
        let lives = &plan.options[0];
        assert_eq!(lives.id, "lives");
        assert_eq!(lives.name, "Lives");
        // Everything off would be a mod that does nothing.
        assert_eq!(lives.default, "normal");
        assert_eq!(lives.choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["off", "normal", "one-health"]);
        assert_eq!(plan.layers.len(), 3);
        assert_eq!(plan.layers[1].name, "lives-one-health");
        assert_eq!(plan.layers[1].when.option["lives"], ["one-health"]);
        assert_eq!(plan.file_count(), 2);
    }

    #[test]
    fn versions_with_the_same_files_need_no_layers() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "riivolution/m.xml", r#"<wiidisc version="1"><id game="RMG"/><options><section name="M"><option name="Region">
            <choice name="NTSC"><patch id="a"/></choice><choice name="PAL"><patch id="a"/></choice><choice name="NTSC-J"><patch id="a"/></choice></option></section></options>
            <patch id="a"><file disc="/StageData/A.arc" external="/M/a.arc" create="true"/></patch></wiidisc>"#);
        write(dir.path(), "M/a.arc", "a");
        let plan = build(&Layout::of(dir.path()), "RMGJ01", None).unwrap();
        assert!(plan.layers.is_empty());
        assert!(plan.options.is_empty());
        assert_eq!(plan.files.len(), 1);
    }

    #[test]
    fn a_mod_for_galaxy_2_is_named_as_that() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "riivolution/m.xml", r#"<wiidisc version="1"><id game="SB4"/></wiidisc>"#);
        write(dir.path(), "game/CustomCode/CustomCode_SB4E.bin", "x");
        let plan = build(&Layout::of(dir.path()), "RMGE01", None).unwrap();
        assert!(plan.blocked());
        assert!(plan.notes.iter().any(|n| n.text.contains("Super Mario Galaxy 2")), "{:?}", plan.notes);
    }

    #[test]
    fn names_and_versions_come_from_file_names() {
        assert_eq!(nice_name("KaizoMarioGalaxyRebalanced V2.1"), "Kaizo Mario Galaxy Rebalanced V2.1");
        assert_eq!(nice_name("super_mario_galaxy_the_green_stars_"), "Super Mario Galaxy The Green Stars");
        assert_eq!(nice_name("nmg_v1.6.0"), "Nmg v1.6.0");
        assert_eq!(split_version("Kaizo Mario Galaxy Rebalanced V2.1"), ("Kaizo Mario Galaxy Rebalanced".to_string(), Some("2.1".to_string())));
        assert_eq!(split_version("Nmg v1.6.0"), ("Nmg".to_string(), Some("1.6.0".to_string())));
        assert_eq!(split_version("Galaxy 2"), ("Galaxy 2".to_string(), None));
        assert_eq!(split_version("Green Stars"), ("Green Stars".to_string(), None));
    }

    #[test]
    fn folders_in_any_case_and_one_file_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "ObjectData/A.arc", "first");
        write(dir.path(), "objectdata2/a.arc", "x");
        write(dir.path(), "readme.txt", "note");
        write(dir.path(), "preview.png", "note");
        let plan = build(&Layout::of(dir.path()), "RMGE01", None).unwrap();
        assert_eq!(plan.files.len(), 1);
    }

    #[test]
    fn a_mod_that_adds_a_galaxy_is_noticed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "StageData/MyGalaxy/MyGalaxyScenario.arc", "x");
        let store = store();
        let plan = build(&Layout::of(dir.path()), "RMGE01", Some(&store)).unwrap();
        assert!(plan.adds_galaxy);
        assert!(plan.notes.iter().any(|n| n.text.contains("own saves")), "{:?}", plan.notes);
    }

    #[test]
    fn a_mod_for_one_version_converts_for_every_version() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "riivolution/m.xml", r#"<wiidisc version="1"><id game="RMGP"/><options><section name="M"><option name="On"><choice name="Yes"><patch id="p"/></choice></option></section></options><patch id="p"><folder disc="/StageData" external="/M/stage" create="true"/></patch></wiidisc>"#);
        write(dir.path(), "M/stage/a.arc", "x");
        let plan = build(&Layout::of(dir.path()), "RMGE01", None).unwrap();
        assert!(!plan.blocked(), "{:?}", plan.notes);
        assert_eq!(plan.files.len(), 1);
    }

    #[test]
    fn compiled_code_is_named_and_the_rest_applies() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "ObjectData/A.arc", "x");
        write(dir.path(), "CustomCode/CustomCode_USA.bin", "x");
        let plan = build(&Layout::of(dir.path()), "RMGE01", None).unwrap();
        assert_eq!(plan.files.len(), 1);
        assert!(plan.notes.iter().any(|n| n.level == Level::Lost && n.text.contains("CustomCode_USA.bin")), "{:?}", plan.notes);
    }

    #[test]
    fn a_pack_for_galaxy_2_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "SB4/Neo/LayoutData/Buttons/a.png", "x");
        write(dir.path(), "readme.txt", "x");
        let plan = build(&Layout::of(dir.path()), "RMGE01", None).unwrap();
        assert!(plan.blocked());
        assert!(plan.notes.iter().any(|n| n.text.contains("Super Mario Galaxy 2")), "{:?}", plan.notes);
    }

    #[test]
    fn a_path_cannot_leave_the_mod() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve(dir.path(), dir.path(), "../outside").is_none());
        assert!(resolve(dir.path(), dir.path(), "C:/outside").is_none());
    }
}
