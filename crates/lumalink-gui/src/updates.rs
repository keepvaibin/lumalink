//! The window's update banner. An installed LumaLink looks for a newer release when the window opens (at most once an
//! hour, on a thread of its own; `lumalink_core::update`), and the settings menu checks when asked. A newer release
//! shows a banner along the top: Update (the signed Setup is downloaded and checked, and installs once the window
//! closes, then opens it again; for a copy Setup did not install, such as a build or a package manager's, the
//! release's page), What's new, Not now, or Skip this version.

use eframe::egui::{self, RichText};
use lumalink_core::say;
use lumalink_core::update::{self, Release, Settings};
use lumalink_gui::sky::{BLUE_INK, GREEN, RED};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// Where the update stands.
pub enum Update {
    /// Nothing to show.
    Quiet,
    /// A check is running; `asked` when the person asked for it (its answer is shown even when there is nothing newer).
    Checking { answer: Receiver<Result<Option<Release>, String>>, asked: bool },
    /// A newer release to offer.
    Offer(Release),
    /// The person asked, and this is the newest.
    UpToDate,
    /// The Setup is downloading.
    Downloading { release: Release, got: Arc<AtomicU64>, whole: Arc<AtomicU64>, cancel: Arc<AtomicBool>, done: Receiver<Result<PathBuf, String>> },
    /// The Setup is downloaded and checked; it starts once no job is running.
    Ready { release: Release, setup: PathBuf },
    /// A check or a download failed.
    Failed { release: Option<Release>, why: String },
}

/// Whether this copy installs updates itself: one Setup installed. Any other copy gets the release's page.
fn installs_itself() -> bool {
    update::installed_copy()
}

impl Update {
    /// The check of an installed LumaLink as the window opens; nothing for a build or with the setting off.
    pub fn at_start() -> Update {
        if !update::checks_by_itself() {
            return Update::Quiet;
        }
        update::remove_old_downloads();
        Update::check(false)
    }

    /// Starts a check on a thread; one the person asked for reads the list afresh and shows a skipped version too.
    pub fn check(asked: bool) -> Update {
        let (sender, answer) = channel();
        std::thread::spawn(move || {
            let result = if asked {
                update::check_now().map(|r| r.filter(|r| update::offered(r, None))).map_err(|e| format!("{e:#}"))
            } else {
                Ok(update::newer_to_offer(update::CHECK_INTERVAL))
            };
            let _ = sender.send(result);
        });
        Update::Checking { answer, asked }
    }

    fn download(release: Release) -> Update {
        let (got, whole, cancel) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (sender, done) = channel();
        let (g, w, c, r) = (got.clone(), whole.clone(), cancel.clone(), release.clone());
        std::thread::spawn(move || {
            let result = update::download(
                &r,
                &mut |so_far, all| {
                    g.store(so_far, Ordering::Relaxed);
                    w.store(all, Ordering::Relaxed);
                },
                &c,
            )
            .map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        Update::Downloading { release, got, whole, cancel, done }
    }

    /// Takes a finished check's or download's answer; a frame comes again soon while one runs.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if matches!(self, Update::Checking { .. } | Update::Downloading { .. } | Update::Ready { .. }) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let next = match self {
            Update::Checking { answer, asked } => match answer.try_recv() {
                Ok(Ok(Some(release))) => Some(Update::Offer(release)),
                Ok(Ok(None)) => Some(if *asked { Update::UpToDate } else { Update::Quiet }),
                Ok(Err(why)) => Some(if *asked { Update::Failed { release: None, why } } else { Update::Quiet }),
                Err(TryRecvError::Empty) => None,
                // The check's thread ended with no answer (a LumaLink fault): said when asked for, as an answer is.
                Err(TryRecvError::Disconnected) => Some(if *asked { Update::Failed { release: None, why: say!("LumaLink.Window.Crashed") } } else { Update::Quiet }),
            },
            Update::Downloading { release, done, cancel, .. } => match done.try_recv() {
                Ok(Ok(setup)) => Some(Update::Ready { release: release.clone(), setup }),
                // Cancelled: the offer again.
                Ok(Err(_)) if cancel.load(Ordering::Relaxed) => Some(Update::Offer(release.clone())),
                Ok(Err(why)) => Some(Update::Failed { release: Some(release.clone()), why }),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Update::Failed { release: Some(release.clone()), why: say!("LumaLink.Window.Crashed") }),
            },
            _ => None,
        };
        if let Some(next) = next {
            *self = next;
        }
    }

    /// Whether there is a banner to show.
    pub fn showing(&self) -> bool {
        !matches!(self, Update::Quiet | Update::Checking { asked: false, .. })
    }

    /// The banner's frame; true when the Setup was started and the window is to close. `busy`: a job is running (the
    /// Setup waits for it).
    pub fn banner(&mut self, ui: &mut egui::Ui, busy: bool) -> bool {
        let mut next = None;
        let mut close = false;
        ui.horizontal_wrapped(|ui| match self {
            Update::Quiet => {}
            Update::Checking { .. } => {
                ui.label(say!("LumaLink.Window.Update.Checking"));
            }
            Update::UpToDate => {
                ui.label(RichText::new(say!("LumaLink.Window.Update.UpToDate", version = update::VERSION)).color(GREEN));
                if ui.button(say!("LumaLink.Window.Update.Close")).clicked() {
                    next = Some(Update::Quiet);
                }
            }
            Update::Offer(release) => {
                ui.label(RichText::new(say!("LumaLink.Window.Update.Available", version = release.version, current = update::VERSION)).color(BLUE_INK));
                if installs_itself() {
                    if ui.button(say!("LumaLink.Window.Update.Install")).clicked() {
                        next = Some(Update::download(release.clone()));
                    }
                } else if ui.button(say!("LumaLink.Window.Update.Download")).clicked() {
                    // Its page, or with none (N-4066) its signed Setup itself.
                    let to = if release.page.is_empty() { &release.package_url } else { &release.page };
                    ui.ctx().open_url(egui::OpenUrl::new_tab(to));
                }
                if !release.page.is_empty() && ui.button(say!("LumaLink.Window.Update.WhatsNew")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&release.page));
                }
                if ui.button(say!("LumaLink.Window.Update.NotNow")).clicked() {
                    next = Some(Update::Quiet);
                }
                if ui.button(say!("LumaLink.Window.Update.Skip")).clicked() {
                    let mut settings = Settings::load();
                    settings.skipped_version = Some(release.version.clone());
                    next = Some(match settings.save() {
                        Ok(()) => Update::Quiet,
                        Err(e) => Update::Failed { release: None, why: format!("{e:#}") },
                    });
                }
            }
            Update::Downloading { release, got, whole, cancel, .. } => {
                let (got, whole) = (got.load(Ordering::Relaxed), whole.load(Ordering::Relaxed));
                ui.label(say!("LumaLink.Window.Update.Downloading", version = release.version));
                let fraction = if whole > 0 { got as f32 / whole as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(fraction).desired_width(220.0).show_percentage());
                if ui.button(say!("LumaLink.Window.Update.Cancel")).clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Update::Ready { release, setup } => {
                if busy {
                    ui.label(say!("LumaLink.Window.Update.Waiting", version = release.version));
                } else {
                    match update::start_setup(setup, true) {
                        Ok(()) => close = true,
                        Err(e) => next = Some(Update::Failed { release: Some(release.clone()), why: format!("{e:#}") }),
                    }
                }
            }
            Update::Failed { release, why } => {
                ui.label(RichText::new(why.as_str()).color(RED));
                if let Some(release) = release {
                    if ui.button(say!("LumaLink.Window.Update.Retry")).clicked() {
                        next = Some(Update::download(release.clone()));
                    }
                }
                if ui.button(say!("LumaLink.Window.Update.Close")).clicked() {
                    next = Some(Update::Quiet);
                }
            }
        });
        if let Some(next) = next {
            *self = next;
        }
        close
    }
}
