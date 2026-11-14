//! The LumaLink window: every job the command line does, on what is dropped on it or opened. Along the top,
//! the game mods are checked against (the installed one, found as the command line finds it, or a game.pak chosen)
//! and the language; on the left, the mods, published mods and models dropped; on the right, the chosen one's jobs as
//! buttons and what came of the last, each finding with its file and line. The jobs are the library's
//! (`lumalink_core::jobs`); the window only shows them, each on a thread of its own so the window never waits.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sky;
mod words;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText, Stroke, Vec2};
use luma_validate::{Finding, Level, Report};
use lumalink_core::installed::Installed;
use lumalink_core::jobs::{self, Convert, Difference, Inputs, Install, LumaList, NewMod, PackOutcome};
use lumalink_core::{say, text};
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
    Pack,
    Install { play: bool },
    Convert { folder: PathBuf, accept_losses: bool },
    AddCharacter { folder: PathBuf, replaces: String },
    Verify { folder: PathBuf },
    List,
    Extract { into: PathBuf },
    Explain(u16),
}

/// What came back from a job's thread.
struct Done {
    item: u64,
    lines: Vec<Line>,
    /// A path the job made (a .luma, a mod folder): shown beside the result, and added to the list when it is a mod.
    made: Option<PathBuf>,
}

/// Runs a job on `path` and says what came of it; every word is the library's or a row's.
fn run(job: Job, path: &Path, inputs: &Inputs, installed: Option<&Installed>) -> anyhow::Result<(Vec<Line>, Option<PathBuf>)> {
    let root = path.is_dir().then_some(path);
    Ok(match job {
        Job::Check => (report(&jobs::validate(path, inputs)?, root), None),
        Job::Build => (vec![Line::words(jobs::not_built("build").unwrap_or_default(), RED)], None),
        Job::Pack => {
            let (r, outcome) = jobs::pack(path, None, inputs)?;
            pack_lines(&r, outcome, root)
        }
        Job::Install { play } => {
            let Some(found) = installed else { return Ok((vec![Line::words(say!("LumaLink.NoInstalled"), RED)], None)) };
            let mut lines = Vec::new();
            match jobs::install(path, inputs, found, &[])? {
                Install::Imported { luma } => lines.push(Line::words(say!("LumaLink.Run.Imported", file = luma.display()), GREEN)),
                Install::NotImported { luma, why } => {
                    lines.push(Line::words(say!("LumaLink.Run.NotImported", file = luma.display(), why = why), RED));
                    return Ok((lines, None));
                }
                Install::NotPacked(r, outcome) => return Ok((pack_lines(&r, outcome, root).0, None)),
                Install::Refused(r) => return Ok((report(&r, root), None)),
            }
            if play {
                let pak = inputs.game.clone().unwrap_or_else(|| found.pak.clone());
                let program = jobs::start(found, &pak, &[])?;
                lines.push(Line::words(say!("LumaLink.Run.Started", program = program.display()), GREEN));
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

struct Item {
    /// Never reused, so a job's answer finds its item after others have gone.
    id: u64,
    path: PathBuf,
    kind: Kind,
    busy: bool,
    lines: Vec<Line>,
    made: Option<PathBuf>,
}

/// The New mod form.
#[derive(Default)]
struct NewForm {
    folder: Option<PathBuf>,
    id: String,
    name: String,
    summary: String,
    authors: String,
    refused: Vec<Line>,
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
    sender: Sender<Done>,
    receiver: Receiver<Done>,
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
            sender,
            receiver,
        };
        for path in paths {
            app.add(path);
        }
        app
    }

    /// The validator's inputs: the game.pak chosen, else the installed game's, with its self-description.
    fn inputs(&self) -> Inputs {
        Inputs { names: None, platforms: vec![], game: self.game.clone(), no_installed: false, mods: None }.resolved(self.installed.as_ref())
    }

    /// Adds a path (once) and chooses it; its id.
    fn add(&mut self, path: PathBuf) -> u64 {
        if let Some(item) = self.items.iter().find(|i| i.path == path) {
            self.chosen = Some(item.id);
            return item.id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Item { id, kind: Kind::of(&path), path, busy: false, lines: vec![], made: None });
        self.chosen = Some(id);
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
            // A job that panics is said as one, never silently lost.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job, &path, &inputs, installed.as_ref())));
            let (lines, made) = match result {
                Ok(Ok(done)) => done,
                Ok(Err(error)) => (vec![Line::words(format!("{error:#}"), RED)], None),
                Err(_) => (vec![Line::words(say!("LumaLink.Window.Crashed"), RED)], None),
            };
            let _ = sender.send(Done { item: id, lines, made });
        });
    }

    fn take_done(&mut self) {
        while let Ok(done) = self.receiver.try_recv() {
            let made = done.made.clone();
            if let Some(item) = self.items.iter_mut().find(|i| i.id == done.item) {
                item.busy = false;
                item.lines = done.lines;
                item.made = done.made;
            }
            // A mod folder a job made is a mod to work on next: listed, the chosen one kept.
            if let Some(folder) = made.filter(|m| Kind::of(m) == Kind::Folder) {
                let keep = self.chosen;
                self.add(folder);
                self.chosen = keep;
            }
        }
    }

    fn top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            match self.game.clone().or_else(|| self.installed.as_ref().map(|f| f.pak.clone())) {
                Some(pak) => ui.label(RichText::new(say!("LumaLink.Window.Game", path = pak.display())).color(INK)),
                None => ui.label(RichText::new(say!("LumaLink.Window.NoGame")).color(AMBER)),
            };
            if ui.button(say!("LumaLink.Window.ChooseGame")).clicked() {
                if let Some(pak) = rfd::FileDialog::new().add_filter("game.pak", &["pak"]).pick_file() {
                    self.game = Some(pak);
                }
            }
            ui.separator();
            let mut chosen = text::language();
            egui::ComboBox::from_id_salt("language").selected_text(words::name(chosen)).show_ui(ui, |ui| {
                for l in luma_validate::LANGUAGES {
                    ui.selectable_value(&mut chosen, l, words::name(l));
                }
            });
            if chosen != text::language() {
                text::set_language(chosen);
            }
        });
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button(say!("LumaLink.Window.OpenFolder")).clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.add(folder);
                }
            }
            if ui.button(say!("LumaLink.Window.OpenFile")).clicked() {
                for file in rfd::FileDialog::new().pick_files().unwrap_or_default() {
                    self.add(file);
                }
            }
            if ui.button(say!("LumaLink.Window.New")).clicked() {
                self.new_form = Some(NewForm::default());
            }
        });
        ui.add_space(6.0);
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("items").show(ui, |ui| {
            for item in &self.items {
                ui.horizontal(|ui| {
                    let name = item.path.file_name().map_or_else(|| item.path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    let label = if item.busy { format!("{name} …") } else { name };
                    if ui.selectable_label(self.chosen == Some(item.id), label).on_hover_text(item.path.display().to_string()).clicked() {
                        self.chosen = Some(item.id);
                    }
                    if !item.busy && ui.small_button("×").on_hover_text(say!("LumaLink.Window.Remove")).clicked() {
                        remove = Some(item.id);
                    }
                });
            }
        });
        if let Some(id) = remove {
            self.items.retain(|i| i.id != id);
            if self.chosen == Some(id) {
                self.chosen = self.items.last().map(|i| i.id);
            }
        }
    }

    /// The chosen item's jobs as buttons, then what came of the last.
    fn page(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.chosen.and_then(|id| self.items.iter().position(|i| i.id == id)) else {
            ui.centered_and_justified(|ui| ui.label(RichText::new(say!("LumaLink.Window.Drop")).color(SOFT).size(18.0)));
            return;
        };
        let (id, path, kind, busy) = {
            let item = &self.items[index];
            (item.id, item.path.clone(), item.kind, item.busy)
        };
        ui.label(RichText::new(path.display().to_string()).strong().color(INK));
        ui.label(RichText::new(kind.label()).color(SOFT));
        ui.add_space(6.0);
        // (item, job): verifying a folder runs on the .luma chosen, which joins the list.
        let mut job: Option<(u64, Job)> = None;
        let mut add = None;
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                let button = |ui: &mut egui::Ui, row: &str| ui.button(text::say(row, &[])).clicked();
                match kind {
                    Kind::Folder => {
                        if button(ui, "LumaLink.Window.Check") {
                            job = Some((id, Job::Check));
                        }
                        if button(ui, "LumaLink.Window.Build") {
                            job = Some((id, Job::Build));
                        }
                        if button(ui, "LumaLink.Window.Pack") {
                            job = Some((id, Job::Pack));
                        }
                        if button(ui, "LumaLink.Window.Install") {
                            job = Some((id, Job::Install { play: false }));
                        }
                        if button(ui, "LumaLink.Window.Play") {
                            job = Some((id, Job::Install { play: true }));
                        }
                        if button(ui, "LumaLink.Window.Verify") {
                            if let Some(luma) = rfd::FileDialog::new().add_filter("luma", &["luma"]).pick_file() {
                                add = Some((luma, Job::Verify { folder: path.clone() }));
                            }
                        }
                    }
                    Kind::Luma => {
                        if button(ui, "LumaLink.Window.Check") {
                            job = Some((id, Job::Check));
                        }
                        if button(ui, "LumaLink.Window.Install") {
                            job = Some((id, Job::Install { play: false }));
                        }
                        if button(ui, "LumaLink.Window.Play") {
                            job = Some((id, Job::Install { play: true }));
                        }
                        if button(ui, "LumaLink.Window.List") {
                            job = Some((id, Job::List));
                        }
                        if button(ui, "LumaLink.Window.Extract") {
                            if let Some(into) = rfd::FileDialog::new().pick_folder() {
                                job = Some((id, Job::Extract { into }));
                            }
                        }
                        if button(ui, "LumaLink.Window.Verify") {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                job = Some((id, Job::Verify { folder }));
                            }
                        }
                    }
                    Kind::Model => {
                        ui.label(say!("LumaLink.Window.Replaces"));
                        egui::ComboBox::from_id_salt("replaces").selected_text(self.replaces.clone()).show_ui(ui, |ui| {
                            for who in ["mario", "luigi"] {
                                ui.selectable_value(&mut self.replaces, who.to_string(), who);
                            }
                        });
                        if button(ui, "LumaLink.Window.AddCharacter") {
                            if let Some(folder) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChooseMod")).pick_folder() {
                                job = Some((id, Job::AddCharacter { folder, replaces: self.replaces.clone() }));
                            }
                        }
                    }
                    Kind::Other => {
                        ui.checkbox(&mut self.accept_losses, say!("LumaLink.Window.AcceptLosses"));
                        if button(ui, "LumaLink.Window.Convert") {
                            if let Some(folder) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChooseNewFolder")).save_file() {
                                job = Some((id, Job::Convert { folder, accept_losses: self.accept_losses }));
                            }
                        }
                    }
                }
            });
        });
        if let Some((luma, verify)) = add {
            let luma_id = self.add(luma);
            job = Some((luma_id, verify));
        }
        if let Some((on, job)) = job {
            self.start(on, job);
        }
        ui.separator();
        let Some(item) = self.items.iter().find(|i| i.id == id) else { return };
        let mut explain = None;
        let mut open = None;
        egui::ScrollArea::vertical().id_salt("lines").auto_shrink([false, false]).show(ui, |ui| {
            for line in &item.lines {
                ui.horizontal_wrapped(|ui| {
                    if let Some(code) = line.code {
                        if ui.link(RichText::new(format!("L{code}")).color(line.color)).on_hover_text(say!("LumaLink.Window.Explain")).clicked() {
                            explain = Some(code);
                        }
                    }
                    ui.label(RichText::new(&line.text).color(line.color));
                    if let Some((place, on_disk)) = &line.place {
                        match on_disk {
                            Some(file) => {
                                if ui.link(place).on_hover_text(say!("LumaLink.Window.Show")).clicked() {
                                    open = Some(file.clone());
                                }
                            }
                            None => {
                                ui.label(RichText::new(place).color(SOFT));
                            }
                        }
                    }
                });
            }
            if let Some(made) = &item.made {
                if ui.button(say!("LumaLink.Window.Show")).clicked() {
                    open = Some(made.clone());
                }
            }
        });
        if let Some(code) = explain {
            // What the code means, in place of the report; Check brings the report back.
            self.start(id, Job::Explain(code));
        }
        if let Some(file) = open {
            show_file(&file);
        }
    }

    fn new_window(&mut self, ctx: &egui::Context) {
        let Some(form) = self.new_form.as_mut() else { return };
        let mut open = true;
        let mut made = None;
        egui::Window::new(say!("LumaLink.Window.New")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(say!("LumaLink.Window.ChooseNewFolder")).clicked() {
                    form.folder = rfd::FileDialog::new().save_file();
                }
                if let Some(folder) = &form.folder {
                    ui.label(folder.display().to_string());
                }
            });
            egui::Grid::new("new").num_columns(2).show(ui, |ui| {
                for (row, value) in [("LumaLink.Window.Id", &mut form.id), ("LumaLink.Window.Name", &mut form.name), ("LumaLink.Window.Summary", &mut form.summary), ("LumaLink.Window.Authors", &mut form.authors)] {
                    ui.label(text::say(row, &[]));
                    ui.text_edit_singleline(value);
                    ui.end_row();
                }
            });
            for line in &form.refused {
                ui.label(RichText::new(&line.text).color(line.color));
            }
            if ui.add_enabled(form.folder.is_some(), egui::Button::new(say!("LumaLink.Window.Make"))).clicked() {
                if let Some(folder) = form.folder.clone() {
                    let authors: Vec<String> = form.authors.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
                    form.refused = match jobs::new_mod(&folder, form.id.trim(), form.name.trim(), form.summary.trim(), &authors) {
                        Ok(NewMod::Made(folder)) => {
                            made = Some(folder);
                            vec![]
                        }
                        Ok(NewMod::Refused(r)) => report(&r, None),
                        Err(error) => vec![Line::words(format!("{error:#}"), RED)],
                    };
                }
            }
        });
        if let Some(folder) = made {
            self.new_form = None;
            self.add(folder);
        } else if !open {
            self.new_form = None;
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

impl App {
    /// One frame of the window (eframe's update, and the tests' frames).
    fn ui(&mut self, ctx: &egui::Context) {
        self.take_done();
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for path in dropped {
            self.add(path);
        }
        if self.items.iter().any(|i| i.busy) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        sky::backdrop(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect());
        egui::TopBottomPanel::top("top").frame(egui::Frame::new().inner_margin(12.0)).show(ctx, |ui| self.top(ui));
        egui::SidePanel::left("items").resizable(true).default_width(260.0).frame(egui::Frame::new().inner_margin(12.0)).show(ctx, |ui| self.list(ui));
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(12.0)).show(ctx, |ui| {
            if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                ui.painter().rect_stroke(ui.max_rect(), 14.0, Stroke::new(3.0, sky::BLUE), egui::StrokeKind::Inside);
            }
            self.page(ui);
        });
        self.new_window(ctx);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.95, 0.96, 0.97, 1.0]
    }
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("rounded".to_string(), std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/MPLUSRounded1c-Medium.ttf"))));
    fonts.font_data.insert("rounded-bold".to_string(), std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/MPLUSRounded1c-ExtraBold.ttf"))));
    fonts.families.insert(FontFamily::Name("bold".into()), vec!["rounded-bold".to_string(), "rounded".to_string()]);
    if let Some(body) = fonts.families.get_mut(&FontFamily::Proportional) {
        body.insert(0, "rounded".to_string());
    }
    // Korean and Chinese are not in the rounded font: the system's stand behind it, each weight behind its own family.
    for (family, weight, bold) in [(FontFamily::Name("bold".into()), "system-bold", true), (FontFamily::Proportional, "system", false)] {
        for (i, (path, face)) in words::system_fonts(bold).into_iter().enumerate() {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let name = format!("{weight}{i}");
            let mut data = FontData::from_owned(bytes);
            data.index = face;
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(data));
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push(name);
            }
        }
    }
    if words::system_font_first() {
        // Every character of a line from one font, Latin letters too: the rounded font holds some Chinese characters.
        for family in [FontFamily::Proportional, FontFamily::Name("bold".into())] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.sort_by_key(|name| !name.starts_with("system"));
            }
        }
    }
    fonts
}

/// The window's fonts and look (the window's and the tests' context).
fn setup(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals(egui::Visuals::light());
    let mut style = (*ctx.style()).clone();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.override_text_color = Some(INK);
    style.visuals.window_fill = Color32::WHITE;
    style.visuals.window_corner_radius = egui::CornerRadius::same(14);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(2.0, sky::BLUE_EDGE);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(2.5, sky::BLUE);
    style.visuals.widgets.open.bg_stroke = Stroke::new(2.5, sky::BLUE);
    for widget in [&mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active, &mut style.visuals.widgets.open] {
        widget.corner_radius = egui::CornerRadius::same(15);
    }
    style.spacing.button_padding = Vec2::new(14.0, 7.0);
    ctx.set_style(style);
}

fn main() -> eframe::Result<()> {
    text::set_language(words::language());
    let viewport = egui::ViewportBuilder::default().with_title("LumaLink").with_inner_size([960.0, 640.0]).with_min_inner_size([720.0, 480.0]).with_drag_and_drop(true);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "LumaLink",
        options,
        Box::new(|creation| {
            setup(&creation.egui_ctx);
            // Paths given to the program (a file dropped on its icon, "Open with") are added as dropped ones are.
            Ok(Box::new(App::with(Installed::find(), std::env::args_os().skip(1).map(PathBuf::from))))
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
            setup(&ctx);
            let mut screen = Screen { ctx, app, nodes: vec![], events: vec![], dropped: vec![] };
            screen.frame();
            screen
        }

        /// One frame of the window at its first size, with what was dropped and pressed since the last.
        fn frame(&mut self) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
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

    #[test]
    fn a_person_works_on_a_mod_in_the_window() {
        text::set_language("UsEnglish");
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("round");
        assert!(matches!(jobs::new_mod(&folder, "test.round", "Round", "One line.", &["Me".into()]).unwrap(), NewMod::Made(_)));
        let mut s = Screen::new(App::with(None, std::iter::empty()));
        // Nothing yet: the window asks for a mod and says it found no game.
        assert!(s.shows(&say!("LumaLink.Window.Drop")) && s.shows(&say!("LumaLink.Window.NoGame")), "{:?}", s.words());

        // The folder dropped: listed and chosen, its kind and its jobs shown.
        s.drop_on(&folder);
        assert!(s.shows(&folder.display().to_string()) && s.shows(&say!("LumaLink.Window.Kind.Folder")), "{:?}", s.words());
        for row in ["LumaLink.Window.Check", "LumaLink.Window.Build", "LumaLink.Window.Pack", "LumaLink.Window.Install", "LumaLink.Window.Play"] {
            assert!(s.shows(&text::say(row, &[])), "{row}: {:?}", s.words());
        }
        // Check: the library's report on the window's own inputs, its count line shown.
        s.button("LumaLink.Window.Check");
        let expected = jobs::summary_line(&jobs::validate(&folder, &s.app.inputs()).unwrap());
        s.wait_for(&expected);

        // Pack: the .luma written beside the folder, named in the window.
        s.button("LumaLink.Window.Pack");
        s.wait_for(".luma");
        let luma = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok().map(|e| e.path())).find(|p| p.extension().is_some_and(|x| x == "luma")).expect("a .luma beside the folder");
        assert!(s.shows(&luma.display().to_string()), "{:?}", s.words());

        // Install with no game found: refused by name.
        s.button("LumaLink.Window.Install");
        s.wait_for(&say!("LumaLink.NoInstalled"));

        // The .luma dropped: its own jobs; List gives its entries.
        s.drop_on(&luma);
        assert!(s.shows(&say!("LumaLink.Window.Kind.Luma")), "{:?}", s.words());
        s.button("LumaLink.Window.List");
        s.wait_for("luma.json");

        // Another language from the menu at the top: the window's words are that language's at the next frame.
        let english = say!("LumaLink.Window.List");
        s.press(Role::ComboBox, None);
        s.frame();
        s.press(Role::Button, Some(words::name("EuGerman")));
        s.frame();
        assert_eq!(text::language(), "EuGerman");
        let german = say!("LumaLink.Window.List");
        assert_ne!(english, german);
        assert!(s.shows(&german) && !s.shows(&english), "{:?}", s.words());
        // The menu closed as a screen reader closes it (its button again); back to English the same way.
        s.press(Role::ComboBox, None);
        s.frame();
        s.press(Role::ComboBox, None);
        s.frame();
        s.press(Role::Button, Some(words::name("UsEnglish")));
        s.frame();
        s.press(Role::ComboBox, None);
        s.frame();
        assert_eq!(text::language(), "UsEnglish");
        assert!(!s.shows(words::name("EuGerman")), "the menu is still open: {:?}", s.words());

        // Both taken off the list, one at a time: the window asks for a mod again.
        s.press(Role::Button, Some("×"));
        s.frame();
        assert_eq!(s.words().iter().filter(|w| *w == "×").count(), 1, "{:?}", s.words());
        s.press(Role::Button, Some("×"));
        s.frame();
        assert!(s.shows(&say!("LumaLink.Window.Drop")) && !s.shows("×"), "{:?}", s.words());
    }
}
