//! LumaLink's jobs, for its command line and its window alike: each takes paths and inputs and returns what
//! came of it as data, its words in the chosen language. Neither front end has logic of its own: the command line
//! prints what a job returns, the window shows it.

use crate::installed::Installed;
use crate::{character, luma_file, say, text};
use anyhow::{bail, Context, Result};
use luma_validate::names::Names;
use luma_validate::{Finding, Level, Report};
use nebula_import::store_view::PakStore;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The Nebula release this LumaLink checks against: the one its validator and cooker come from (`nebula.commit`).
pub const NEBULA: &str = "2.0.0";

/// What the validator is given beyond the mod itself. Each input left out keeps its rules from running, and each such
/// rule is named in the report (docs/modding/format/validator.md, section 2).
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// The game's self-description folder (`versions/<v>/luma`): function rows, types, placement facts.
    pub names: Option<PathBuf>,
    /// The platforms to check the code for; all nine when empty.
    pub platforms: Vec<String>,
    /// The imported game (Nebula's game.pak), for the rules on layer files and patches.
    pub game: Option<PathBuf>,
    /// Leave the installed game out: what `game` and `names` do not name is not given.
    pub no_installed: bool,
    /// LumaLink's mod folder: the mods (folders and .luma files) a mod's `needs` are found in (docs/modding/format/validator.md,
    /// "the other mods"). None: the needs are not checked, and the report says so.
    pub mods: Option<PathBuf>,
}

impl Inputs {
    /// No game at all: only the rules that need none run.
    pub fn none() -> Self {
        Self { names: None, platforms: vec![], game: None, no_installed: true, mods: None }
    }

    /// These inputs with the installed game's game.pak and self-description folder in place of the ones not named.
    pub fn resolved(&self, installed: Option<&Installed>) -> Inputs {
        let mut out = self.clone();
        if self.no_installed || (self.game.is_some() && self.names.is_some()) {
            return out;
        }
        let Some(found) = installed else { return out };
        if out.game.is_none() {
            out.game = Some(found.pak.clone());
        }
        if out.names.is_none() {
            out.names = found.self_description();
        }
        out
    }
}

/// The mods of LumaLink's mod folder as the validator takes the other installed mods: each folder with a luma.json and
/// each .luma file in it, on, by what its own check says it is (a mod whose luma.json does not read is left out, and
/// its needers find it missing), with what that check found it touches, as the game takes them from each mod's kept
/// report (the files and cells it changes, the data schemas a library gives). The mod being checked is left out of its
/// own list.
pub fn mod_folder(folder: &Path, checked: &Path) -> Result<Vec<luma_validate::Installed>> {
    let entries = std::fs::read_dir(folder).with_context(|| say!("LumaLink.CannotRead", path = folder.display()))?;
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    let same = |p: &Path| std::fs::canonicalize(p).ok().is_some_and(|a| std::fs::canonicalize(checked).ok().is_some_and(|b| a == b));
    let mut out = Vec::new();
    for p in paths {
        let source = if p.is_dir() && p.join("luma.json").is_file() {
            luma_validate::Source::Folder(&p)
        } else if p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("luma")) {
            luma_validate::Source::File(&p)
        } else {
            continue;
        };
        if same(&p) {
            continue;
        }
        let report = luma_validate::validate(&luma_validate::Input { source, game: None, installed: &[], platforms: &[], options: None, position: None });
        if let Some(a) = report.about {
            let file = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(luma_validate::Installed { id: a.id, version: a.version, on: true, file, conflicts: a.conflicts, needs: a.needs, touches: report.touches });
        }
    }
    Ok(out)
}

/// Why the folder of the game's function rows and types file does not read, in the chosen language.
pub fn rows_error(folder: &Path, e: luma_validate::names::RowsError) -> anyhow::Error {
    use luma_validate::names::RowsError;
    let rows = || say!("LumaLink.RowsDoNotRead", folder = folder.display());
    match e {
        RowsError::Read { path, error } => anyhow::Error::new(error).context(say!("LumaLink.CannotRead", path = path.display())).context(rows()),
        RowsError::Line { path, line } => anyhow::anyhow!(say!("LumaLink.Rows.BadLine", path = path.display(), line = line)).context(rows()),
        RowsError::Types { path, error } => anyhow::anyhow!(say!("LumaLink.Rows.BadTypes", path = path.display(), why = error)),
    }
}

/// The validator on a folder or a .luma with exactly the inputs given (resolve them first for the installed game's),
/// then the mod's effects and its source files at game paths cooked as the game's start cooks them.
pub fn validate(path: &Path, inputs: &Inputs) -> Result<Report> {
    validate_as(path, path, inputs)
}

/// `validate` on `path` standing for the mod at `mod_path`: the written .luma of a folder is that folder's mod, so the
/// folder, when it sits in the mod folder, is not another installed mod.
fn validate_as(path: &Path, mod_path: &Path, inputs: &Inputs) -> Result<Report> {
    for p in &inputs.platforms {
        if !luma_validate::PLATFORMS.contains(&p.as_str()) {
            bail!(say!("LumaLink.UnknownPlatform", value = p, list = luma_validate::PLATFORMS.join(", ")));
        }
    }
    let names = match &inputs.names {
        Some(folder) => Some(Names::from_rows_folder(folder, 1).map_err(|e| rows_error(folder, e))?),
        None => None,
    };
    let store = match &inputs.game {
        Some(pak) => {
            // Its header first, in the player's language: a store that is not one, or of another version, by name.
            let mut file = std::fs::File::open(pak).with_context(|| say!("LumaLink.CannotRead", path = pak.display()))?;
            crate::game_store::header(&mut file, pak)?;
            Some(PakStore::open(pak).with_context(|| say!("LumaLink.Store.Not", path = pak.display()))?)
        }
        None => None,
    };
    let game = (names.is_some() || store.is_some()).then(|| luma_validate::Game {
        nebula: luma_validate::semver::Version::parse(NEBULA).expect("a version"),
        // The game the mod is checked against: its program, when the store says it.
        region: store.as_ref().map(PakStore::game_id),
        extra_languages: &[],
        // LumaLink checks for every device: no device's features narrow it.
        device_features: None,
        names: names.as_ref(),
        store: store.as_ref().map(|s| s as &dyn luma_validate::Store),
    });
    let platforms: Vec<&str> = if inputs.platforms.is_empty() { luma_validate::PLATFORMS.to_vec() } else { inputs.platforms.iter().map(String::as_str).collect() };
    let source = if path.is_dir() {
        luma_validate::Source::Folder(path)
    } else if path.is_file() {
        luma_validate::Source::File(path)
    } else {
        bail!(say!("LumaLink.NotFolderOrFile", path = path.display()));
    };
    let others = match &inputs.mods {
        Some(folder) => mod_folder(folder, mod_path)?,
        None => Vec::new(),
    };
    let mut report = luma_validate::validate(&luma_validate::Input { source, game: game.as_ref(), installed: &others, platforms: &platforms, options: None, position: None });
    if inputs.mods.is_none() {
        // With no mod folder a need cannot be found: L800's "missing" is not checked, not an error of the mod.
        let missing = |f: &Finding| f.code == 800 && f.args.iter().any(|(name, value)| *name == "found" && matches!(value, luma_validate::Arg::Phrase(p) if *p == "Need.Missing"));
        let places = report.findings.iter().filter(|f| missing(f)).count() as u32;
        if places > 0 {
            report.findings.retain(|f| !missing(f));
            report.not_checked.push(luma_validate::NotChecked { code: 800, because: "Missing.ModFolder", places });
            report.not_checked.sort();
        }
    }
    // The mod's effects, cooked as the game's start check cooks them (docs/modding/content/effects.md, section 7).
    let id = report.about.as_ref().map(|a| a.id.clone());
    match crate::effects_check::findings(path, inputs.game.as_deref(), id.as_deref()) {
        Ok(found) => report.findings.extend(found),
        // A mod the validator already refuses (a broken .luma) is said by its findings.
        Err(_) if !report.passed() => {}
        Err(error) => return Err(error),
    }
    // The mod's source files at game paths, cooked as the game's start cooks them (L304, L804). A stage row's
    // arguments by name are read from the game's placement facts in the self-description folder (docs/modding/content/stages.md, section 1).
    if let Some(folder) = &inputs.names {
        nebula_import::cook::placement_args::use_file(folder);
    }
    match crate::files_check::findings(path, inputs.game.as_deref(), id.as_deref()) {
        Ok(found) => report.findings.extend(found),
        Err(_) if !report.passed() => {}
        Err(error) => return Err(error),
    }
    // Without the game the files are not cooked: said as not checked, one place for each file of files/ and assets/
    // (the game always cooks them at its start).
    let places = u32::try_from(report.touches.replaces.len() + report.touches.assets.len()).unwrap_or(u32::MAX);
    if inputs.game.is_none() && places > 0 {
        report.not_checked.push(luma_validate::NotChecked { code: 804, because: "Missing.Store", places });
        report.not_checked.sort();
    }
    Ok(report)
}

/// A finding's file with its line and column, as editors read them (`file:line:column`).
pub fn place(f: &Finding) -> String {
    match (f.line, f.column) {
        (Some(l), Some(c)) => format!("{}:{l}:{c}", f.file),
        (Some(l), None) => format!("{}:{l}", f.file),
        _ => f.file.clone(),
    }
}

/// A finding's level in words.
pub fn level(f: &Finding) -> String {
    if f.level == Level::Error { say!("LumaLink.Level.Error") } else { say!("LumaLink.Level.Note") }
}

/// A finding as one line: its level and code, then the validator's message, which names the mod and the file itself
/// (its template, as the Mods screen shows it). A line and column the template does not show follow it.
pub fn finding_line(f: &Finding) -> String {
    let message = luma_validate::message(f, text::language());
    let template = luma_validate::phrase(&format!("L{}", f.code), "UsEnglish");
    if f.line.is_some() && !template.contains("{line}") {
        say!("LumaLink.FindingAt", level = level(f), code = f.code, message = message, place = place(f))
    } else {
        say!("LumaLink.Finding", level = level(f), code = f.code, message = message)
    }
}

/// A whole report as lines: each finding, each rule not checked with why and where, then the count.
pub fn report_lines(report: &Report) -> Vec<String> {
    let mut lines: Vec<String> = report.findings.iter().map(finding_line).collect();
    for n in &report.not_checked {
        let because = luma_validate::phrase(n.because, text::language());
        let message = say!("LumaLink.Places", because = because, places = n.places);
        lines.push(say!("LumaLink.Finding", level = say!("LumaLink.Level.NotChecked"), code = n.code, message = message));
    }
    lines.push(summary_line(report));
    lines
}

/// The report's count line.
pub fn summary_line(report: &Report) -> String {
    let notes = report.findings.len() - report.errors();
    say!("LumaLink.Summary", errors = report.errors(), notes = notes, unchecked = report.not_checked.len())
}

/// The report for tools: codes, levels and places as data (the same in every language), the messages in the one
/// chosen.
pub fn report_json(report: &Report) -> serde_json::Value {
    let language = text::language();
    let findings: Vec<serde_json::Value> = report
        .findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "code": format!("L{}", f.code),
                "level": if f.level == Level::Error { "error" } else { "note" },
                "mod": f.mod_id, "file": f.file, "line": f.line, "column": f.column, "pointer": f.pointer,
                "message": luma_validate::message(f, language),
            })
        })
        .collect();
    let not_checked: Vec<serde_json::Value> = report
        .not_checked
        .iter()
        .map(|n| serde_json::json!({ "code": format!("L{}", n.code), "because": luma_validate::phrase(n.because, language), "places": n.places }))
        .collect();
    serde_json::json!({
        "validator": format!("{}.{}", report.validator.0, report.validator.1),
        "passed": report.passed(), "findings": findings, "not_checked": not_checked,
    })
}

/// What came of `new`: the folder made, or the validator's refusal of the id or name given (nothing written).
pub enum NewMod {
    Made(PathBuf),
    Refused(Report),
}

/// A new mod folder: its luma.json, checked before anything is written, and an empty base layer.
pub fn new_mod(folder: &Path, id: &str, name: &str, summary: &str, authors: &[String]) -> Result<NewMod> {
    if folder.exists() {
        bail!(say!("LumaLink.AlreadyThere", path = folder.display()));
    }
    // In the order of docs/modding/format/luma.md, section 4, so the file reads as the page shows it.
    let fields: [(&str, serde_json::Value); 7] = [
        ("format", format!("{}.{}", luma_validate::FORMAT.0, luma_validate::FORMAT.1).into()),
        ("id", id.into()),
        ("version", "0.1.0".into()),
        ("name", name.into()),
        ("summary", summary.into()),
        ("authors", authors.into()),
        ("game", serde_json::json!({ "id": "RMG", "regions": luma_validate::REGIONS })),
    ];
    let lines: Vec<String> = fields.iter().map(|(k, v)| format!("  \"{k}\": {}", serde_json::to_string(v).expect("JSON"))).collect();
    let text = format!("{{\n{}\n}}\n", lines.join(",\n"));
    // Checked before anything is written: a wrong id or name is said now, not at pack.
    let staging = tempfile::tempdir_in(parent_of(folder))?;
    std::fs::write(staging.path().join("luma.json"), &text)?;
    let report = validate(staging.path(), &Inputs::none())?;
    if !report.passed() {
        return Ok(NewMod::Refused(report));
    }
    std::fs::create_dir_all(folder.join("layers/base/files")).with_context(|| say!("LumaLink.CannotMake", path = folder.display()))?;
    let json = folder.join("luma.json");
    std::fs::write(&json, text).with_context(|| say!("LumaLink.CannotWrite", path = json.display()))?;
    Ok(NewMod::Made(folder.to_path_buf()))
}

/// The folder a path is in, `.` for a bare name.
fn parent_of(path: &Path) -> &Path {
    path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// What came of `pack`, after the folder's report.
pub enum PackOutcome {
    Packed { file: PathBuf, summary: luma_file::WriteSummary },
    /// The report has errors.
    Errors,
    /// Some rule could not be checked: pack refuses (docs/modding/format/validator.md, section 2).
    Unchecked,
    /// The written file did not pass as the folder did; it was removed. Its report.
    WrittenFails(Report),
}

/// A folder written as a .luma once the validator passes it with every rule checked, the written file checked again.
/// `output` defaults to `<id>-<version>.luma` beside the folder. Returns the folder's report and what came of it.
pub fn pack(folder: &Path, output: Option<PathBuf>, inputs: &Inputs) -> Result<(Report, PackOutcome)> {
    if !folder.is_dir() {
        bail!(say!("LumaLink.NotFolder", path = folder.display()));
    }
    let report = validate(folder, inputs)?;
    if !report.passed() {
        return Ok((report, PackOutcome::Errors));
    }
    // A rule only the player's game can check (a class a mod's code makes, the player's device) is said and packed;
    // one an input of LumaLink's would check refuses.
    if report.not_checked.iter().any(luma_validate::NotChecked::needs_input) {
        return Ok((report, PackOutcome::Unchecked));
    }
    let about = report.about.as_ref().with_context(|| say!("LumaLink.NoAbout"))?;
    let output = output.unwrap_or_else(|| folder.parent().unwrap_or(Path::new(".")).join(format!("{}-{}.luma", about.id, about.version)));
    if output.exists() {
        bail!(say!("LumaLink.AlreadyThere", path = output.display()));
    }
    let files = luma_file::folder_files(folder)?;
    let temporary = output.with_extension("luma.part");
    let summary = luma_file::write(&files, &temporary)?;
    // The written file is checked again, whole: what is published is what passed.
    let again = validate_as(&temporary, folder, inputs)?;
    if !again.passed() || again.not_checked.iter().any(luma_validate::NotChecked::needs_input) {
        let _ = std::fs::remove_file(&temporary);
        return Ok((report, PackOutcome::WrittenFails(again)));
    }
    std::fs::rename(&temporary, &output).with_context(|| say!("LumaLink.CannotWrite", path = output.display()))?;
    Ok((report, PackOutcome::Packed { file: output, summary }))
}

/// What came of putting a mod into the installed game's mods.
pub enum Install {
    /// The game's mod tool took it.
    Imported { luma: PathBuf },
    /// The game's mod tool refused it, with its words.
    NotImported { luma: PathBuf, why: String },
    /// A folder that does not pack: its report and what came of packing.
    NotPacked(Report, PackOutcome),
    /// A .luma the validator refuses.
    Refused(Report),
}

/// The mod at `path` (a folder, packed first as `pack` packs it, or a .luma) imported into the installed
/// game's mods by the game's own mod tool (`--import-mod`, as the launcher's Mods page does). `inputs` resolved;
/// `game_args` the arguments `start` will give the game (their folder options are the import's too).
pub fn install(path: &Path, inputs: &Inputs, found: &Installed, game_args: &[OsString]) -> Result<Install> {
    let program = found.program().with_context(|| say!("LumaLink.Run.NoProgram", folder = found.version.as_ref().unwrap_or(&found.data).display()))?;
    let packed = tempfile::tempdir()?;
    let luma = if path.is_dir() {
        // The game imports a .luma by the id inside it; the file's own name is not read.
        let output = packed.path().join("mod.luma");
        let (report, outcome) = pack(path, Some(output.clone()), inputs)?;
        if !matches!(outcome, PackOutcome::Packed { .. }) {
            return Ok(Install::NotPacked(report, outcome));
        }
        output
    } else if path.is_file() {
        let report = validate(path, inputs)?;
        if !report.passed() {
            return Ok(Install::Refused(report));
        }
        path.to_path_buf()
    } else {
        bail!(say!("LumaLink.NotFolderOrFile", path = path.display()));
    };
    let pak = inputs.game.clone().unwrap_or_else(|| found.pak.clone());
    // The game's mod tool prints one JSON line: {"ok": ..., "message": ...}. Its mods are those of the data folder
    // found (or the folders `game_args` name), the ones the game `start` starts reads.
    let import = std::process::Command::new(&program)
        .arg("--import-mod")
        .arg(&luma)
        .arg("--pak")
        .arg(&pak)
        .args(folder_args(found, game_args))
        .output()
        .with_context(|| say!("LumaLink.Run.CannotStart", program = program.display()))?;
    let said = String::from_utf8_lossy(&import.stdout).trim().to_string();
    // The temporary .luma is gone once this returns: a folder's mod is named by its folder.
    let shown = if path.is_dir() { path.to_path_buf() } else { luma };
    if !import.status.success() {
        let why = serde_json::from_str::<serde_json::Value>(&said).ok().and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string)).unwrap_or(said);
        return Ok(Install::NotImported { luma: shown, why });
    }
    Ok(Install::Imported { luma: shown })
}

/// The installed game started on `pak` with its own data folder and saves, then `game_args` (its command line,
/// docs/COMMAND_LINE.md; a --data or --saves there is used instead of the found one). It runs on after LumaLink ends.
/// A LumaLink started hidden (a test's) starts it hidden, as the harness does. The program started.
pub fn start(found: &Installed, pak: &Path, game_args: &[OsString]) -> Result<PathBuf> {
    let program = found.program().with_context(|| say!("LumaLink.Run.NoProgram", folder = found.version.as_ref().unwrap_or(&found.data).display()))?;
    let named = |option: &str| game_args.iter().any(|a| a == option);
    let mut args: Vec<OsString> = vec!["--pak".into(), pak.as_os_str().to_owned()];
    if !named("--saves") {
        args.extend(["--saves".into(), found.saves().into_os_string()]);
    }
    if !named("--data") {
        args.extend(["--data".into(), found.data.clone().into_os_string()]);
    }
    args.extend(game_args.iter().cloned());
    spawn_like_this(&program, &args).with_context(|| say!("LumaLink.Run.CannotStart", program = program.display()))?;
    Ok(program)
}

/// The game's options that name its folders (docs/COMMAND_LINE.md), each with its value.
const FOLDER_OPTIONS: [&str; 3] = ["--data", "--settings", "--crash-dumps"];

/// The folder options of `game_args`, and `--data` the found data folder when they name none: given to the game's mod
/// tool, the mod goes into the mods of the game `start` starts with the same arguments.
fn folder_args(found: &Installed, game_args: &[OsString]) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < game_args.len() {
        if game_args[i].to_str().is_some_and(|a| FOLDER_OPTIONS.contains(&a)) {
            out.extend([game_args[i].clone(), game_args[i + 1].clone()]);
            i += 2;
        } else {
            i += 1;
        }
    }
    if !out.iter().step_by(2).any(|a| a == "--data") {
        out.extend(["--data".into(), found.data.clone().into_os_string()]);
    }
    out
}

/// `program` with `args`, detached; hidden when this process was started hidden (Windows: its STARTUPINFO's SW_HIDE,
/// as a test starts it), so a test of `run` shows no window and plays no sound.
fn spawn_like_this(program: &Path, args: &[OsString]) -> std::io::Result<()> {
    #[cfg(windows)]
    if hidden::started_hidden() {
        return hidden::spawn(program, args);
    }
    std::process::Command::new(program).args(args).spawn().map(|_| ())
}

#[cfg(windows)]
mod hidden {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetStartupInfoW, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
    };

    /// SW_HIDE (winuser.h).
    const SW_HIDE: u16 = 0;

    pub fn started_hidden() -> bool {
        // SAFETY: GetStartupInfoW fills the structure it is given; it cannot fail.
        let mut info: STARTUPINFOW = unsafe { std::mem::zeroed() };
        unsafe { GetStartupInfoW(&mut info) };
        info.dwFlags & STARTF_USESHOWWINDOW != 0 && info.wShowWindow == SW_HIDE
    }

    /// One argument as the C runtime's command-line parser reads it back (quotes and the backslashes before them).
    fn quote(arg: &[u16], out: &mut Vec<u16>) {
        let (space, tab, quote_mark, backslash) = (b' ' as u16, b'\t' as u16, b'"' as u16, b'\\' as u16);
        if !arg.is_empty() && !arg.iter().any(|&c| c == space || c == tab || c == quote_mark) {
            out.extend_from_slice(arg);
            return;
        }
        out.push(quote_mark);
        let mut backslashes = 0usize;
        for &c in arg {
            if c == backslash {
                backslashes += 1;
                continue;
            }
            // Backslashes before a quote are doubled and the quote escaped; elsewhere they stand as they are.
            let times = if c == quote_mark { backslashes * 2 + 1 } else { backslashes };
            out.extend(std::iter::repeat_n(backslash, times));
            backslashes = 0;
            out.push(c);
        }
        out.extend(std::iter::repeat_n(backslash, backslashes * 2));
        out.push(quote_mark);
    }

    pub fn spawn(program: &Path, args: &[OsString]) -> std::io::Result<()> {
        let mut line = Vec::new();
        quote(&program.as_os_str().encode_wide().collect::<Vec<u16>>(), &mut line);
        for arg in args {
            line.push(b' ' as u16);
            quote(&arg.encode_wide().collect::<Vec<u16>>(), &mut line);
        }
        line.push(0);
        let application: Vec<u16> = program.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: both strings end in NUL and outlive the call; `line` is writable, as CreateProcessW wants.
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        startup.dwFlags = STARTF_USESHOWWINDOW;
        startup.wShowWindow = SW_HIDE;
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let made = unsafe {
            CreateProcessW(application.as_ptr(), line.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, 0,
                           std::ptr::null(), std::ptr::null(), &startup, &mut process)
        };
        if made == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the handles CreateProcessW gave; the game runs on without them.
        unsafe {
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        fn quoted(arg: &str) -> String {
            let mut out = Vec::new();
            super::quote(&arg.encode_utf16().collect::<Vec<u16>>(), &mut out);
            String::from_utf16(&out).unwrap()
        }

        #[test]
        fn arguments_come_back_as_given() {
            assert_eq!(quoted("--seconds"), "--seconds");
            assert_eq!(quoted(""), "\"\"");
            assert_eq!(quoted(r"D:\Test Install\Nebula"), r#""D:\Test Install\Nebula""#);
            assert_eq!(quoted(r"D:\folder with space\"), r#""D:\folder with space\\""#);
            assert_eq!(quoted(r#"say "hi""#), r#""say \"hi\"""#);
            assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
        }
    }
}

/// What `add character` made: the model's place in the mod, the report's file beside the folder, the report's lines,
/// and the mod's check after.
pub struct Character {
    pub file: PathBuf,
    pub report_file: PathBuf,
    pub lines: Vec<String>,
    pub report: Report,
}

/// The rigged model rebound to the skeleton of the game's model `replaces` (mario, luigi, or a model's
/// name). Without `as_asset` it is that model's stand-in, written at its place in the mod's base layer, and the game's
/// own hands and face of that character are hidden. With `as_asset` it is a new model of the mod,
/// `layers/base/assets/<as_asset>.glb`, that the mod draws on that skeleton (a form's look, a costume); the game's
/// model and parts stay. The report is written beside the folder; then the mod is checked with `check` (resolved).
pub fn add_character(model: &Path, folder: &Path, replaces: &str, as_asset: Option<&str>, joints: &[(String, String)], pak: &Path, check: &Inputs) -> Result<Character> {
    if !folder.join("luma.json").is_file() {
        bail!(say!("LumaLink.Add.NoMod", folder = folder.display()));
    }
    // A path that is no mod path is refused before anything is read.
    let asset = match as_asset {
        Some(at) => match character::asset_in_mod(folder, at) {
            Some(out) => Some(out),
            None => bail!(say!("LumaLink.Character.AsPath", value = at)),
        },
        None => None,
    };
    // The game's own names for its two players; any other is a model's name as the game has it.
    let name = match replaces.to_ascii_lowercase().as_str() {
        "mario" => "Mario".to_string(),
        "luigi" => "Luigi".to_string(),
        _ => replaces.to_string(),
    };
    let (path, block) = character::find_model(pak, &name)?;
    let skeleton = character::GameSkeleton::read(&path, &block).with_context(|| say!("LumaLink.Character.NotModelBlock", path = path))?;
    let source = std::fs::read(model).with_context(|| say!("LumaLink.CannotRead", path = model.display()))?;
    let (glb, report) = character::rebind(&source, &skeleton, joints)?;
    let out = asset.unwrap_or_else(|| character::place_in_mod(folder, &path));
    if out.exists() {
        bail!(say!("LumaLink.AlreadyThere", path = out.display()));
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).with_context(|| say!("LumaLink.CannotMake", path = parent.display()))?;
    }
    std::fs::write(&out, glb).with_context(|| say!("LumaLink.CannotWrite", path = out.display()))?;
    let mut lines = character::report_lines(&report);
    // A new asset leaves the game's model and its parts as they are.
    let parts: &[&str] = if as_asset.is_some() { &[] } else { character::part_models(&name) };
    for part in parts {
        let Ok((part_path, _)) = character::find_model(pak, part) else { continue };
        let entry = character::hide_in_mod(folder, &part_path);
        if entry.exists() {
            continue;
        }
        if let Some(parent) = entry.parent() {
            std::fs::create_dir_all(parent).with_context(|| say!("LumaLink.CannotMake", path = parent.display()))?;
        }
        std::fs::write(&entry, character::HIDE_ENTRY).with_context(|| say!("LumaLink.CannotWrite", path = entry.display()))?;
        lines.push(say!("LumaLink.Character.Hidden", path = part_path));
    }
    let folder_name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "mod".into());
    let report_file = parent_of(folder).join(format!("{folder_name}-{}-character.txt", name.to_ascii_lowercase()));
    std::fs::write(&report_file, lines.join("\n") + "\n").with_context(|| say!("LumaLink.CannotWrite", path = report_file.display()))?;
    let checked = validate(folder, check)?;
    Ok(Character { file: out, report_file, lines, report: checked })
}

/// What came of `convert`.
pub enum Convert {
    /// The folder written, then checked with the inputs given.
    Done { converted: crate::convert::Converted, report: Report },
    /// Files the game cannot use, each with why: nothing written until the maker accepts leaving them out.
    Losses(crate::convert::Losses),
}

/// The mod at `from` as the mod folder `folder` in the one form, each file cooked as the game cooks it, then
/// checked with `check` (resolved; its game places the files and names the textures).
pub fn convert(from: &Path, folder: &Path, given: &crate::convert::Given, check: &Inputs) -> Result<Convert> {
    let store = check.game.as_deref().map(crate::game_store::GameStore::open).transpose()?;
    let converted = match crate::convert::convert(from, folder, store.as_ref(), given) {
        Ok(done) => done,
        Err(error) => match error.downcast::<crate::convert::Losses>() {
            Ok(losses) => return Ok(Convert::Losses(losses)),
            Err(error) => return Err(error),
        },
    };
    let report = validate(folder, check)?;
    Ok(Convert::Done { converted, report })
}

/// The container rules' errors (L100 to L199): a .luma with any is not opened.
fn container_errors(report: &Report) -> Vec<Finding> {
    report.findings.iter().filter(|f| (100..200).contains(&f.code) && f.level == Level::Error).cloned().collect()
}

/// What is in a .luma: its entries, or the validator's words on why it does not read. Every entry's data is read and
/// checked (L100 to L107); `damaged` names those that are not whole.
pub enum LumaList {
    Entries { entries: Vec<luma_file::Listed>, damaged: Vec<Finding> },
    Unreadable(Report),
}

pub fn list_luma(luma: &Path) -> Result<LumaList> {
    let report = validate(luma, &Inputs::none())?;
    let Ok(entries) = luma_file::list(luma) else {
        return Ok(LumaList::Unreadable(report));
    };
    Ok(LumaList::Entries { entries, damaged: container_errors(&report) })
}

/// Entries of a .luma (all when `names` is empty) written into `output`: the names written, or the container errors
/// that keep it shut (L100 to L107 hold every name to what every system stores).
pub fn extract_luma(luma: &Path, names: &[String], output: &Path) -> Result<std::result::Result<Vec<String>, Vec<Finding>>> {
    let report = validate(luma, &Inputs::none())?;
    let container = container_errors(&report);
    if !container.is_empty() {
        return Ok(Err(container));
    }
    Ok(Ok(luma_file::extract(luma, names, output)?))
}

/// What a validator code means: its number, level and template with its parameter names in braces.
pub fn explain(code: &str) -> Result<(u16, Level, String)> {
    let Ok(number) = code.trim_start_matches(['L', 'l']).parse::<u16>() else {
        bail!(say!("LumaLink.NotACode", value = code));
    };
    if !luma_validate::CODES.contains(&number) {
        let version = format!("{}.{}", luma_validate::VERSION.0, luma_validate::VERSION.1);
        bail!(say!("LumaLink.UnknownCode", value = format!("L{number}"), version = version));
    }
    let level = luma_validate::level_of(number);
    let shown = Finding { code: number, level, mod_id: Some("{mod}".into()), file: "{file}".into(), line: None, column: None, pointer: None, args: vec![] };
    Ok((number, level, luma_validate::message(&shown, text::language())))
}

/// How a .luma and the folder it was packed from differ, entry by entry.
pub enum Difference {
    /// Only in the .luma.
    OnlyInLuma(String),
    /// Only in what the folder packs to.
    OnlyInFolder(String),
    /// In both, not the same bytes.
    Differs(String),
}

/// Whether a .luma is exactly what its folder packs to: Ok(bytes) when it is, else each difference.
pub fn verify(luma: &Path, source: &Path) -> Result<std::result::Result<usize, Vec<Difference>>> {
    let files = luma_file::folder_files(source)?;
    let scratch = tempfile::tempdir_in(parent_of(luma))?;
    let again = scratch.path().join("again.luma");
    luma_file::write(&files, &again)?;
    let a = std::fs::read(luma).with_context(|| say!("LumaLink.CannotRead", path = luma.display()))?;
    let b = std::fs::read(&again)?;
    if a == b {
        return Ok(Ok(a.len()));
    }
    // Each file opened once and its entries found by name: linear in the entries, however many.
    let mut theirs = luma_file::Reader::open(luma)?;
    let mut ours = luma_file::Reader::open(&again)?;
    let mut out = Vec::new();
    for e in theirs.list() {
        if !ours.has(&e.name) {
            out.push(Difference::OnlyInLuma(e.name));
        } else if theirs.read(&e.name)? != ours.read(&e.name)? {
            out.push(Difference::Differs(e.name));
        }
    }
    for o in ours.list() {
        if !theirs.has(&o.name) {
            out.push(Difference::OnlyInFolder(o.name));
        }
    }
    Ok(Err(out))
}

/// The `.lum` files `paths` name, a folder's every one under it (but build output: `target`, `build`), in order, each
/// put in the one style (`lumascript::fmt`); with `check` none is written. Gives each file and whether its
/// text was not in the style. A path that is neither a folder nor a `.lum` is refused by name.
pub fn format(paths: &[PathBuf], check: bool) -> Result<Vec<(PathBuf, bool)>> {
    fn walk(folder: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let entries = std::fs::read_dir(folder).with_context(|| say!("LumaLink.CannotRead", path = folder.display()))?;
        let mut here: Vec<PathBuf> = Vec::new();
        for e in entries {
            here.push(e.with_context(|| say!("LumaLink.CannotRead", path = folder.display()))?.path());
        }
        here.sort();
        for path in here {
            if path.is_dir() {
                if !path.file_name().is_some_and(|n| n == "target" || n == "build") {
                    walk(&path, out)?;
                }
            } else if is_lum(&path) {
                out.push(path);
            }
        }
        Ok(())
    }
    fn is_lum(path: &Path) -> bool {
        path.extension().is_some_and(|x| x == "lum")
    }
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut files)?;
        } else if p.is_file() && is_lum(p) {
            files.push(p.clone());
        } else {
            bail!(say!("LumaLink.Format.NotLum", path = p.display()));
        }
    }
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let src = std::fs::read_to_string(&f).with_context(|| say!("LumaLink.CannotRead", path = f.display()))?;
        let styled = lumascript::fmt::format(&src);
        let changed = styled != src;
        if changed && !check {
            std::fs::write(&f, styled).with_context(|| say!("LumaLink.CannotWrite", path = f.display()))?;
        }
        out.push((f, changed));
    }
    Ok(out)
}

/// The jobs not built yet, with the task that brings each: the refusal both front ends give.
pub fn not_built(command: &str) -> Option<String> {
    let what = match command {
        "build" => say!("LumaLink.NotBuilt.Build"),
        _ => return None,
    };
    Some(say!("LumaLink.NotBuilt", command = command, what = what))
}

/// Where `lumalink test` finds the proof harness (docs/modding/lumalink/test-harness.md): its runner (`tools/lumacore/luma_test.py`), the
/// Python that runs it, the bin folder of a game build with the harness's parts (the host extension and the game
/// half), the work folder each run is left in, the run lock, and the renderer (the runner's default when none).
#[derive(Clone, Debug)]
pub struct TestSetup {
    pub bin: PathBuf,
    pub runner: PathBuf,
    pub python: PathBuf,
    pub work: PathBuf,
    pub lock: PathBuf,
    pub renderer: Option<String>,
}

/// One test's run: the test folder, every result it left (one per renderer), and what the runner said when it left
/// none (its exit code and output).
#[derive(Clone, Debug)]
pub struct TestRun {
    pub test: PathBuf,
    pub results: Vec<TestResult>,
    pub code: i32,
    pub said: String,
}

/// A run's `result.json` (docs/modding/lumalink/test-harness.md, section 5): the verdict, its reason, and each check with what was found.
#[derive(Clone, Debug)]
pub struct TestResult {
    pub file: PathBuf,
    pub test: String,
    pub renderer: String,
    pub verdict: String,
    pub reason: String,
    pub checks: Vec<(String, String, serde_json::Value)>,
}

impl TestRun {
    /// A test passes when it left a result for each renderer run and every verdict is `pass` (docs/modding/lumalink/test-harness.md, section 5: only pass
    /// is proof).
    pub fn passed(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.verdict == "pass")
    }
}

/// The runner beside a game build's bin folder: the first `tools/lumacore/luma_test.py` above it (a Nebula tree's
/// build folders are under the tree).
pub fn find_runner(bin: &Path) -> Option<PathBuf> {
    let bin = std::path::absolute(bin).ok()?;
    bin.ancestors().map(|a| a.join("tools/lumacore/luma_test.py")).find(|p| p.is_file())
}

/// Whether the mod has code to build: LumaScript, C or C++ under `source/`.
pub fn has_code(folder: &Path) -> bool {
    fn any(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        entries.flatten().any(|e| {
            let p = e.path();
            if p.is_dir() {
                any(&p)
            } else {
                p.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "lum" | "c" | "cpp" | "cc" | "cxx"))
            }
        })
    }
    any(&folder.join("source"))
}

/// The mods the tests bring with them: each folder under `tests/` holding a `luma.json` and sources, in path order. A
/// mod's folder is not searched further (its own `tests/` are its own).
pub fn test_mods(folder: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let entries = std::fs::read_dir(dir).with_context(|| say!("LumaLink.CannotRead", path = dir.display()))?;
        let mut here: Vec<PathBuf> = Vec::new();
        for e in entries {
            here.push(e.with_context(|| say!("LumaLink.CannotRead", path = dir.display()))?.path());
        }
        here.sort();
        for p in here.iter().filter(|p| p.is_dir()) {
            if p.join("luma.json").is_file() {
                if has_code(p) {
                    out.push(p.clone());
                }
            } else {
                walk(p, out)?;
            }
        }
        Ok(())
    }
    let tests = folder.join("tests");
    let mut out = Vec::new();
    if tests.is_dir() {
        walk(&tests, &mut out)?;
    }
    Ok(out)
}

/// The mod's tests: each folder under `tests/` holding a `test.json`, in path order.
pub fn test_folders(folder: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let entries = std::fs::read_dir(dir).with_context(|| say!("LumaLink.CannotRead", path = dir.display()))?;
        let mut here: Vec<PathBuf> = Vec::new();
        for e in entries {
            here.push(e.with_context(|| say!("LumaLink.CannotRead", path = dir.display()))?.path());
        }
        here.sort();
        if here.iter().any(|p| p.file_name().is_some_and(|n| n == "test.json") && p.is_file()) {
            out.push(dir.to_path_buf());
        }
        for p in here.iter().filter(|p| p.is_dir()) {
            walk(p, out)?;
        }
        Ok(())
    }
    let tests = folder.join("tests");
    let mut out = Vec::new();
    if tests.is_dir() {
        walk(&tests, &mut out)?;
    }
    Ok(out)
}

/// Reads a run's `result.json`; one that does not read is refused naming it.
pub fn test_result(file: &Path) -> Result<TestResult> {
    let bytes = std::fs::read(file).with_context(|| say!("LumaLink.CannotRead", path = file.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!(say!("LumaLink.Test.BadResult", path = file.display(), why = e)))?;
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    let Some(verdict) = v.get("verdict").and_then(serde_json::Value::as_str) else {
        bail!(say!("LumaLink.Test.BadResult", path = file.display(), why = say!("LumaLink.Test.NoVerdict")));
    };
    let checks = v
        .get("checks")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| {
                    let s = |k: &str| c.get(k).map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string)).unwrap_or_default();
                    (s("check"), s("result"), c.get("actual").cloned().unwrap_or(serde_json::Value::Null))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(TestResult { file: file.to_path_buf(), test: text("test"), renderer: text("renderer"), verdict: verdict.to_string(), reason: text("reason"), checks })
}

/// Every `result.json` under `dir`, in path order.
fn result_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut here: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    here.sort();
    for p in here {
        if p.is_dir() {
            result_files(&p, out);
        } else if p.file_name().is_some_and(|n| n == "result.json") {
            out.push(p);
        }
    }
}

/// The mod's tests through the proof harness. The mod is built first when it has code (`build`, the front
/// end's own build of every platform), then its `tests/**/test.json` are run by the harness's runner as one suite, in
/// path order (a `suite.json` in this command's own folder under `setup.work`), so a test may continue from one before
/// it (`continue_from`). The runner installs the game half and the test's mods into a private run folder for each and
/// starts the game hidden and silent. `each` hears each test, with the results found under its name, once the suite
/// ends. A mod with no tests is refused by name: a test command that tests nothing proves nothing.
pub fn test(folder: &Path, setup: &TestSetup, build: &dyn Fn(&Path) -> Result<()>, each: &mut dyn FnMut(&TestRun)) -> Result<Vec<TestRun>> {
    if !folder.join("luma.json").is_file() {
        bail!(say!("LumaLink.Add.NoMod", folder = folder.display()));
    }
    let tests = test_folders(folder)?;
    if tests.is_empty() {
        bail!(say!("LumaLink.Test.NoTests", folder = folder.join("tests").display()));
    }
    if !setup.runner.is_file() {
        bail!(say!("LumaLink.Test.NoRunner", path = setup.runner.display()));
    }
    if !setup.bin.is_dir() {
        bail!(say!("LumaLink.NotFolder", path = setup.bin.display()));
    }
    if has_code(folder) {
        build(folder)?;
    }
    // A test's own mods (a listener the mod talks to, a second player) live beside its test.json, with their sources:
    // they are built the same way, so a test folder is whole without built code in it.
    for m in test_mods(folder)? {
        build(&m)?;
    }
    // Each lumalink test leaves its runs in a folder of its own: earlier runs are never overwritten or read as these.
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let session = setup.work.join(format!("lumalink-test-{stamp}"));
    std::fs::create_dir_all(&session).with_context(|| say!("LumaLink.CannotMake", path = session.display()))?;
    // The suite: every test folder, whole paths, in path order.
    let mut listed = Vec::new();
    for t in &tests {
        let whole = std::path::absolute(t).with_context(|| say!("LumaLink.CannotRead", path = t.display()))?;
        listed.push(serde_json::Value::String(whole.to_string_lossy().into_owned()));
    }
    let suite = session.join("suite.json");
    let text = serde_json::to_string_pretty(&serde_json::json!({ "harness": 1, "tests": listed }))?;
    std::fs::write(&suite, text + "\n").with_context(|| say!("LumaLink.CannotWrite", path = suite.display()))?;
    let mut cmd = std::process::Command::new(&setup.python);
    cmd.args(["-X", "utf8", "-I"]).arg(&setup.runner).arg("run").arg(&suite);
    cmd.arg("--bin").arg(&setup.bin).arg("--work").arg(&session).arg("--who").arg("lumalink").arg("--lock").arg(&setup.lock);
    if let Some(r) = &setup.renderer {
        cmd.arg("--renderer").arg(r);
    }
    let out = cmd.output().with_context(|| say!("LumaLink.Test.NoPython", path = setup.python.display()))?;
    let mut said = String::from_utf8_lossy(&out.stdout).into_owned();
    said.push_str(&String::from_utf8_lossy(&out.stderr));
    let code = out.status.code().unwrap_or(-1);
    // The runner leaves a test's runs under runs/<suite>/<the test's name>/, and one it could not read under its folder's
    // name.
    let suite_runs = session.join("runs").join(session.file_name().unwrap_or_default());
    let mut runs = Vec::new();
    for test in &tests {
        let folder_name = test.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = test_name(test).unwrap_or_else(|| folder_name.clone());
        let mut files = Vec::new();
        result_files(&suite_runs.join(&name), &mut files);
        if name != folder_name {
            result_files(&suite_runs.join(&folder_name), &mut files);
        }
        let mut results = Vec::new();
        for f in &files {
            results.push(test_result(f)?);
        }
        let run = TestRun { test: test.clone(), results, code, said: said.clone() };
        each(&run);
        runs.push(run);
    }
    Ok(runs)
}

/// A test's name as its test.json gives it (the runner names its runs' folder after it); None when it gives none.
fn test_name(test: &Path) -> Option<String> {
    let text = std::fs::read_to_string(test.join("test.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("name")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library in the mod folder gives its data schemas to the check of a mod that uses it, as in the game.
    #[test]
    fn a_library_in_the_mod_folder_gives_its_schemas_to_the_mods_that_use_it() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = |id: &str, more: &str| format!(r#"{{"format": "1.0", "id": "{id}", "version": "1.0.0", "name": "T", "summary": "A test.", "authors": ["G"], "game": {{"id": "RMG"}}{more}}}"#);
        let lib = dir.path().join("lib");
        std::fs::create_dir_all(lib.join("code/lib.mod.kit/shop")).unwrap();
        std::fs::write(lib.join("luma.json"), manifest("lib.mod", "")).unwrap();
        std::fs::write(lib.join("code/lib.mod.kit/shop/1.json"), r#"{"type": "object", "properties": {"price": {"type": "integer", "minimum": 0}}}"#).unwrap();
        let user = dir.path().join("user");
        std::fs::create_dir_all(user.join("kit/shop")).unwrap();
        std::fs::write(user.join("luma.json"), manifest("user.mod", r#", "needs": [{"id": "lib.mod", "version": ">=1.0.0"}], "kit": {"lib.mod.shop": 1}"#)).unwrap();
        std::fs::write(user.join("kit/shop/items.json"), r#"{"price": -1}"#).unwrap();
        let installed = mod_folder(dir.path(), &user).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].touches.kit_schemas.len(), 1, "{:?}", installed[0].touches);
        let report = luma_validate::validate(&luma_validate::Input {
            source: luma_validate::Source::Folder(&user),
            game: None,
            installed: &installed,
            platforms: &[],
            options: None,
            position: None,
        });
        let codes: Vec<u16> = report.findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, vec![701], "{:?}", report.findings);
        // Without the game the files are not cooked, and the report says so for each one.
        std::fs::create_dir_all(user.join("layers/base/assets")).unwrap();
        std::fs::write(user.join("layers/base/assets/Ball.glb"), b"").unwrap();
        let report = validate(&user, &Inputs { no_installed: true, ..Inputs::default() }).unwrap();
        assert!(report.not_checked.iter().any(|n| n.code == 804 && n.because == "Missing.Store" && n.places == 1), "{:?}", report.not_checked);
    }

    /// A test's own mod with sources is found to be built; one with no sources, and anything inside a found mod's
    /// folder, is not.
    #[test]
    fn a_tests_own_mods_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let m = dir.path().join("m");
        for (f, text) in [
            ("tests/run/test.json", "{}"),
            ("tests/run/server/luma.json", "{}"),
            ("tests/run/server/source/main.lum", "on start { }"),
            ("tests/run/server/tests/x/inner/luma.json", "{}"),
            ("tests/run/server/tests/x/inner/source/main.lum", ""),
            ("tests/other/data/luma.json", "{}"),
        ] {
            let p = m.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let found = test_mods(&m).unwrap();
        let rel: Vec<String> = found.iter().map(|p| p.strip_prefix(&m).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["tests/run/server"]);
        assert!(test_mods(&dir.path().join("none")).unwrap().is_empty());
    }

    #[test]
    fn tests_are_found_and_their_results_read() {
        let dir = tempfile::tempdir().unwrap();
        let m = dir.path().join("m");
        for t in ["tests/b", "tests/a/inner", "tests/a/inner/deeper", "tests/empty"] {
            std::fs::create_dir_all(m.join(t)).unwrap();
        }
        for t in ["tests/b", "tests/a/inner", "tests/a/inner/deeper"] {
            std::fs::write(m.join(t).join("test.json"), "{}").unwrap();
        }
        let found = test_folders(&m).unwrap();
        let rel: Vec<String> = found.iter().map(|p| p.strip_prefix(&m).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["tests/a/inner", "tests/a/inner/deeper", "tests/b"]);
        assert!(!has_code(&m));
        std::fs::create_dir_all(m.join("source/x")).unwrap();
        std::fs::write(m.join("source/x/main.lum"), "").unwrap();
        assert!(has_code(&m));
        let file = dir.path().join("result.json");
        std::fs::write(&file, r#"{"harness": 1, "test": "t", "renderer": "d3d12", "verdict": "fail", "reason": "r", "checks": [{"check": "fact m.x is 1", "result": "fail", "actual": 2}]}"#).unwrap();
        let r = test_result(&file).unwrap();
        assert_eq!((r.test.as_str(), r.renderer.as_str(), r.verdict.as_str(), r.reason.as_str()), ("t", "d3d12", "fail", "r"));
        assert_eq!(r.checks, vec![("fact m.x is 1".to_string(), "fail".to_string(), serde_json::json!(2))]);
        std::fs::write(&file, r#"{"test": "t"}"#).unwrap();
        assert!(test_result(&file).is_err());
        let run = TestRun { test: m.clone(), results: vec![], code: 1, said: String::new() };
        assert!(!run.passed(), "no result is no pass");
        // A mod with no tests is refused by name.
        let none = dir.path().join("none");
        std::fs::create_dir_all(&none).unwrap();
        std::fs::write(none.join("luma.json"), "{}").unwrap();
        let setup = TestSetup { bin: dir.path().into(), runner: file.clone(), python: "python".into(), work: dir.path().into(), lock: dir.path().join("run.lock"), renderer: None };
        let e = test(&none, &setup, &|_| Ok(()), &mut |_| {}).err().unwrap();
        assert!(e.to_string().contains("tests"), "{e}");
    }

    /// A mod's tests run as one suite: one runner call with every test folder in path order in one work folder, so the
    /// second test, which continues from the first, finds it passed. The runner here stands in for the harness's: it
    /// passes a test unless its continue_from names one that did not pass before it in the same call.
    #[test]
    fn a_mods_tests_run_as_one_suite_so_one_can_continue_from_another() {
        if std::process::Command::new("python").arg("--version").output().is_err() {
            eprintln!("skipped: no python on the path to run the stand-in runner");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let m = dir.path().join("m");
        for (t, text) in [("tests/a-first", r#"{"name": "first"}"#), ("tests/b-second", r#"{"name": "second", "continue_from": "first"}"#)] {
            std::fs::create_dir_all(m.join(t)).unwrap();
            std::fs::write(m.join(t).join("test.json"), text).unwrap();
        }
        std::fs::write(m.join("luma.json"), "{}").unwrap();
        let runner = dir.path().join("runner.py");
        std::fs::write(
            &runner,
            r#"import json, pathlib, sys
a = sys.argv[1:]
suite = pathlib.Path(a[1])
work = pathlib.Path(a[a.index("--work") + 1])
lock = pathlib.Path(a[a.index("--lock") + 1])
assert not lock.exists()
lock.write_text("held")
tests = json.loads(suite.read_text())["tests"]
calls = work.parent.parent / "calls.txt"
calls.write_text(calls.read_text() + "x" if calls.exists() else "x")
passed = set()
for t in tests:
    test = json.loads((pathlib.Path(t) / "test.json").read_text())
    after = test.get("continue_from")
    verdict = "pass" if after is None or after in passed else "setup-failed"
    if verdict == "pass":
        passed.add(test["name"])
    out = work / "runs" / suite.parent.name / test["name"] / "d3d12"
    out.mkdir(parents=True)
    (out / "result.json").write_text(json.dumps({"harness": 1, "test": test["name"], "renderer": "d3d12", "verdict": verdict, "reason": "", "checks": []}))
    print(test["name"], verdict)
lock.unlink()
"#,
        )
        .unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let setup = TestSetup { bin: dir.path().into(), runner, python: "python".into(), work: work.clone(), lock: dir.path().join("run.lock"), renderer: None };
        let mut heard = Vec::new();
        let runs = test(&m, &setup, &|_| Ok(()), &mut |r| heard.push(r.test.clone())).unwrap();
        assert_eq!(heard.len(), 2);
        let verdicts: Vec<(String, String)> = runs.iter().flat_map(|r| r.results.iter().map(|x| (x.test.clone(), x.verdict.clone()))).collect();
        assert_eq!(verdicts, [("first".to_string(), "pass".to_string()), ("second".to_string(), "pass".to_string())], "{}", runs[0].said);
        assert!(runs.iter().all(|r| r.passed()));
        assert_eq!(std::fs::read_to_string(dir.path().join("calls.txt")).unwrap(), "x", "one runner call for the suite");
        // The suite the runner was given: both folders, whole, in path order.
        let session = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.path()).next().unwrap();
        let suite: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(session.join("suite.json")).unwrap()).unwrap();
        let listed: Vec<String> = suite["tests"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().replace('\\', "/")).collect();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].ends_with("m/tests/a-first") && listed[1].ends_with("m/tests/b-second"), "{listed:?}");
        assert!(std::path::Path::new(&listed[0]).is_absolute());
    }

    #[test]
    fn a_packed_mod_lists_extracts_and_verifies_against_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("m");
        assert!(matches!(new_mod(&folder, "test.round", "Round", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        // Nothing in it needs the game, so it packs with no inputs.
        let (report, outcome) = pack(&folder, None, &Inputs::none()).unwrap();
        let PackOutcome::Packed { file, .. } = outcome else { panic!("not packed: {:?} {:?}", report.findings, report.not_checked) };
        let LumaList::Entries { entries, damaged } = list_luma(&file).unwrap() else { panic!("unreadable") };
        assert!(damaged.is_empty() && entries.iter().any(|e| e.name == "luma.json"));
        let out = dir.path().join("out");
        let written = extract_luma(&file, &[], &out).unwrap().unwrap_or_else(|f| panic!("{f:?}"));
        assert!(written.iter().any(|n| n == "luma.json") && out.join("luma.json").is_file());
        assert_eq!(std::fs::read(out.join("luma.json")).unwrap(), std::fs::read(folder.join("luma.json")).unwrap());
        assert!(matches!(verify(&file, &folder).unwrap(), Ok(bytes) if bytes as u64 == std::fs::metadata(&file).unwrap().len()));
        // A change to the folder is named as a difference.
        std::fs::write(folder.join("luma.json"), b"{}").unwrap();
        let differences = verify(&file, &folder).unwrap().err().expect("differences");
        assert!(differences.iter().any(|d| matches!(d, Difference::Differs(name) if name == "luma.json")));
    }

    #[test]
    fn a_new_mod_is_checked_before_it_is_written_and_packs_with_every_rule_said() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad");
        assert!(matches!(new_mod(&bad, "9no", "Name", "One line.", &[]).unwrap(), NewMod::Refused(_)));
        assert!(!bad.exists(), "a refused mod writes nothing");
        let good = dir.path().join("good");
        assert!(matches!(new_mod(&good, "test.mod", "Name", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        assert!(good.join("luma.json").is_file() && good.join("layers/base/files").is_dir());
        // A need with no mod folder to find it in is not checked, so pack refuses and says so.
        let need = crate::add::Part::Need { id: "test.kit".into(), version: ">=0.1.0".into() };
        assert!(matches!(crate::add::add(&good, &need, &Inputs::none()).unwrap(), crate::add::Added::Done { .. }));
        let (report, outcome) = pack(&good, None, &Inputs::none()).unwrap();
        assert!(report.passed());
        assert!(matches!(outcome, PackOutcome::Unchecked));
        assert!(report.not_checked.iter().any(|n| n.code == 800 && n.because == "Missing.ModFolder" && n.places == 1), "{:?}", report.not_checked);
        let lines = report_lines(&report);
        assert_eq!(lines.last().unwrap(), &summary_line(&report));
        // With LumaLink's mod folder the need is found there, or named missing when it is not.
        let mods = dir.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        let found = Inputs { mods: Some(mods.clone()), ..Inputs::none() };
        assert!(matches!(new_mod(&mods.join("kit"), "test.kit", "Kit", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        let report = validate(&good, &found).unwrap();
        assert!(report.passed() && report.not_checked.iter().all(|n| n.code != 800), "{:?} {:?}", report.findings, report.not_checked);
        std::fs::remove_dir_all(mods.join("kit")).unwrap();
        let report = validate(&good, &found).unwrap();
        assert!(report.findings.iter().any(|f| f.code == 800), "{:?}", report.findings);
    }

    #[test]
    fn a_folder_packed_from_inside_the_mod_folder_is_not_another_installed_mod() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        let folder = mods.join("tweak");
        assert!(matches!(new_mod(&folder, "test.tweak", "Tweak", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        let inputs = Inputs { mods: Some(mods.clone()), ..Inputs::none() };
        let output = dir.path().join("tweak.luma");
        let (report, outcome) = pack(&folder, Some(output.clone()), &inputs).unwrap();
        assert!(report.passed(), "{:?}", report.findings);
        match outcome {
            PackOutcome::Packed { file, .. } => assert_eq!(file, output),
            PackOutcome::WrittenFails(again) => panic!("{:?}", again.findings),
            _ => panic!("not packed: {:?}", report.not_checked),
        }
        // Another mod in the folder with the same id is still one.
        let twin = mods.join("twin");
        std::fs::create_dir(&twin).unwrap();
        std::fs::copy(folder.join("luma.json"), twin.join("luma.json")).unwrap();
        let (report, outcome) = pack(&folder, Some(dir.path().join("again.luma")), &inputs).unwrap();
        assert!(matches!(outcome, PackOutcome::Errors) && report.findings.iter().any(|f| f.code == 802), "{:?}", report.findings);
    }

    #[test]
    fn explain_gives_the_template_and_refuses_unknown_codes() {
        let (number, level, template) = explain("L302").unwrap();
        assert_eq!((number, level), (302, Level::Error));
        assert!(template.contains("{mod}") && template.contains("{file}"));
        assert!(explain("L9999").is_err() && explain("nope").is_err());
        assert!(not_built("build").is_some() && not_built("pack").is_none() && not_built("test").is_none());
    }

    #[test]
    fn the_installed_game_fills_only_the_inputs_not_named() {
        let found = Installed { pak: "a/game.pak".into(), data: "d".into(), root: None, version: None };
        let named = Inputs { game: Some("mine.pak".into()), ..Inputs::default() };
        assert_eq!(named.resolved(Some(&found)).game, Some(PathBuf::from("mine.pak")));
        assert_eq!(Inputs::default().resolved(Some(&found)).game, Some(PathBuf::from("a/game.pak")));
        assert_eq!(Inputs::none().resolved(Some(&found)).game, None);
    }

    /// `add character --as`: a path that is no mod path is refused by name before the game or the model is read, and
    /// nothing is written into the mod.
    #[test]
    fn add_character_as_refuses_a_path_outside_the_assets() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("m");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("luma.json"), "{}").unwrap();
        let no_pak = dir.path().join("no.pak");
        for bad in ["../Gold", "C:/Gold", "a\\b", "Gold.gltf", ""] {
            let e = add_character(Path::new("no.glb"), &folder, "mario", Some(bad), &[], &no_pak, &Inputs::none()).err().unwrap();
            assert_eq!(e.to_string(), say!("LumaLink.Character.AsPath", value = bad), "{bad}");
        }
        assert!(!folder.join("layers").exists());
    }
}
