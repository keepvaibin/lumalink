//! The LumaLink window: every job the command line does, on what is dropped on it or opened. The mods, published
//! mods and models dropped are tiles; a tile opens its own screen with its jobs and what came of the last, each finding
//! with its file and line (`screen`). The game mods are checked against is the installed one, found as the command
//! line finds it, or a game.pak chosen. The jobs are the library's (`lumalink_core::jobs`); the window only shows them,
//! each on a thread of its own so the window never waits.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText, Stroke};
use lumalink_gui::chrome::Mark;
use lumalink_gui::{look, sky, words};
use updates::Update;
use luma_validate::{Finding, Level, Report};
use lumalink_core::installed::Installed;
use lumalink_core::jobs::{self, Convert, Difference, Inputs, Install, LumaList, NewMod, PackOutcome};
use lumalink_core::{say, text};
mod screen;
#[cfg(debug_assertions)]
mod shot;
mod updates;

use sky::{AMBER, BLUE_INK, GREEN, INK, RED, SOFT};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

/// What a dropped or opened path is, which says the jobs it has.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A folder with a luma.json.
    Folder,
    /// A .luma file.
    Luma,
    /// A rigged model (.glb, .gltf): added to a mod as a character.
    Model,
    /// Anything else: a published mod to convert (Riivolution pack, disc-layout folder, archive, texture pack...).
    Other,
}

impl Kind {
    fn of(path: &Path) -> Kind {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        if path.is_dir() && path.join("luma.json").is_file() {
            Kind::Folder
        } else if lower.ends_with(".luma") {
            Kind::Luma
        } else if lower.ends_with(".glb") || lower.ends_with(".gltf") {
            Kind::Model
        } else {
            Kind::Other
        }
    }

    fn label(self) -> String {
        match self {
            Kind::Folder => say!("LumaLink.Window.Kind.Folder"),
            Kind::Luma => say!("LumaLink.Window.Kind.Luma"),
            Kind::Model => say!("LumaLink.Window.Kind.Model"),
            Kind::Other => say!("LumaLink.Window.Kind.Other"),
        }
    }
}

/// One line of what a job said: a finding (level, code, its file and line) or plain words.
#[derive(Clone)]
struct Line {
    color: Color32,
    code: Option<u16>,
    text: String,
    /// The finding's file with its line and column (`file:line:column`), and the file on disk when it is one.
    place: Option<(String, Option<PathBuf>)>,
}

impl Line {
    fn words(text: String, color: Color32) -> Line {
        Line { color, code: None, text, place: None }
    }
}

/// A finding as a line, its file found under the mod's folder when the mod is one.
fn finding(f: &Finding, root: Option<&Path>) -> Line {
    let color = if f.level == Level::Error { RED } else { AMBER };
    let on_disk = root.map(|r| r.join(&f.file)).filter(|p| p.is_file());
    Line { color, code: Some(f.code), text: jobs::finding_line(f), place: Some((jobs::place(f), on_disk)) }
}

/// A report's lines: each finding, each rule not checked, then the count in the colour of the outcome.
fn report(r: &Report, root: Option<&Path>) -> Vec<Line> {
    let mut out: Vec<Line> = r.findings.iter().map(|f| finding(f, root)).collect();
    let all = jobs::report_lines(r);
    // report_lines: the findings, then the rules not checked, then the count.
    for line in &all[r.findings.len()..all.len() - 1] {
        out.push(Line::words(line.clone(), SOFT));
    }
    let color = if !r.passed() { RED } else if r.not_checked.iter().any(luma_validate::NotChecked::needs_input) { AMBER } else { GREEN };
    out.push(Line::words(jobs::summary_line(r), color));
    out
}

/// A pack's report and what came of it; the .luma written, when one was.
fn pack_lines(r: &Report, outcome: PackOutcome, root: Option<&Path>) -> (Vec<Line>, Option<PathBuf>) {
    let mut lines = report(r, root);
    let mut made = None;
    match outcome {
        PackOutcome::Packed { file, summary } => {
            lines.push(Line::words(say!("LumaLink.Packed", file = file.display(), files = summary.files, bytes = summary.bytes, unpacked = summary.unpacked), GREEN));
            made = Some(file);
        }
        PackOutcome::Errors => lines.push(Line::words(say!("LumaLink.NotPacked.Errors"), RED)),
        PackOutcome::Unchecked => lines.push(Line::words(say!("LumaLink.NotPacked.Unchecked"), RED)),
        PackOutcome::WrittenFails(again) => {
            lines.extend(report(&again, root));
            lines.push(Line::words(say!("LumaLink.WrittenFails"), RED));
        }
    }
    (lines, made)
}

/// A job the window runs, with what it needs beyond the item.
enum Job {
    Check,
    Build,
    /// The .luma at `to` (beside the folder when None), its code built first when it changed.
    Pack { to: Option<PathBuf> },
    Install { play: bool },
    /// A published mod as a mod folder, to work on.
    Convert { folder: PathBuf, accept_losses: bool },
    /// A published mod as a .luma: converted in a folder of its own that goes away, then packed to `luma`.
    MakeLuma { luma: PathBuf, accept_losses: bool },
    AddCharacter { folder: PathBuf, replaces: String },
    /// A new mod beside the model, with the model as the character it stands in for.
    CharacterMod { replaces: String },
    Verify { folder: PathBuf },
    List,
    Extract { into: PathBuf },
    Explain(u16),
}

impl Job {
    /// The job's name in its job file (lumalink_core::running), for whoever reads the file.
    fn name(&self) -> &'static str {
        match self {
            Job::Check => "check",
            Job::Build => "build",
            Job::Pack { .. } => "pack",
            Job::Install { .. } => "install",
            Job::Convert { .. } => "convert",
            Job::MakeLuma { .. } => "make-luma",
            Job::AddCharacter { .. } => "add-character",
            Job::CharacterMod { .. } => "character-mod",
            Job::Verify { .. } => "verify",
            Job::List => "list",
            Job::Extract { .. } => "extract",
            Job::Explain(_) => "explain",
        }
    }
}

/// What came back from a job's thread.
struct Done {
    item: u64,
    /// More lines of a job still going (dev mode's watch): added to the ones shown, the item still busy.
    more: bool,
    lines: Vec<Line>,
    /// A path the job made (a .luma, a mod folder): shown beside the result, and added to the list when it is a mod.
    made: Option<PathBuf>,
}

/// Runs a job on `path` and says what came of it; every word is the library's or a row's.
fn run(job: Job, path: &Path, inputs: &Inputs, installed: Option<&Installed>, more: &dyn Fn(Vec<Line>)) -> anyhow::Result<(Vec<Line>, Option<PathBuf>)> {
    let root = path.is_dir().then_some(path);
    Ok(match job {
        Job::Check => (report(&jobs::validate(path, inputs)?, root), None),
        Job::Build => {
            let (mut lines, ok) = build_lines(path, installed)?;
            if ok {
                lines.push(Line::words(say!("LumaLink.Window.Built"), GREEN));
            }
            (lines, None)
        }
        Job::Pack { to } => {
            let (mut lines, ok) = build_if_changed(path, installed)?;
            if !ok {
                return Ok((lines, None));
            }
            let (r, outcome) = jobs::pack(path, to, inputs)?;
            let (packed, made) = pack_lines(&r, outcome, root);
            lines.extend(packed);
            (lines, made)
        }
        Job::Install { play } => {
            let Some(found) = installed else { return Ok((vec![Line::words(say!("LumaLink.NoInstalled"), RED)], None)) };
            let (mut lines, ok) = build_if_changed(path, Some(found))?;
            if !ok {
                return Ok((lines, None));
            }
            let (id, mods) = match jobs::install(path, inputs, found, &[])? {
                Install::Imported { luma, id, mods } => {
                    lines.push(Line::words(say!("LumaLink.Run.Imported", file = luma.display()), GREEN));
                    (id, mods)
                }
                Install::NotImported { luma, why } => {
                    lines.push(Line::words(say!("LumaLink.Run.NotImported", file = luma.display(), why = why), RED));
                    return Ok((lines, None));
                }
                Install::NotPacked(r, outcome) => return Ok((pack_lines(&r, outcome, root).0, None)),
                Install::Refused(r) => return Ok((report(&r, root), None)),
            };
            if play {
                // A mod folder with code plays in dev mode: each change of its source is built and reloaded in the game
                // (lumalink_core::dev) until the game is closed.
                let coded = path.is_dir() && jobs::has_source(path);
                let dev = if coded { id } else { None };
                if coded && dev.is_none() {
                    lines.push(Line::words(say!("LumaLink.Run.NoDevId"), AMBER));
                }
                let args: Vec<std::ffi::OsString> = dev.iter().flat_map(|id| ["--mod-dev".into(), id.into()]).collect();
                let pak = inputs.game.clone().unwrap_or_else(|| found.pak.clone());
                let (program, game) = jobs::start_game(found, &pak, &args)?;
                lines.push(Line::words(say!("LumaLink.Run.Started", program = program.display()), GREEN));
                if let Some(id) = dev {
                    lines.push(Line::words(say!("LumaLink.Run.DevMode", id = id), SOFT));
                    more(std::mem::take(&mut lines));
                    watch(path, &id, &mods, game, inputs, found, more)?;
                }
            }
            (lines, None)
        }
        Job::Convert { folder, accept_losses } => {
            let given = lumalink_core::convert::Given { name: None, authors: vec![], accept_losses };
            match jobs::convert(path, &folder, &given, inputs)? {
                Convert::Losses(losses) => {
                    let mut lines: Vec<Line> = losses.0.iter().map(|(p, why)| Line::words(say!("LumaLink.Convert.LeftOut", path = p, why = why), AMBER)).collect();
                    lines.push(Line::words(losses.to_string(), RED));
                    (lines, None)
                }
                Convert::Done { converted, report: r } => {
                    let mut lines: Vec<Line> = converted
                        .notes
                        .iter()
                        .map(|n| match n.level {
                            lumalink_core::plan::Level::Info => Line::words(format!("{}: {}", say!("LumaLink.Level.Note"), n.text), SOFT),
                            _ => Line::words(format!("{}: {}", say!("LumaLink.Level.Error"), n.text), RED),
                        })
                        .collect();
                    lines.extend(converted.left_out.iter().map(|(p, why)| Line::words(say!("LumaLink.Convert.LeftOut", path = p, why = why), AMBER)));
                    lines.push(Line::words(say!("LumaLink.Convert.Done", folder = folder.display(), files = converted.files, textures = converted.textures), GREEN));
                    lines.extend(report(&r, Some(&folder)));
                    (lines, Some(folder))
                }
            }
        }
        Job::MakeLuma { luma, accept_losses } => {
            let work = tempfile::tempdir()?;
            let stem = luma.file_stem().map_or_else(|| "mod".into(), |s| s.to_string_lossy().into_owned());
            let folder = work.path().join(&stem);
            let given = lumalink_core::convert::Given { name: None, authors: vec![], accept_losses };
            match jobs::convert(path, &folder, &given, inputs)? {
                Convert::Losses(losses) => {
                    let mut lines: Vec<Line> = losses.0.iter().map(|(p, why)| Line::words(say!("LumaLink.Convert.LeftOut", path = p, why = why), AMBER)).collect();
                    lines.push(Line::words(say!("LumaLink.Window.Losses", switch = say!("LumaLink.Window.AcceptLosses")), RED));
                    (lines, None)
                }
                Convert::Done { converted, .. } => {
                    let mut lines: Vec<Line> = converted
                        .notes
                        .iter()
                        .filter(|n| n.level != lumalink_core::plan::Level::Info)
                        .map(|n| Line::words(format!("{}: {}", say!("LumaLink.Level.Error"), n.text), RED))
                        .collect();
                    lines.extend(converted.left_out.iter().map(|(p, why)| Line::words(say!("LumaLink.Convert.LeftOut", path = p, why = why), AMBER)));
                    let (r, outcome) = jobs::pack(&folder, Some(luma.clone()), inputs)?;
                    let (packed, made) = pack_lines(&r, outcome, None);
                    lines.extend(packed);
                    if made.is_some() {
                        lines.push(Line::words(say!("LumaLink.Window.LumaReady", file = luma.display()), GREEN));
                    }
                    (lines, made)
                }
            }
        }
        Job::CharacterMod { replaces } => {
            let Some(pak) = inputs.game.clone() else { return Ok((vec![Line::words(say!("LumaLink.NoInstalled"), RED)], None)) };
            let stem = path.file_stem().map_or_else(|| "character".into(), |s| s.to_string_lossy().into_owned());
            let parent = path.parent().unwrap_or(Path::new("."));
            let folder = (1..100).map(|n| parent.join(if n == 1 { format!("{stem} mod") } else { format!("{stem} mod {n}") })).find(|f| !f.exists()).unwrap_or_default();
            let author = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| say!("LumaLink.Window.Me"));
            let who = if replaces == "luigi" { say!("LumaLink.Window.Luigi") } else { say!("LumaLink.Window.Mario") };
            let summary = say!("LumaLink.Window.CharacterSummary", name = stem, who = who);
            match jobs::new_mod(&folder, &mod_id(&stem), &stem, &summary, &[author])? {
                NewMod::Refused(r) => (report(&r, None), None),
                NewMod::Made(folder) => {
                    let made = jobs::add_character(path, &folder, &replaces, None, &[], &pak, inputs)?;
                    let mut lines: Vec<Line> = made.lines.iter().map(|l| Line::words(l.clone(), INK)).collect();
                    lines.extend(report(&made.report, Some(&folder)));
                    lines.push(Line::words(say!("LumaLink.Window.ModMade", folder = folder.display()), GREEN));
                    (lines, Some(folder))
                }
            }
        }
        Job::AddCharacter { folder, replaces } => {
            let Some(pak) = inputs.game.clone() else { return Ok((vec![Line::words(say!("LumaLink.NoInstalled"), RED)], None)) };
            let made = jobs::add_character(path, &folder, &replaces, None, &[], &pak, inputs)?;
            let mut lines: Vec<Line> = made.lines.iter().map(|l| Line::words(l.clone(), INK)).collect();
            lines.push(Line::words(say!("LumaLink.Character.Written", file = made.file.display(), report = made.report_file.display()), GREEN));
            lines.extend(report(&made.report, Some(&folder)));
            (lines, Some(folder))
        }
        Job::Verify { folder } => match jobs::verify(path, &folder)? {
            Ok(bytes) => (vec![Line::words(say!("LumaLink.Verify.Same", luma = path.display(), folder = folder.display(), bytes = bytes), GREEN)], None),
            Err(differences) => {
                let mut lines: Vec<Line> = differences
                    .into_iter()
                    .map(|d| match d {
                        Difference::OnlyInLuma(entry) => Line::words(say!("LumaLink.Verify.OnlyIn", place = path.display(), entry = entry), AMBER),
                        Difference::OnlyInFolder(entry) => Line::words(say!("LumaLink.Verify.OnlyIn", place = folder.display(), entry = entry), AMBER),
                        Difference::Differs(entry) => Line::words(say!("LumaLink.Verify.Differs", entry = entry), AMBER),
                    })
                    .collect();
                lines.push(Line::words(say!("LumaLink.Verify.Not", luma = path.display(), folder = folder.display()), RED));
                (lines, None)
            }
        },
        Job::List => match jobs::list_luma(path)? {
            LumaList::Entries { entries, damaged } => {
                let (stored, deflate) = (say!("LumaLink.Method.Stored"), say!("LumaLink.Method.Deflate"));
                let mut lines: Vec<Line> = entries
                    .iter()
                    .map(|e| Line::words(format!("{:>10} {:>10}  {}  {}", e.unpacked, e.packed, if e.stored { &stored } else { &deflate }, e.name), INK))
                    .collect();
                lines.extend(damaged.iter().map(|f| finding(f, None)));
                (lines, None)
            }
            LumaList::Unreadable(r) => (report(&r, None), None),
        },
        Job::Extract { into } => match jobs::extract_luma(path, &[], &into)? {
            Ok(written) => (vec![Line::words(say!("LumaLink.Extracted", folder = into.display(), count = written.len()), GREEN)], Some(into)),
            Err(container) => {
                let mut lines: Vec<Line> = container.iter().map(|f| finding(f, None)).collect();
                lines.push(Line::words(say!("LumaLink.NotExtracted"), RED));
                (lines, None)
            }
        },
        Job::Explain(code) => {
            let (number, level, template) = jobs::explain(&code.to_string())?;
            let kind = if level == Level::Error { say!("LumaLink.Explain.Error") } else { say!("LumaLink.Explain.Note") };
            (vec![Line::words(format!("L{number} ({kind})"), BLUE_INK), Line::words(template, INK)], None)
        }
    })
}

/// A mod id from a name: its letters and digits in lowercase, others as `-`, after `prefix`; at least 3 long,
/// starting with a letter, and never a name Windows keeps (`con`, `com1`: `-mod` after it), so the build takes it.
fn id_from(prefix: &str, name: &str) -> String {
    let mut body = String::new();
    for c in name.chars().map(|c| c.to_ascii_lowercase()) {
        if c.is_ascii_alphanumeric() {
            body.push(c);
        } else if !body.is_empty() && !body.ends_with('-') {
            body.push('-');
        }
    }
    let body = body.trim_start_matches(|c: char| !c.is_ascii_lowercase()).trim_end_matches('-');
    let kept = |b: &str| matches!(b, "con" | "prn" | "aux" | "nul") || (b.len() == 4 && (b.starts_with("com") || b.starts_with("lpt")) && (b'1'..=b'9').contains(&b.as_bytes()[3]));
    let body = match body.len() {
        0 => "mod".to_string(),
        1 | 2 => format!("{body}-mod"),
        _ if prefix.is_empty() && kept(body) => format!("{body}-mod"),
        _ => body.to_string(),
    };
    format!("{prefix}{body}").chars().take(64).collect::<String>().trim_end_matches('-').to_string()
}

/// A character mod's id: `character.` and the model's name.
fn mod_id(name: &str) -> String {
    id_from("character.", name)
}

/// A mod folder's code built (`lumalink build`, every platform) with the installed game's names; its lines and whether
/// it built.
fn build_lines(folder: &Path, installed: Option<&Installed>) -> anyhow::Result<(Vec<Line>, bool)> {
    let Some(names) = installed.and_then(Installed::self_description) else { return Ok((vec![Line::words(say!("LumaLink.NoNames"), RED)], false)) };
    let built = jobs::build_mod(folder, &names, jobs::sdk_folder(None, Some(&names)), None, None, &[], &jobs::NeedsFrom::default())?;
    let mut lines: Vec<Line> = built.lines.iter().map(|l| Line::words(l.text.trim_end().to_string(), if l.error { RED } else { SOFT })).collect();
    if let Some(jobs::BuildFailed::Platforms(why)) = &built.failed {
        lines.push(Line::words(why.clone(), RED));
    }
    Ok((lines, built.failed.is_none()))
}

/// Dev mode in the window: the folder watched until the game is closed, each thing seen sent as lines.
fn watch(path: &Path, id: &str, mods: &Path, game: jobs::Game, inputs: &Inputs, found: &Installed, more: &dyn Fn(Vec<Line>)) -> anyhow::Result<()> {
    use lumalink_core::dev::{Event, Watch};
    let root = Some(path);
    let mut watch = Watch::new(path, id, mods, game);
    let mut build = |folder: &Path| -> anyhow::Result<(Vec<jobs::BuildLine>, bool)> {
        let Some(names) = found.self_description() else {
            return Ok((vec![jobs::BuildLine { text: say!("LumaLink.NoNames"), error: true, to_stderr: true }], false));
        };
        let built = jobs::build_mod(folder, &names, jobs::sdk_folder(None, Some(&names)), None, None, &[], &jobs::NeedsFrom::default())?;
        let mut lines = built.lines;
        if let Some(jobs::BuildFailed::Platforms(why)) = &built.failed {
            lines.push(jobs::BuildLine { text: why.clone(), error: true, to_stderr: true });
        }
        Ok((lines, built.failed.is_none()))
    };
    let refused: std::cell::RefCell<Vec<Line>> = Default::default();
    let mut install = |folder: &Path| -> anyhow::Result<Option<String>> {
        Ok(match jobs::install(folder, inputs, found, &[])? {
            Install::Imported { .. } => None,
            Install::NotImported { why, .. } => Some(why),
            Install::NotPacked(r, outcome) => {
                *refused.borrow_mut() = pack_lines(&r, outcome, root).0;
                Some(say!("LumaLink.Run.NotPacked"))
            }
            Install::Refused(r) => {
                *refused.borrow_mut() = report(&r, root);
                Some(say!("LumaLink.Run.NotPacked"))
            }
        })
    };
    while !watch.ended() {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut lines = vec![];
        for event in watch.poll(&mut build, &mut install)? {
            match event {
                Event::Building => lines.push(Line::words(say!("LumaLink.Run.Building"), SOFT)),
                Event::Built { lines: built, ok } => {
                    lines.extend(built.iter().filter(|l| l.error || !ok).map(|l| Line::words(l.text.trim_end().to_string(), if l.error { RED } else { SOFT })));
                }
                Event::NotInstalled(why) => {
                    lines.append(&mut refused.borrow_mut());
                    lines.push(Line::words(say!("LumaLink.Run.NotReloaded", why = why), RED));
                }
                Event::Asked(n) => lines.push(Line::words(say!("LumaLink.Run.Asked", n = n), SOFT)),
                Event::Reloaded(answer) if answer.ok => lines.push(Line::words(say!("LumaLink.Run.Reloaded"), GREEN)),
                Event::Reloaded(answer) => lines.push(Line::words(say!("LumaLink.Run.NotReloaded", why = answer.why), RED)),
                Event::ContentChanged => lines.push(Line::words(say!("LumaLink.Run.ContentLater"), AMBER)),
                Event::GameEnded => lines.push(Line::words(say!("LumaLink.Run.GameEnded"), SOFT)),
            }
        }
        if !lines.is_empty() {
            more(lines);
        }
    }
    Ok(())
}

/// Builds a mod folder's code when its last build was made from anything else (`jobs::code_out_of_date`: its source,
/// LumaLink, the installed game's self-description); nothing for a .luma or a mod without code. Its lines and whether
/// the mod can go on.
fn build_if_changed(path: &Path, installed: Option<&Installed>) -> anyhow::Result<(Vec<Line>, bool)> {
    if !path.is_dir() || !jobs::has_source(path) {
        return Ok((vec![], true));
    }
    // With no self-description, build_lines says so.
    let names = installed.and_then(Installed::self_description);
    if names.as_deref().is_some_and(|n| !jobs::code_out_of_date(path, n, jobs::sdk_folder(None, Some(n)).as_deref(), &jobs::NeedsFrom::default())) {
        return Ok((vec![], true));
    }
    build_lines(path, installed)
}

struct Item {
    /// Never reused, so a job's answer finds its item after others have gone.
    id: u64,
    path: PathBuf,
    kind: Kind,
    busy: bool,
    lines: Vec<Line>,
    made: Option<PathBuf>,
    /// How its last job went, shown on its tile.
    mark: Option<Mark>,
}

/// The New mod form.
#[derive(Default)]
struct NewForm {
    /// The folder the mod's folder is made in.
    place: Option<PathBuf>,
    id: String,
    name: String,
    summary: String,
    authors: String,
    refused: Vec<Line>,
}

/// Settings' Add LumaScript to Visual Studio Code (lumalink_core::vscode).
enum VsCode {
    Idle,
    /// VS Code is installing it on a thread of its own (a few seconds).
    Working(Receiver<Result<Result<(), lumalink_core::vscode::Missing>, String>>),
    /// What came of it, and its colour.
    Said(String, Color32),
}

impl VsCode {
    fn start() -> VsCode {
        let (sender, answer) = channel();
        std::thread::spawn(move || {
            let _ = sender.send(lumalink_core::vscode::install().map_err(|e| format!("{e:#}")));
        });
        VsCode::Working(answer)
    }

    /// The answer, once it came.
    fn poll(&mut self) {
        let VsCode::Working(answer) = self else { return };
        let said = match answer.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Ok(Ok(Ok(()))) => (say!("LumaLink.Window.VsCode.Done"), GREEN),
            Ok(Ok(Err(lumalink_core::vscode::Missing::Code))) => (say!("LumaLink.Window.VsCode.NoCode"), AMBER),
            Ok(Ok(Err(lumalink_core::vscode::Missing::Extension))) => (say!("LumaLink.Window.VsCode.NoExtension"), AMBER),
            Ok(Err(why)) => (say!("LumaLink.Window.VsCode.Failed", why = why), RED),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => (say!("LumaLink.Window.VsCode.Failed", why = "-"), RED),
        };
        *self = VsCode::Said(said.0, said.1);
    }
}

struct App {
    items: Vec<Item>,
    next_id: u64,
    chosen: Option<u64>,
    installed: Option<Installed>,
    /// A game.pak chosen in place of the installed one.
    game: Option<PathBuf>,
    replaces: String,
    accept_losses: bool,
    new_form: Option<NewForm>,
    /// The Add mods choices (files, a folder, a new mod) are open.
    add_menu: bool,
    sender: Sender<Done>,
    receiver: Receiver<Done>,
    /// A newer LumaLink and the person's answer (the banner along the top).
    update: Update,
    /// The settings' "check for updates when LumaLink opens".
    check_for_updates: bool,
    /// Settings' Add LumaScript to Visual Studio Code: its answer while it runs, then what came of it.
    vscode: VsCode,
    /// Home, Settings or the languages (a mod's own screen is `chosen`).
    view: screen::View,
    /// The page of tiles shown, and the wheel turned toward the next.
    sheet: usize,
    /// Tiles on a page in the shape the window had at the last frame.
    per_page: usize,
    wheel: f32,
    /// The height of the chosen mod's card at the last frame (it stands in the middle while it is alone).
    card_height: f32,
}

impl App {
    /// The window on the installed game found (none: a game.pak is chosen), the paths given added.
    fn with(installed: Option<Installed>, paths: impl Iterator<Item = PathBuf>) -> Self {
        let (sender, receiver) = channel();
        let mut app = App {
            items: vec![],
            next_id: 0,
            chosen: None,
            installed,
            game: None,
            replaces: "mario".into(),
            accept_losses: false,
            new_form: None,
            add_menu: false,
            sender,
            receiver,
            update: Update::Quiet,
            check_for_updates: lumalink_core::update::Settings::load().check_for_updates,
            vscode: VsCode::Idle,
            view: screen::View::Home,
            sheet: 0,
            per_page: 12,
            wheel: 0.0,
            card_height: 0.0,
        };
        let added: Vec<u64> = paths.map(|p| app.add(p)).collect();
        // One path given opens its screen.
        if let [one] = added[..] {
            app.chosen = Some(one);
        }
        app
    }

    /// The validator's inputs: the game.pak chosen, else the installed game's, with its self-description.
    fn inputs(&self) -> Inputs {
        Inputs { names: None, platforms: vec![], game: self.game.clone(), no_installed: false, mods: None }.resolved(self.installed.as_ref())
    }

    /// The mods of jobs the last LumaLink did not finish (the system ended it: lumalink_core::running), each added
    /// with that said.
    fn add_stopped(&mut self, left_over: Vec<(String, PathBuf)>) {
        for (_, path) in left_over {
            let id = self.add(path);
            if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                item.lines = vec![Line::words(say!("LumaLink.Window.StoppedLastTime"), RED)];
            }
        }
    }

    /// Adds a path (once); its id.
    fn add(&mut self, path: PathBuf) -> u64 {
        if let Some(item) = self.items.iter().find(|i| i.path == path) {
            return item.id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Item { id, kind: Kind::of(&path), path, busy: false, lines: vec![], made: None, mark: None });
        id
    }

    fn start(&mut self, id: u64, job: Job) {
        let inputs = self.inputs();
        let Some(item) = self.items.iter_mut().find(|i| i.id == id) else { return };
        if item.busy {
            return;
        }
        item.busy = true;
        item.lines = vec![Line::words(say!("LumaLink.Window.Working"), SOFT)];
        let (path, installed, sender) = (item.path.clone(), self.installed.clone(), self.sender.clone());
        std::thread::spawn(move || {
            // A job that panics is said as one, never silently lost; one the system ends with LumaLink (memory ran
            // out) is said at the next start, from its job file.
            let _running = lumalink_core::running::Running::start(lumalink_core::update::data_folder().as_deref(), id, job.name(), &path);
            let more = |lines: Vec<Line>| {
                let _ = sender.send(Done { item: id, more: true, lines, made: None });
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job, &path, &inputs, installed.as_ref(), &more)));
            let (lines, made) = match result {
                Ok(Ok(done)) => done,
                Ok(Err(error)) => (vec![Line::words(format!("{error:#}"), RED)], None),
                Err(_) => (vec![Line::words(say!("LumaLink.Window.Crashed"), RED)], None),
            };
            let _ = sender.send(Done { item: id, more: false, lines, made });
        });
    }

    fn take_done(&mut self) {
        while let Ok(mut done) = self.receiver.try_recv() {
            if done.more {
                if let Some(item) = self.items.iter_mut().find(|i| i.id == done.item) {
                    // The first lines replace "Working"; later ones follow them, the last 200 kept.
                    if item.lines.len() == 1 && item.lines[0].text == say!("LumaLink.Window.Working") {
                        item.lines.clear();
                    }
                    item.lines.append(&mut done.lines);
                    let over = item.lines.len().saturating_sub(200);
                    item.lines.drain(..over);
                    item.mark = screen::mark_of(&item.lines);
                }
                continue;
            }
            // A job that ended after sending more keeps what it sent before its last lines.
            if let Some(item) = self.items.iter_mut().find(|i| i.id == done.item && i.busy) {
                if !(item.lines.len() == 1 && item.lines[0].text == say!("LumaLink.Window.Working")) && !done.lines.is_empty() {
                    let mut kept = std::mem::take(&mut item.lines);
                    kept.append(&mut done.lines);
                    done.lines = kept;
                } else if !(item.lines.len() == 1 && item.lines[0].text == say!("LumaLink.Window.Working")) {
                    done.lines = std::mem::take(&mut item.lines);
                }
            }
            let made = done.made.clone();
            if let Some(item) = self.items.iter_mut().find(|i| i.id == done.item) {
                item.busy = false;
                item.mark = screen::mark_of(&done.lines);
                item.lines = done.lines;
                item.made = done.made;
            }
            // A mod folder a job made is a mod to work on next: a tile of its own.
            if let Some(made) = made.filter(|m| matches!(Kind::of(m), Kind::Folder | Kind::Luma)) {
                self.add(made);
            }
        }
    }
}

/// The file in the system's file browser.
fn show_file(file: &Path) {
    let result = if cfg!(windows) {
        std::process::Command::new("explorer").arg("/select,").arg(file).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(file).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(file.parent().unwrap_or(file)).spawn()
    };
    if let Err(error) = result {
        eprintln!("{error}");
    }
}

/// The window as eframe runs it.
struct Window {
    app: App,
    #[cfg(debug_assertions)]
    shot: Option<shot::Shot>,
}

impl eframe::App for Window {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.ui(ctx);
        #[cfg(debug_assertions)]
        if let Some(shot) = self.shot.as_mut() {
            shot.frame(&mut self.app, ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.95, 0.96, 0.97, 1.0]
    }
}

fn main() -> eframe::Result<()> {
    text::set_language(words::language());
    #[cfg(debug_assertions)]
    let size = shot::size().unwrap_or([1024.0, 700.0]);
    #[cfg(not(debug_assertions))]
    let size = [1024.0, 700.0];
    let viewport = egui::ViewportBuilder::default().with_title("LumaLink").with_app_id(look::APP_ID).with_inner_size(size).with_min_inner_size([480.0, 420.0]).with_drag_and_drop(true).with_icon(look::icon());
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "LumaLink",
        options,
        Box::new(|creation| {
            look::setup(&creation.egui_ctx);
            // Paths given to the program (a file dropped on its icon, "Open with") are added as dropped ones are.
            let mut app = App::with(Installed::find(), std::env::args_os().skip(1).map(PathBuf::from));
            app.add_stopped(lumalink_core::running::left_over(lumalink_core::update::data_folder().as_deref()));
            app.update = Update::at_start();
            Ok(Box::new(Window {
                app,
                #[cfg(debug_assertions)]
                shot: shot::asked(),
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    //! The window as a person uses it, with no screen: egui's frames with AccessKit on (the window as a
    //! screen reader reads it), a folder and a .luma dropped on it, its buttons pressed by their words, and what it says
    //! read back from that tree, each answer checked against the library's own for the same job.
    use super::*;
    use eframe::egui::accesskit::{Action, ActionRequest, Node, NodeId, Role};

    struct Screen {
        ctx: egui::Context,
        app: App,
        nodes: Vec<(NodeId, Node)>,
        events: Vec<egui::Event>,
        dropped: Vec<egui::DroppedFile>,
    }

    impl Screen {
        fn new(app: App) -> Screen {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            look::setup(&ctx);
            let mut screen = Screen { ctx, app, nodes: vec![], events: vec![], dropped: vec![] };
            screen.frame();
            screen
        }

        /// One frame of the window at its first size, with what was dropped and pressed since the last.
        fn frame(&mut self) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 700.0))),
                events: std::mem::take(&mut self.events),
                dropped_files: std::mem::take(&mut self.dropped),
                ..Default::default()
            };
            let app = &mut self.app;
            let out = self.ctx.run(input, |ctx| app.ui(ctx));
            if let Some(update) = out.platform_output.accesskit_update {
                self.nodes = update.nodes;
            }
        }

        /// Every word the window shows: its controls' names and its labels' text.
        fn words(&self) -> Vec<String> {
            self.nodes.iter().filter_map(|(_, n)| n.label().or(n.value()).map(str::to_string)).collect()
        }

        fn shows(&self, text: &str) -> bool {
            self.words().iter().any(|w| w.contains(text))
        }

        fn drop_on(&mut self, path: &Path) {
            self.dropped.push(egui::DroppedFile { path: Some(path.to_path_buf()), ..Default::default() });
            self.frame();
        }

        /// The first enabled control of `role` whose name is `name` (any name for None), clicked as a screen reader
        /// clicks it.
        fn press(&mut self, role: Role, name: Option<&str>) {
            let Some((id, _)) = self.nodes.iter().find(|(_, n)| n.role() == role && !n.is_disabled() && name.is_none_or(|w| n.label() == Some(w))) else {
                panic!("no {role:?} {name:?} to press in {:?}", self.words())
            };
            self.events.push(egui::Event::AccessKitActionRequest(ActionRequest { action: Action::Click, target: *id, data: None }));
            // The press is taken in this frame; what it changed is drawn in the next.
            self.frame();
            self.frame();
        }

        fn button(&mut self, row: &str) {
            self.press(Role::Button, Some(&text::say(row, &[])));
        }

        /// Frames until the window shows `text` (a job runs on a thread of its own), for at most a minute.
        fn wait_for(&mut self, text: &str) {
            for _ in 0..1200 {
                self.frame();
                if self.shows(text) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("the window never showed {text:?}: {:?}", self.words());
        }
    }

    /// A job the system ended with the last LumaLink (its job file left): the next window shows its mod with that said
    /// (M-165).
    #[test]
    fn a_job_the_last_window_did_not_finish_is_said() {
        text::set_language("UsEnglish");
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("cut");
        assert!(matches!(jobs::new_mod(&folder, "test.cut", "Cut", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        let running = dir.path().join(lumalink_core::running::RUNNING_FOLDER);
        std::fs::create_dir_all(&running).unwrap();
        std::fs::write(running.join("4242-0.txt"), format!("pack\n{}", folder.display())).unwrap();
        let mut app = App::with(None, std::iter::empty());
        app.add_stopped(lumalink_core::running::left_over(Some(dir.path())));
        let mut s = Screen::new(app);
        s.drop_on(&folder);
        assert!(s.shows(&say!("LumaLink.Window.StoppedLastTime")), "{:?}", s.words());
    }

    #[test]
    fn a_person_works_on_a_mod_in_the_window() {
        text::set_language("UsEnglish");
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("round");
        assert!(matches!(jobs::new_mod(&folder, "test.round", "Round", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        let mut s = Screen::new(App::with(None, std::iter::empty()));
        // Nothing yet: the home screen says what to do, with Settings and Add mods on its bar.
        assert!(s.shows(&say!("LumaLink.Window.Drop")), "{:?}", s.words());
        assert!(s.shows(&say!("LumaLink.Window.Settings")) && s.shows(&say!("LumaLink.Window.Add")), "{:?}", s.words());

        // The folder dropped: its own screen, its kind and its jobs.
        s.drop_on(&folder);
        assert!(s.shows(&folder.display().to_string()) && s.shows(&say!("LumaLink.Window.Kind.Folder")), "{:?}", s.words());
        for row in ["LumaLink.Window.Check", "LumaLink.Window.Pack", "LumaLink.Window.Install", "LumaLink.Window.Play", "LumaLink.Window.Menu"] {
            assert!(s.shows(&text::say(row, &[])), "{row}: {:?}", s.words());
        }
        // Check: the library's report on the window's own inputs, its count line shown.
        s.button("LumaLink.Window.Check");
        let expected = jobs::summary_line(&jobs::validate(&folder, &s.app.inputs()).unwrap());
        s.wait_for(&expected);

        // Pack: the .luma written beside the folder, named in the window.
        s.button("LumaLink.Window.Pack");
        // Done: the .luma can be opened on its own screen (the button's own words name a .luma too).
        s.wait_for(&say!("LumaLink.Window.OpenMade"));
        let luma = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok().map(|e| e.path())).find(|p| p.extension().is_some_and(|x| x == "luma")).expect("a .luma beside the folder");
        assert!(s.shows(&luma.display().to_string()), "{:?}", s.words());

        // Install with no game found: refused by name.
        s.button("LumaLink.Window.Install");
        s.wait_for(&say!("LumaLink.NoInstalled"));

        // Menu: home, the folder's tile there by its name.
        s.button("LumaLink.Window.Menu");
        assert!(s.shows("round") && !s.shows(&say!("LumaLink.Window.Kind.Folder")), "{:?}", s.words());

        // The .luma dropped: its own screen; List gives its entries.
        s.drop_on(&luma);
        assert!(s.shows(&say!("LumaLink.Window.Kind.Luma")), "{:?}", s.words());
        s.button("LumaLink.Window.List");
        s.wait_for("luma.json");
        let luma_name = luma.file_name().unwrap().to_string_lossy().into_owned();

        // Another language from Settings: Settings, Language, German; the words are German at the next frame, the
        // list goes back to Settings, and Back to home.
        let english = say!("LumaLink.Window.List");
        s.button("LumaLink.Window.Menu");
        s.button("LumaLink.Window.Settings");
        s.button("LumaLink.Window.Language");
        assert!(s.shows(words::name("EuGerman")), "{:?}", s.words());
        s.press(Role::Button, Some(words::name("EuGerman")));
        assert_eq!(text::language(), "EuGerman");
        assert!(s.shows(&say!("LumaLink.Window.Language")) && !s.shows(words::name("EuDutch")), "{:?}", s.words());
        s.button("LumaLink.Window.Back");
        // The .luma's tile, by its name: its jobs in German.
        s.press(Role::Button, Some(&luma_name));
        let german = say!("LumaLink.Window.List");
        assert_ne!(english, german);
        assert!(s.shows(&german) && !s.shows(&english), "{:?}", s.words());
        // Back to English the same way.
        s.button("LumaLink.Window.Menu");
        s.button("LumaLink.Window.Settings");
        s.button("LumaLink.Window.Language");
        s.press(Role::Button, Some(words::name("UsEnglish")));
        assert_eq!(text::language(), "UsEnglish");
        s.button("LumaLink.Window.Back");

        // Both taken off the list from their own screens: home says what to do again.
        for name in [luma_name.as_str(), "round"] {
            s.press(Role::Button, Some(name));
            s.button("LumaLink.Window.Remove");
            assert!(!s.shows(name), "{name}: {:?}", s.words());
        }
        assert!(s.shows(&say!("LumaLink.Window.Drop")), "{:?}", s.words());
    }

    /// N-4061: the window's new mod is made in one folder of the place chosen, whatever its name: a name with characters
    /// no system's file names take (a colon, a slash), dots or spaces at its end, or a Windows device's name. Before, the
    /// folder was the name as typed: "Galaxy: Remix" failed with the system's words, "A/B" made two folders.
    /// The id the New mod form makes from a name is one the build takes (build.rs id_ok, L206): a name Windows
    /// keeps (`Con`, `COM1`) gets `-mod` after it.
    #[test]
    fn a_new_mods_id_from_its_name_is_always_one_the_build_takes() {
        assert_eq!(id_from("", "Con"), "con-mod");
        assert_eq!(id_from("", "COM1"), "com1-mod");
        assert_eq!(id_from("", "Lpt9 Port"), "lpt9-port");
        assert_eq!(id_from("", "Coin Rush"), "coin-rush");
        assert_eq!(id_from("character.", "Con"), "character.con");
        for name in ["Con", "aux", "nul", "com1", "LPT3", "x", "", "..", "a.b."] {
            let id = id_from("", name);
            let first = id.split('.').next().unwrap_or("");
            let kept = matches!(first, "con" | "prn" | "aux" | "nul") || (first.len() == 4 && (first.starts_with("com") || first.starts_with("lpt")) && first.as_bytes()[3].is_ascii_digit());
            assert!(!kept && id.len() >= 3 && !id.contains("..") && !id.ends_with('.'), "{name:?} -> {id:?}");
        }
    }

    #[test]
    fn a_new_mods_folder_is_one_folder_whatever_its_name() {
        let cases = [
            ("Moon jump", "Moon jump"),
            ("Galaxy: Remix", "Galaxy Remix"),
            (r"A/B\C", "A B C"),
            ("../up", ".. up"),
            ("What?  *Really*", "What Really"),
            ("Ends in dots...", "Ends in dots"),
            ("con", "con mod"),
            ("Com1.galaxy", "Com1.galaxy mod"),
            ("COM10", "COM10"),
            ("..", "mod"),
            ("\t", "mod"),
            ("スーパーマリオ", "スーパーマリオ"),
        ];
        for (name, folder) in cases {
            assert_eq!(screen::folder_name(name), folder, "{name:?}");
            let place = tempfile::tempdir().unwrap();
            let made = place.path().join(screen::folder_name(name));
            std::fs::create_dir(&made).unwrap_or_else(|e| panic!("{name:?}: {e}"));
            assert_eq!(made.parent(), Some(place.path()), "{name:?}");
            assert_eq!(std::fs::read_dir(place.path()).unwrap().count(), 1, "{name:?}");
        }
    }
}
