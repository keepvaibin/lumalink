//! Installing, updating and removing LumaLink, for the signed-in user only: no administrator, nothing outside the
//! user's own folders (and on Windows the user's own registry).
//!
//! - **Files.** The Setup carries LumaLink's files in one archive (build.rs): the command line ([`CLI`]), the window
//!   ([`GUI`]), `toolchain/` (clang and lld of LLVM [`lumalink_core::build::LLVM`], with clang's headers), `lumakit/`
//!   and the licences. Each file is written beside its place and then moved over the old one; on Windows a file in use
//!   (a LumaLink window, `lumalink lsp` in an editor, a build's clang) is moved aside first and removed by the next
//!   Setup. What the last install had and this one has not is removed (`installed-files.txt`).
//! - **Choices.** The folder, PATH, the menu entry and the desktop shortcut, and the update check, kept for the next
//!   Setup (an update installs with them).
//! - **The system's records** ([`system`]): on Windows the Apps entry (Modify and Uninstall run this Setup, copied
//!   into the folder), LumaLink in the .luma files' "Open with" list (Nebula opens them by default), the Start menu
//!   and desktop shortcuts and the folder on the user's PATH. On Linux the desktop entry (the applications menu and
//!   "Open with" for .luma), the icon and the .luma file type; on macOS LumaLink.app (Applications and the desktop).
//!   On both, PATH through the shells' startup files.

use anyhow::{bail, Context, Result};
use lumalink_core::say;
use lumalink_core::update::{self, Settings, SETUP_NAME, VERSION};
use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

/// The command line's file name.
pub const CLI: &str = if cfg!(windows) { "lumalink.exe" } else { "lumalink" };
/// The window's file name.
pub const GUI: &str = if cfg!(windows) { "lumalink-gui.exe" } else { "lumalink-gui" };
/// The files the last install wrote, one relative path a line.
const MANIFEST: &str = "installed-files.txt";
/// lld, and the names clang calls it by (each a link to it).
const LLD: &str = if cfg!(windows) { "lld.exe" } else { "lld" };
const LLD_NAMES: [&str; 3] = if cfg!(windows) { ["ld.lld.exe", "lld-link.exe", "ld64.lld.exe"] } else { ["ld.lld", "lld-link", "ld64.lld"] };
/// A file moved aside because it was in use; the next Setup removes it.
const ASIDE: &str = ".lumalink-old";

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));

/// LumaLink's icon as a PNG of `size` (128, 256 or 512; build.rs draws them), for Linux's menu and macOS's bundle.
#[cfg(unix)]
fn icon_png(size: u32) -> &'static [u8] {
    match size {
        128 => include_bytes!(concat!(env!("OUT_DIR"), "/icon-128.png")),
        256 => include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.png")),
        _ => include_bytes!(concat!(env!("OUT_DIR"), "/icon-512.png")),
    }
}

/// The person's choices for an install.
#[derive(Clone, Debug, PartialEq)]
pub struct Choices {
    pub folder: PathBuf,
    /// The folder on the user's PATH: `lumalink` in any new terminal.
    pub path: bool,
    /// LumaLink in the system's list of programs: the Start menu, the applications menu, Applications.
    pub menu: bool,
    pub desktop: bool,
    pub check_for_updates: bool,
}

impl Choices {
    /// The last install's choices; for a first install the defaults: the per-user programs folder, PATH, the menu
    /// entry and the update check on, no desktop shortcut.
    pub fn last() -> Choices {
        let (path, menu, desktop) = system::last_flags().unwrap_or((true, true, false));
        let folder = installed().map(|f| f.folder).unwrap_or_else(system::default_folder);
        Choices { folder, path, menu, desktop, check_for_updates: Settings::load().check_for_updates }
    }
}

/// The LumaLink installed now: its folder and version.
#[derive(Clone, Debug)]
pub struct Found {
    pub folder: PathBuf,
    pub version: String,
}

/// The install the system has a record of, when its folder still has LumaLink in it.
pub fn installed() -> Option<Found> {
    system::installed().filter(|f| f.folder.join(CLI).is_file())
}

/// Whether this Setup carries LumaLink's files (a Setup built without them is a test's).
pub fn has_files() -> bool {
    zip::ZipArchive::new(Cursor::new(PAYLOAD)).is_ok_and(|a| !a.is_empty())
}

/// What an install is doing, for its progress bar.
pub trait Progress: Sync {
    fn step(&self, done: u64, whole: u64);
}

/// Installs LumaLink into `choices.folder` with `choices`, over the install there (an update or a repair).
pub fn install(choices: &Choices, progress: &dyn Progress) -> Result<()> {
    let folder = &choices.folder;
    let mut archive = zip::ZipArchive::new(Cursor::new(PAYLOAD)).context("the Setup's archive")?;
    if archive.is_empty() {
        bail!(say!("LumaLink.Setup.Empty", setup = update::package_name(VERSION)));
    }
    std::fs::create_dir_all(folder).map_err(|e| anyhow::anyhow!(say!("LumaLink.Setup.CannotWrite", folder = folder.display(), why = e)))?;
    // A running LumaLink (a window, `lumalink lsp` in an editor, a build's clang) holds its files. It is waited for a
    // moment (an update starts while the window it replaces closes), then named, as Uninstall names it.
    let start = std::time::Instant::now();
    loop {
        match system::in_use(own_files(folder)).first() {
            None => break,
            Some(first) if start.elapsed() >= HELD_FOR => {
                bail!(say!("LumaLink.Setup.InUse", file = first.display(), why = say!("LumaLink.Setup.Running")))
            }
            Some(_) => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
    remove_set_aside(folder);
    let whole: u64 = (0..archive.len()).filter_map(|i| archive.by_index_raw(i).ok().map(|e| e.size())).sum();
    let mut done = 0u64;
    let mut written = BTreeSet::new();
    let mut buffer = vec![0u8; 1 << 20];
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else { bail!("{}: not a plain path in the Setup's archive", entry.name()) };
        let runs = entry.unix_mode().is_some_and(|m| m & 0o111 != 0);
        let target = folder.join(&relative);
        let new = beside(&target, ".new");
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
        }
        let mut out = patiently(|| std::fs::File::create(&new)).map_err(|e| anyhow::anyhow!(say!("LumaLink.Setup.CannotWrite", folder = new.display(), why = e)))?;
        loop {
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            out.write_all(&buffer[..n]).map_err(|e| anyhow::anyhow!(say!("LumaLink.Setup.CannotWrite", folder = new.display(), why = e)))?;
            done += n as u64;
            progress.step(done, whole);
        }
        out.sync_all()?;
        drop(out);
        // The programs run; everything else is read.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(if runs { 0o755 } else { 0o644 }))?;
        }
        let _ = runs;
        put(&new, &target)?;
        written.insert(relative.to_string_lossy().replace('\\', "/"));
    }
    // lld under the names clang calls it by: links to the one file (on Windows hard links, copies where the disk has
    // none). A link takes no room, so the size Windows shows counts it once.
    let bin = folder.join("toolchain").join("bin");
    // Only Windows' hard links are counted (a Unix link is its own small file).
    #[cfg_attr(unix, allow(unused_mut))]
    let mut linked = 0u64;
    if bin.join(LLD).is_file() {
        for name in LLD_NAMES {
            let new = bin.join(format!("{name}.new"));
            let _ = std::fs::remove_file(&new);
            #[cfg(windows)]
            if std::fs::hard_link(bin.join(LLD), &new).is_ok() {
                linked += new.metadata().map_or(0, |m| m.len());
            } else {
                std::fs::copy(bin.join(LLD), &new).with_context(|| new.display().to_string())?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(LLD, &new).with_context(|| new.display().to_string())?;
            put(&new, &bin.join(name))?;
            written.insert(format!("toolchain/bin/{name}"));
        }
    }
    // This Setup, for Uninstall (and Modify in Windows' Apps), and as the mark of an installed LumaLink, which installs
    // its own updates (update::installed_copy).
    let me = std::env::current_exe()?;
    let copy = folder.join(SETUP_NAME);
    if !same_file(&me, &copy) {
        let new = beside(&copy, ".new");
        std::fs::copy(&me, &new).with_context(|| new.display().to_string())?;
        put(&new, &copy)?;
    }
    written.insert(SETUP_NAME.to_string());
    let mut settings = Settings::load();
    settings.check_for_updates = choices.check_for_updates;
    settings.save()?;
    // The system's records, with the files they put in the folder.
    written.extend(system::register(choices, folder_size(folder).saturating_sub(linked))?);
    // What the last install wrote and this one did not.
    if let Ok(last) = std::fs::read_to_string(folder.join(MANIFEST)) {
        for relative in last.lines().filter(|l| !l.is_empty() && !written.contains(*l)) {
            let path = folder.join(relative);
            if path.starts_with(folder) && !relative.contains("..") {
                remove_or_set_aside(&path);
            }
        }
        remove_empty_folders(folder);
    }
    let list: String = written.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(folder.join(MANIFEST), list)?;
    update::remove_old_downloads();
    Ok(())
}

/// `path` with `suffix` added to its name.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// How long a file operation waits for a file another program holds for a moment.
const HELD_FOR: std::time::Duration = std::time::Duration::from_secs(15);

/// Whether `error` is Windows refusing a file that another program has open: a file just written is opened by the
/// antivirus scan (and the search indexer) for a moment, and moving or removing it meanwhile fails with "Access is
/// denied" (5) or a sharing violation (32, 33). Elsewhere an open file can be moved and removed.
fn held(error: &std::io::Error) -> bool {
    cfg!(windows) && (error.kind() == std::io::ErrorKind::PermissionDenied || matches!(error.raw_os_error(), Some(5 | 32 | 33)))
}

/// Runs `op` until it succeeds, again while the file is held ([`held`]), for up to [`HELD_FOR`].
fn patiently<T>(mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let start = std::time::Instant::now();
    let mut wait = std::time::Duration::from_millis(20);
    loop {
        match op() {
            Err(e) if held(&e) && start.elapsed() < HELD_FOR => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(std::time::Duration::from_millis(500));
            }
            result => return result,
        }
    }
}

/// Moves `new` over `target`. A target in use (on Windows a running program can be renamed but not replaced) is moved
/// aside first; a file held for a moment is waited for ([`patiently`]). When `new` cannot take its place, the old
/// file goes back, so a failed install leaves the last one working.
fn put(new: &Path, target: &Path) -> Result<()> {
    if std::fs::rename(new, target).is_ok() {
        return Ok(());
    }
    let aside = beside(target, &format!(".{}{ASIDE}", std::process::id()));
    let moved = target.exists();
    if moved {
        patiently(|| std::fs::rename(target, &aside)).map_err(|e| anyhow::anyhow!(say!("LumaLink.Setup.InUse", file = target.display(), why = e)))?;
    }
    patiently(|| std::fs::rename(new, target)).map_err(|e| {
        if moved {
            let _ = patiently(|| std::fs::rename(&aside, target));
        }
        let _ = std::fs::remove_file(new);
        anyhow::anyhow!(say!("LumaLink.Setup.CannotWrite", folder = target.display(), why = e))
    })
}

fn remove_or_set_aside(path: &Path) {
    if patiently(|| std::fs::remove_file(path)).is_err() && path.exists() {
        let _ = std::fs::rename(path, beside(path, &format!(".{}{ASIDE}", std::process::id())));
    }
}

/// Removes the files an earlier Setup moved aside (in use then; most are free now).
fn remove_set_aside(folder: &Path) {
    for file in files_under(folder) {
        if file.to_string_lossy().ends_with(ASIDE) {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// The files under `folder`, links as themselves (never followed).
fn files_under(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![folder.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => todo.push(path),
                Ok(_) => out.push(path),
                Err(_) => {}
            }
        }
    }
    out
}

/// Removes the empty folders under `folder` (deepest first), never `folder` itself.
fn remove_empty_folders(folder: &Path) {
    fn walk(dir: &Path, top: bool) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&entry.path(), false);
            }
        }
        if !top {
            let _ = std::fs::remove_dir(dir);
        }
    }
    walk(folder, true);
}

fn folder_size(folder: &Path) -> u64 {
    files_under(folder).iter().filter_map(|f| f.symlink_metadata().ok()).map(|m| m.len()).sum()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether `path` is there, as a file or a link (a link to nothing included).
fn present(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// The files an install wrote in `folder` (its list; for an install without one, LumaLink's own names), the files
/// moved aside, and the list itself. Nothing else in the folder is LumaLink's.
fn own_files(folder: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match std::fs::read_to_string(folder.join(MANIFEST)) {
        Ok(list) => list.lines().filter(|l| inside(l)).map(|l| folder.join(l)).collect(),
        Err(_) => [CLI, GUI, SETUP_NAME, "LICENSE", "README.md"]
            .iter()
            .map(|n| folder.join(n))
            .chain(["toolchain", "lumakit", "licenses"].iter().flat_map(|d| files_under(&folder.join(d))))
            .collect(),
    };
    out.extend(files_under(folder).into_iter().filter(|f| f.to_string_lossy().ends_with(ASIDE)));
    out.push(folder.join(MANIFEST));
    out.retain(|f| present(f) && !f.is_dir());
    out
}

/// Whether a line of the install's list names a place inside the install folder: a relative path of plain names only.
/// An absolute path, a drive, `..` or `.` is not one of the files an install writes, so it is never removed.
fn inside(line: &str) -> bool {
    !line.is_empty() && Path::new(line).components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// The files of `found` that a running program holds (a LumaLink window, `lumalink lsp` in an editor, a build). Only
/// Windows keeps a running program's file from being removed.
pub fn in_use(found: &Found) -> Vec<PathBuf> {
    system::in_use(own_files(&found.folder))
}

/// Removes LumaLink: its files (and its folder when nothing else is in it), its records in the system, and its own
/// files in its data folder (`update::remove_own_data`; the folder goes only when nothing else is in it).
pub fn uninstall(found: &Found) -> Result<()> {
    let busy = in_use(found);
    if let Some(first) = busy.first() {
        bail!(say!("LumaLink.Setup.InUse", file = first.display(), why = say!("LumaLink.Setup.Running")));
    }
    system::unregister(found)?;
    let me = std::env::current_exe().unwrap_or_default();
    for file in own_files(&found.folder) {
        // On Windows this Setup's own file goes once it has ended (main: the copy in the temporary folder).
        if cfg!(windows) && same_file(&file, &me) {
            continue;
        }
        patiently(|| std::fs::remove_file(&file)).map_err(|e| anyhow::anyhow!(say!("LumaLink.Setup.InUse", file = file.display(), why = e)))?;
    }
    remove_empty_folders(&found.folder);
    let _ = std::fs::remove_dir(&found.folder);
    if let Some(data) = update::data_folder() {
        update::remove_own_data(&data);
    }
    system::forget();
    Ok(())
}

#[cfg(windows)]
pub mod system {
    //! Windows' records: the Apps entry, "Open with" for .luma, PATH (`HKCU\Environment`), the Start menu and desktop
    //! shortcuts, and the choices in `HKCU\Software\LumaLink`.
    use super::*;
    use winreg::enums::{RegType, HKEY_CURRENT_USER};
    use winreg::{RegKey, RegValue};

    /// Windows' record of LumaLink in Apps (Installed apps, "Programs and Features").
    pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\LumaLink";
    /// The last install's choices.
    pub const CHOICES_KEY: &str = r"Software\LumaLink";
    // Each name split where it would read as a text row's id (text::tests).
    /// LumaLink's file type for "Open with".
    const PROG_ID: &str = concat!("LumaLink", ".Mod");
    /// The shortcuts' file name.
    const SHORTCUT: &str = concat!("LumaLink", ".lnk");

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    /// `%LOCALAPPDATA%\Programs\LumaLink`, where per-user programs go.
    pub fn default_folder() -> PathBuf {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Programs").join("LumaLink")
    }

    /// The last install's PATH, Start menu and desktop choices.
    pub fn last_flags() -> Option<(bool, bool, bool)> {
        let key = hkcu().open_subkey(CHOICES_KEY).ok()?;
        let flag = |name: &str, default: bool| key.get_value::<u32, _>(name).ok().map_or(default, |v| v != 0);
        Some((flag("Path", true), flag("StartMenu", true), flag("Desktop", false)))
    }

    /// The install Windows has a record of.
    pub fn installed() -> Option<Found> {
        let key = hkcu().open_subkey(UNINSTALL_KEY).ok()?;
        let folder = PathBuf::from(key.get_value::<String, _>("InstallLocation").ok()?);
        let version = key.get_value::<String, _>("DisplayVersion").unwrap_or_default();
        Some(Found { folder, version })
    }

    /// Windows' records of the install: the choices, Apps, "Open with" for .luma, PATH and the shortcuts. Nothing is
    /// written into the folder.
    pub fn register(choices: &Choices, size: u64) -> Result<Vec<String>> {
        let (saved, _) = hkcu().create_subkey(CHOICES_KEY)?;
        saved.set_value("Path", &u32::from(choices.path))?;
        saved.set_value("StartMenu", &u32::from(choices.menu))?;
        saved.set_value("Desktop", &u32::from(choices.desktop))?;
        let folder = &choices.folder;
        let setup = folder.join(SETUP_NAME);
        let gui = folder.join(GUI);
        let (key, _) = hkcu().create_subkey(UNINSTALL_KEY)?;
        let quoted = |p: &Path| format!("\"{}\"", p.display());
        key.set_value("DisplayName", &"LumaLink")?;
        key.set_value("DisplayVersion", &VERSION)?;
        key.set_value("Publisher", &"keepvaibin")?;
        key.set_value("InstallLocation", &folder.display().to_string())?;
        key.set_value("DisplayIcon", &gui.display().to_string())?;
        key.set_value("UninstallString", &format!("{} --uninstall", quoted(&setup)))?;
        key.set_value("QuietUninstallString", &format!("{} --uninstall --silent", quoted(&setup)))?;
        key.set_value("ModifyPath", &quoted(&setup))?;
        key.set_value("URLInfoAbout", &format!("https://github.com/{}", update::REPOSITORY))?;
        key.set_value("HelpLink", &format!("https://github.com/{}#readme", update::REPOSITORY))?;
        key.set_value("EstimatedSize", &((size / 1024).min(u64::from(u32::MAX)) as u32))?;
        key.set_value("NoRepair", &1u32)?;

        // .luma: LumaLink in "Open with"; the default (Nebula's, when it is installed) stays.
        let classes = r"Software\Classes";
        let (progid, _) = hkcu().create_subkey(format!(r"{classes}\{PROG_ID}"))?;
        progid.set_value("", &say!("LumaLink.Setup.FileType"))?;
        progid.create_subkey("DefaultIcon")?.0.set_value("", &format!("{},0", gui.display()))?;
        progid.create_subkey(r"shell\open\command")?.0.set_value("", &format!("{} \"%1\"", quoted(&gui)))?;
        let (open_with, _) = hkcu().create_subkey(format!(r"{classes}\.luma\OpenWithProgids"))?;
        open_with.set_raw_value(PROG_ID, &RegValue { bytes: vec![], vtype: RegType::REG_NONE })?;
        shell::associations_changed();

        set_path(folder, choices.path)?;
        let start_menu = shell::programs_folder().map(|p| p.join(SHORTCUT));
        let desktop = shell::desktop_folder().map(|p| p.join(SHORTCUT));
        for (link, wanted) in [(start_menu, choices.menu), (desktop, choices.desktop)] {
            let Some(link) = link else { continue };
            if wanted {
                shell::shortcut(&link, &gui, folder, &say!("LumaLink.Setup.Tagline"))?;
            } else {
                let _ = std::fs::remove_file(&link);
            }
        }
        Ok(Vec::new())
    }

    /// Takes the folder off PATH, removes the shortcuts and "Open with".
    pub fn unregister(found: &Found) -> Result<()> {
        set_path(&found.folder, false)?;
        for link in [shell::programs_folder(), shell::desktop_folder()].into_iter().flatten() {
            let _ = std::fs::remove_file(link.join(SHORTCUT));
        }
        let _ = hkcu().delete_subkey_all(format!(r"Software\Classes\{PROG_ID}"));
        if let Ok(key) = hkcu().open_subkey_with_flags(r"Software\Classes\.luma\OpenWithProgids", winreg::enums::KEY_ALL_ACCESS) {
            let _ = key.delete_value(PROG_ID);
        }
        // The keys LumaLink's entry was in go once nothing else is in them (Nebula keeps its own there).
        for key in [r"Software\Classes\.luma\OpenWithProgids", r"Software\Classes\.luma"] {
            let empty = hkcu().open_subkey(key).and_then(|k| k.query_info()).is_ok_and(|info| info.values == 0 && info.sub_keys == 0);
            if empty {
                let _ = hkcu().delete_subkey(key);
            }
        }
        shell::associations_changed();
        Ok(())
    }

    /// The last records, once the files are gone: the choices and the Apps entry.
    pub fn forget() {
        let _ = hkcu().delete_subkey_all(CHOICES_KEY);
        let _ = hkcu().delete_subkey_all(UNINSTALL_KEY);
    }

    /// Of `files`, the programs another process is running (a running program's file opens for no one else: share
    /// mode 0 fails).
    pub fn in_use(files: Vec<PathBuf>) -> Vec<PathBuf> {
        use std::os::windows::fs::OpenOptionsExt;
        let me = std::env::current_exe().unwrap_or_default();
        files
            .into_iter()
            .filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("dll")))
            .filter(|f| !same_file(f, &me))
            .filter(|f| std::fs::OpenOptions::new().read(true).share_mode(0).open(f).is_err() && f.exists())
            .collect()
    }

    /// The user's PATH (`HKCU\Environment`, never the machine's) with `folder` on it or not, and without the lumalink
    /// folders an earlier Nebula put there (`<Nebula>\versions\<version>\lumalink`, which would stand before it).
    /// Programs started after the change see it.
    fn set_path(folder: &Path, on: bool) -> Result<()> {
        let (env, _) = hkcu().create_subkey("Environment")?;
        let (old, kind) = match env.get_raw_value("Path") {
            Ok(v) => (from_wide(&v.bytes), v.vtype),
            Err(_) => (String::new(), RegType::REG_EXPAND_SZ),
        };
        let new = path_with(&old, folder, on, &expand);
        if new == old {
            return Ok(());
        }
        if new.is_empty() {
            let _ = env.delete_value("Path");
        } else {
            let kind = if kind == RegType::REG_SZ { RegType::REG_SZ } else { RegType::REG_EXPAND_SZ };
            env.set_raw_value("Path", &RegValue { bytes: to_wide(&new), vtype: kind })?;
        }
        shell::environment_changed();
        Ok(())
    }

    fn from_wide(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    fn to_wide(text: &str) -> Vec<u8> {
        text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
    }

    /// `%NAME%`s in `text` filled from the environment.
    fn expand(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find('%') {
            let Some(close) = rest[open + 1..].find('%') else { break };
            out.push_str(&rest[..open]);
            let name = &rest[open + 1..open + 1 + close];
            match std::env::var(name) {
                Ok(value) if !name.is_empty() => out.push_str(&value),
                _ => out.push_str(&rest[open..open + close + 2]),
            }
            rest = &rest[open + close + 2..];
        }
        out.push_str(rest);
        out
    }

    /// A PATH value with `folder` as its last entry (`on`) or without it, each entry as it was written otherwise; an
    /// entry is the folder when it names it once expanded, in any case, with or without a closing backslash.
    pub fn path_with(old: &str, folder: &Path, on: bool, expand: &dyn Fn(&str) -> String) -> String {
        let plain = |p: &str| p.trim().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
        let ours = plain(&folder.to_string_lossy());
        let nebula_lumalink = |p: &str| {
            let p = plain(p);
            let mut parts = p.rsplit('\\');
            parts.next() == Some("lumalink") && parts.next().is_some() && parts.next() == Some("versions")
        };
        let mut entries: Vec<&str> = old.split(';').filter(|e| !e.trim().is_empty()).filter(|e| plain(&expand(e)) != ours && !nebula_lumalink(&expand(e))).collect();
        let folder = folder.to_string_lossy();
        if on {
            entries.push(&folder);
        }
        entries.join(";")
    }

    mod shell {
        //! The shell's parts: the user's folders, shortcuts, and the notices of changed settings.
        use std::path::{Path, PathBuf};
        use windows::core::{Interface, HSTRING};
        use windows::Win32::Foundation::{BOOL, HANDLE};
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::Shell::{IShellLinkW, SHChangeNotify, SHGetKnownFolderPath, ShellLink, FOLDERID_Desktop, FOLDERID_Programs, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

        fn known(id: &windows::core::GUID) -> Option<PathBuf> {
            // SAFETY: the returned string is the shell's, freed once read.
            unsafe {
                let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, HANDLE::default()).ok()?;
                let path = p.to_string().ok().map(PathBuf::from);
                CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
                path
            }
        }

        /// The Start menu's programs folder of the user.
        pub fn programs_folder() -> Option<PathBuf> {
            known(&FOLDERID_Programs)
        }

        pub fn desktop_folder() -> Option<PathBuf> {
            known(&FOLDERID_Desktop)
        }

        /// A shortcut at `link` to `target`, starting in `folder`.
        pub fn shortcut(link: &Path, target: &Path, folder: &Path, tip: &str) -> anyhow::Result<()> {
            // SAFETY: COM on this thread (a second CoInitializeEx is harmless); every string lives through its call.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                shell_link.SetPath(&HSTRING::from(target.as_os_str()))?;
                shell_link.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
                shell_link.SetDescription(&HSTRING::from(tip))?;
                shell_link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
                if let Some(parent) = link.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                shell_link.cast::<IPersistFile>()?.Save(&HSTRING::from(link.as_os_str()), BOOL::from(true))?;
            }
            Ok(())
        }

        /// Tells the shell the file types changed (Open with, icons).
        pub fn associations_changed() {
            unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
        }

        /// Tells running programs the environment changed (Explorer then starts new terminals with the new PATH).
        pub fn environment_changed() {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};
            let name: Vec<u16> = "Environment".encode_utf16().chain(Some(0)).collect();
            let mut result = 0usize;
            // SAFETY: the name lives through the call.
            unsafe { SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, name.as_ptr() as isize, SMTO_ABORTIFHUNG, 5000, &mut result) };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn no_variables(text: &str) -> String {
            text.replace("%LOCALAPPDATA%", r"C:\Users\a\AppData\Local")
        }

        #[test]
        fn path_gets_the_folder_once_at_its_end() {
            let folder = Path::new(r"C:\Users\a\AppData\Local\Programs\LumaLink");
            assert_eq!(path_with("", folder, true, &no_variables), folder.display().to_string());
            assert_eq!(path_with(r"C:\bin;", folder, true, &no_variables), format!(r"C:\bin;{}", folder.display()));
            // Already there, written another way: it moves to the end, once.
            let old = r"%LOCALAPPDATA%\Programs\lumalink\;C:\bin";
            assert_eq!(path_with(old, folder, true, &no_variables), format!(r"C:\bin;{}", folder.display()));
            assert_eq!(path_with(old, folder, false, &no_variables), r"C:\bin");
        }

        #[test]
        fn an_earlier_nebulas_lumalink_folder_leaves_path() {
            let folder = Path::new(r"D:\Tools\LumaLink");
            let old = r"C:\bin;%LOCALAPPDATA%\Programs\Nebula\versions\1.4.0\lumalink;C:\other\lumalink";
            assert_eq!(path_with(old, folder, false, &no_variables), r"C:\bin;C:\other\lumalink", "a lumalink folder of someone else stays");
        }

        #[test]
        fn variables_are_filled_and_unknown_ones_kept() {
            std::env::set_var("LUMALINK_SETUP_TEST_VARIABLE", "x");
            assert_eq!(expand("%LUMALINK_SETUP_TEST_VARIABLE%\\a;%NO_SUCH_VARIABLE_HERE%;100%"), "x\\a;%NO_SUCH_VARIABLE_HERE%;100%");
        }

        #[test]
        fn registry_strings_read_back() {
            assert_eq!(from_wide(&to_wide("C:\\a;D:\\b")), "C:\\a;D:\\b");
        }
    }
}

#[cfg(unix)]
pub mod system {
    //! Linux's and macOS's records, all in the user's home:
    //!
    //! - **The record**: `setup.json` in `update::data_folder` (the folder, the version, the choices).
    //! - **PATH**: `env` in the folder puts the folder on PATH (`env.fish` for fish), and one marked line in each
    //!   shell's startup files sources it: `~/.profile`, the bash files there are (`~/.bash_profile`, `~/.bash_login`,
    //!   `~/.bashrc`), `~/.zshenv` where zsh is, and `~/.config/fish/conf.d/lumalink.fish` where fish is. Terminals
    //!   opened after the install have it, as rustup does it.
    //! - **Linux**: the desktop entry `dev.nebula.lumalink.desktop` in `~/.local/share/applications` (in the menu when
    //!   chosen; always in "Open with" for .luma, Nebula's type `application/x-nebula-luma`), the type itself in
    //!   `~/.local/share/mime/packages`, the icon in `~/.local/share/icons/hicolor`, and a copy of the entry on the
    //!   desktop when chosen.
    //! - **macOS**: `LumaLink.app` in the folder (it starts the window), copied into `~/Applications` for the menu
    //!   (Launchpad, Spotlight) and linked on the desktop when chosen.
    use super::*;
    use serde::{Deserialize, Serialize};

    /// What the install was: the record LumaLink keeps of itself.
    #[derive(Serialize, Deserialize)]
    struct Record {
        folder: PathBuf,
        version: String,
        path: bool,
        menu: bool,
        desktop: bool,
        /// The shells' startup files Setup made for its line: removing the line removes them.
        #[serde(default)]
        made: Vec<PathBuf>,
    }

    /// The desktop entry's, the icon's and the window's name (lumalink_gui::look::APP_ID).
    const APP_ID: &str = lumalink_gui::look::APP_ID;
    /// The mark on the lines this Setup adds to the shells' startup files.
    const MARK: &str = "# LumaLink Setup";
    /// Nebula's type for a .luma (Nebula's platform/linux/dev.nebula.galaxy.xml).
    #[cfg(not(target_os = "macos"))]
    const LUMA_TYPE: &str = "application/x-nebula-luma";

    fn home() -> PathBuf {
        std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute()).unwrap_or_else(|| PathBuf::from("/"))
    }

    fn named(variable: &str) -> Option<PathBuf> {
        std::env::var_os(variable).map(PathBuf::from).filter(|p| p.is_absolute())
    }

    /// `$XDG_DATA_HOME`, else `~/.local/share`.
    fn data_home() -> PathBuf {
        named("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share"))
    }

    /// Where LumaLink goes: `~/.local/share/lumalink` on Linux (`$XDG_DATA_HOME`), `~/Library/LumaLink` on macOS.
    pub fn default_folder() -> PathBuf {
        if cfg!(target_os = "macos") {
            home().join("Library").join("LumaLink")
        } else {
            data_home().join("lumalink")
        }
    }

    fn record_file() -> Option<PathBuf> {
        update::data_folder().map(|d| d.join("setup.json"))
    }

    fn record() -> Option<Record> {
        serde_json::from_slice(&std::fs::read(record_file()?).ok()?).ok()
    }

    pub fn last_flags() -> Option<(bool, bool, bool)> {
        record().map(|r| (r.path, r.menu, r.desktop))
    }

    pub fn installed() -> Option<Found> {
        record().map(|r| Found { folder: r.folder, version: r.version })
    }

    /// The record, PATH, the menu entry and the desktop's; returns the files written into the folder.
    pub fn register(choices: &Choices, _size: u64) -> Result<Vec<String>> {
        let folder = &choices.folder;
        let mut made = record().map(|r| r.made).unwrap_or_default();
        let mut written = Vec::new();
        written.extend(set_path(folder, choices.path, &mut made)?);
        written.extend(desktop::register(folder, choices.menu, choices.desktop)?);
        let file = record_file().context("no home folder")?;
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
        }
        let record = Record {
            folder: folder.clone(),
            version: VERSION.to_string(),
            path: choices.path,
            menu: choices.menu,
            desktop: choices.desktop,
            made,
        };
        std::fs::write(&file, serde_json::to_string_pretty(&record)?).with_context(|| file.display().to_string())?;
        Ok(written)
    }

    pub fn unregister(found: &Found) -> Result<()> {
        let mut made = record().map(|r| r.made).unwrap_or_default();
        set_path(&found.folder, false, &mut made)?;
        desktop::unregister(&found.folder);
        Ok(())
    }

    /// The record goes with the data folder.
    pub fn forget() {
        if let Some(file) = record_file() {
            let _ = std::fs::remove_file(file);
        }
    }

    /// A running program's file can be replaced and removed here.
    pub fn in_use(_files: Vec<PathBuf>) -> Vec<PathBuf> {
        Vec::new()
    }

    /// `text` in single quotes for sh: each `'` closed, escaped and opened again.
    pub fn sh_quoted(text: &str) -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    }

    /// `text` in single quotes for fish, where `\` and `'` are escaped.
    pub fn fish_quoted(text: &str) -> String {
        format!("'{}'", text.replace('\\', r"\\").replace('\'', r"\'"))
    }

    /// The `env` file: the folder first on PATH, once however often it is sourced.
    pub fn env_script(folder: &Path) -> String {
        let q = sh_quoted(&folder.to_string_lossy());
        format!("# LumaLink's folder on PATH. The shells' startup files source this file ({MARK}).\ncase \":${{PATH}}:\" in\n    *:{q}:*) ;;\n    *) export PATH={q}\":$PATH\" ;;\nesac\n")
    }

    pub fn fish_script(folder: &Path) -> String {
        let q = fish_quoted(&folder.to_string_lossy());
        format!("# LumaLink's folder on PATH ({MARK}).\nif not contains -- {q} $PATH\n    set -gx PATH {q} $PATH\nend\n")
    }

    /// `text` without the lines that carry the mark.
    pub fn without_marked(text: &str) -> String {
        text.split_inclusive('\n').filter(|l| !l.contains(MARK)).collect()
    }

    /// `text` with the marked line `line` at its end, on a line of its own.
    pub fn with_marked(text: &str, line: &str) -> String {
        let mut out = without_marked(text);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("{line} {MARK}\n"));
        out
    }

    /// Whether `program` is installed: in one of the usual folders, or the login shell.
    fn has_shell(program: &str) -> bool {
        let shell = std::env::var("SHELL").unwrap_or_default();
        shell.rsplit('/').next() == Some(program)
            || ["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"].iter().any(|d| Path::new(d).join(program).is_file())
    }

    /// The shells' startup files that source `env`: `~/.profile` always; `~/.bash_profile` and `~/.bash_login` where
    /// they are (a bash login shell reads the first of them instead of `~/.profile`); `~/.bashrc` where it is or bash
    /// is the login shell (terminals that are not login shells read it); `~/.zshenv` (in `$ZDOTDIR`) where zsh is.
    fn startup_files() -> Vec<PathBuf> {
        let home = home();
        let mut files = vec![home.join(".profile")];
        for name in [".bash_profile", ".bash_login"] {
            if home.join(name).is_file() {
                files.push(home.join(name));
            }
        }
        if home.join(".bashrc").is_file() || std::env::var("SHELL").is_ok_and(|s| s.ends_with("/bash")) {
            files.push(home.join(".bashrc"));
        }
        let zdotdir = named("ZDOTDIR").unwrap_or_else(|| home.clone());
        if zdotdir.join(".zshenv").is_file() || has_shell("zsh") {
            files.push(zdotdir.join(".zshenv"));
        }
        files
    }

    fn fish_file() -> PathBuf {
        named("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")).join("fish").join("conf.d").join("lumalink.fish")
    }

    /// Writes `text` to `file` through a new file moved over it, so a failure leaves the old one whole; its permissions
    /// are kept.
    fn replace_file(file: &Path, text: &str) -> Result<()> {
        // A startup file that is a link (to a dotfiles repository) is written through the link, by a new file beside
        // the file the link names (a move stays on one file system).
        let target = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let new = beside(&target, ".lumalink-new");
        std::fs::write(&new, text).with_context(|| new.display().to_string())?;
        if let Ok(meta) = std::fs::metadata(&target) {
            let _ = std::fs::set_permissions(&new, meta.permissions());
        }
        std::fs::rename(&new, &target).with_context(|| target.display().to_string())
    }

    /// `file` without LumaLink's lines. A file Setup `made` for the line goes when nothing else is in it.
    fn strip_marked(file: &Path, made: bool) -> Result<()> {
        let Ok(old) = std::fs::read_to_string(file) else { return Ok(()) };
        let new = without_marked(&old);
        if new == old {
            return Ok(());
        }
        if made && new.trim().is_empty() && !file.is_symlink() {
            return std::fs::remove_file(file).with_context(|| file.display().to_string());
        }
        replace_file(file, &new)
    }

    /// The folder on PATH or off it: `env` and `env.fish` in the folder and the marked lines that source them. `made`
    /// is the startup files Setup made, kept up to date. Returns the files written into the folder.
    fn set_path(folder: &Path, on: bool, made: &mut Vec<PathBuf>) -> Result<Vec<String>> {
        let env = folder.join("env");
        let line = format!(". {}", sh_quoted(&env.to_string_lossy()));
        if on {
            for file in startup_files() {
                if std::fs::symlink_metadata(&file).is_err() && !made.contains(&file) {
                    made.push(file.clone());
                }
                let old = std::fs::read_to_string(&file).unwrap_or_default();
                let new = with_marked(&old, &line);
                if new != old {
                    replace_file(&file, &new)?;
                }
            }
        } else {
            // Every file a line may be in, also those of shells no longer installed.
            let home = home();
            let mut files = startup_files();
            files.extend([".profile", ".bash_profile", ".bash_login", ".bashrc", ".zshenv"].map(|name| home.join(name)));
            files.sort();
            files.dedup();
            for file in files {
                strip_marked(&file, made.contains(&file))?;
            }
            made.clear();
        }
        let fish = fish_file();
        let fish_wanted = on && (fish.parent().and_then(Path::parent).is_some_and(Path::is_dir) || has_shell("fish"));
        if fish_wanted {
            if let Some(parent) = fish.parent() {
                std::fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
            }
            let source = format!("source {} {MARK}\n", fish_quoted(&folder.join("env.fish").to_string_lossy()));
            std::fs::write(&fish, source).with_context(|| fish.display().to_string())?;
        } else if std::fs::read_to_string(&fish).is_ok_and(|t| t.contains(MARK)) {
            let _ = std::fs::remove_file(&fish);
        }
        if on {
            std::fs::write(&env, env_script(folder)).with_context(|| env.display().to_string())?;
            std::fs::write(folder.join("env.fish"), fish_script(folder))?;
            Ok(vec!["env".into(), "env.fish".into()])
        } else {
            for name in ["env", "env.fish"] {
                let _ = std::fs::remove_file(folder.join(name));
            }
            Ok(Vec::new())
        }
    }

    #[cfg(not(target_os = "macos"))]
    mod desktop {
        //! The desktop entry, the .luma type and the icon (the freedesktop.org specifications every Linux desktop reads).
        use super::*;

        fn entry_file() -> PathBuf {
            data_home().join("applications").join(format!("{APP_ID}.desktop"))
        }

        fn type_file() -> PathBuf {
            data_home().join("mime").join("packages").join(format!("{APP_ID}.xml"))
        }

        fn icon_file() -> PathBuf {
            data_home().join("icons").join("hicolor").join("256x256").join("apps").join(format!("{APP_ID}.png"))
        }

        /// The desktop's folder: `XDG_DESKTOP_DIR` in `~/.config/user-dirs.dirs`, else `~/Desktop`; none when it is
        /// not there (no desktop).
        fn desktop_folder() -> Option<PathBuf> {
            let dirs = named("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")).join("user-dirs.dirs");
            let named = std::fs::read_to_string(dirs).ok().and_then(|text| {
                text.lines().find_map(|l| {
                    let value = l.trim().strip_prefix("XDG_DESKTOP_DIR=")?.trim().trim_matches('"');
                    let value = value.replace("$HOME", &home().to_string_lossy());
                    Some(PathBuf::from(value)).filter(|p| p.is_absolute())
                })
            });
            named.unwrap_or_else(|| home().join("Desktop")).canonicalize().ok().filter(|p| p.is_dir() && *p != home())
        }

        /// A value of a desktop entry: `\`, newlines and tabs escaped.
        fn value(text: &str) -> String {
            text.replace('\\', r"\\").replace('\n', r"\n").replace('\t', r"\t").replace('\r', r"\r")
        }

        /// `path` as an argument of an entry's Exec: in double quotes, `"`, `` ` ``, `$` and `\` escaped within them,
        /// `%` doubled; then escaped as any value.
        pub fn exec_argument(path: &Path) -> String {
            let text = path.to_string_lossy();
            let mut quoted = String::from("\"");
            for c in text.chars() {
                match c {
                    '"' | '`' | '$' | '\\' => {
                        quoted.push('\\');
                        quoted.push(c);
                    }
                    '%' => quoted.push_str("%%"),
                    _ => quoted.push(c),
                }
            }
            quoted.push('"');
            value(&quoted)
        }

        /// The freedesktop locale name of each language LumaLink speaks, by its place in luma_validate::LANGUAGES.
        fn locale(language: &str) -> Option<&'static str> {
            Some(match language {
                "EuEnglish" => "en_GB",
                "UsFrench" => "fr_CA",
                "EuFrench" => "fr",
                "UsSpanish" => "es_MX",
                "EuSpanish" => "es",
                "EuGerman" => "de",
                "EuItalian" => "it",
                "EuDutch" => "nl",
                "UsPortuguese" => "pt_BR",
                "JpJapanese" => "ja",
                "KrKorean" => "ko",
                "CnSimpChinese" => "zh_CN",
                "TwTradChinese" => "zh_TW",
                _ => return None,
            })
        }

        /// The row `id` in each language that has a locale name, with the name.
        fn translations(id: &str) -> Vec<(&'static str, String)> {
            luma_languages().iter().enumerate().filter_map(|(i, l)| Some((locale(l)?, lumalink_core::text::say_in(i, id, &[])))).collect()
        }

        fn luma_languages() -> &'static [&'static str] {
            &lumalink_core::text::LANGUAGES
        }

        pub fn entry(folder: &Path, in_menu: bool) -> String {
            let mut out = String::from("[Desktop Entry]\nType=Application\nVersion=1.5\nName=LumaLink\n");
            out.push_str(&format!("Comment={}\n", value(&lumalink_core::text::say_in(0, "LumaLink.Setup.Tagline", &[]))));
            for (locale, text) in translations("LumaLink.Setup.Tagline") {
                out.push_str(&format!("Comment[{locale}]={}\n", value(&text)));
            }
            out.push_str(&format!("Icon={APP_ID}\nExec={} %F\nTerminal=false\nCategories=Development;\n", exec_argument(&folder.join(GUI))));
            out.push_str(&format!("MimeType={LUMA_TYPE};\nStartupWMClass={APP_ID}\n"));
            if !in_menu {
                // Only "Open with" lists it.
                out.push_str("NoDisplay=true\n");
            }
            out
        }

        fn xml(text: &str) -> String {
            text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        }

        pub fn type_definition() -> String {
            let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- LumaLink Setup: Nebula's type of a .luma, for \"Open with\". -->\n");
            out.push_str("<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n");
            out.push_str(&format!("  <mime-type type=\"{LUMA_TYPE}\">\n"));
            out.push_str(&format!("    <comment>{}</comment>\n", xml(&lumalink_core::text::say_in(0, "LumaLink.Setup.FileType", &[]))));
            for (locale, text) in translations("LumaLink.Setup.FileType") {
                out.push_str(&format!("    <comment xml:lang=\"{locale}\">{}</comment>\n", xml(&text)));
            }
            out.push_str("    <sub-class-of type=\"application/zip\"/>\n    <glob pattern=\"*.luma\"/>\n  </mime-type>\n</mime-info>\n");
            out
        }

        /// Runs a desktop tool when it is there (it refreshes a cache the desktop also refreshes by itself).
        fn refresh(program: &str, args: &[&std::ffi::OsStr]) {
            let _ = std::process::Command::new(program).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        }

        fn write(file: &Path, bytes: &[u8]) -> Result<()> {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
            }
            std::fs::write(file, bytes).with_context(|| file.display().to_string())
        }

        pub fn register(folder: &Path, menu: bool, on_desktop: bool) -> Result<Vec<String>> {
            write(&icon_file(), super::super::icon_png(256))?;
            write(&type_file(), type_definition().as_bytes())?;
            write(&entry_file(), entry(folder, menu).as_bytes())?;
            if let Some(desktop) = desktop_folder() {
                let shortcut = desktop.join(format!("{APP_ID}.desktop"));
                if on_desktop {
                    write(&shortcut, entry(folder, true).as_bytes())?;
                    // A launcher on the desktop runs only when it is executable (and, in GNOME, marked trusted).
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&shortcut, std::fs::Permissions::from_mode(0o755))?;
                    refresh("gio", &["set".as_ref(), shortcut.as_os_str(), "metadata::trusted".as_ref(), "true".as_ref()]);
                } else {
                    let _ = std::fs::remove_file(shortcut);
                }
            }
            refresh("update-mime-database", &[data_home().join("mime").as_os_str()]);
            refresh("update-desktop-database", &[data_home().join("applications").as_os_str()]);
            Ok(Vec::new())
        }

        pub fn unregister(_folder: &Path) {
            for file in [entry_file(), type_file(), icon_file()] {
                let _ = std::fs::remove_file(file);
            }
            if let Some(desktop) = desktop_folder() {
                let _ = std::fs::remove_file(desktop.join(format!("{APP_ID}.desktop")));
            }
            refresh("update-mime-database", &[data_home().join("mime").as_os_str()]);
            refresh("update-desktop-database", &[data_home().join("applications").as_os_str()]);
        }
    }

    #[cfg(target_os = "macos")]
    mod desktop {
        //! LumaLink.app: a bundle whose program starts the folder's window, so Launchpad, Spotlight, the Dock and the
        //! desktop show LumaLink with its icon.
        use super::*;

        // Split where it would read as a text row's id (text::tests).
        const BUNDLE: &str = concat!("LumaLink", ".app");

        fn applications() -> PathBuf {
            home().join("Applications").join(BUNDLE)
        }

        fn desktop_link() -> PathBuf {
            home().join("Desktop").join(BUNDLE)
        }

        /// An .icns of the icon at 128, 256 and 512 pixels (PNG entries, which macOS reads since 10.7).
        pub fn icns() -> Vec<u8> {
            let mut entries = Vec::new();
            for (kind, size) in [(b"ic07", 128), (b"ic08", 256), (b"ic09", 512)] {
                let png = super::super::icon_png(size);
                entries.extend_from_slice(kind);
                entries.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
                entries.extend_from_slice(png);
            }
            let mut out = b"icns".to_vec();
            out.extend_from_slice(&(entries.len() as u32 + 8).to_be_bytes());
            out.extend_from_slice(&entries);
            out
        }

        fn xml(text: &str) -> String {
            text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        }

        pub fn info_plist() -> String {
            // A bundle's version is numbers and dots: a beta's suffix is left off.
            let numbers = VERSION.split('-').next().unwrap_or(VERSION);
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\
                 \t<key>CFBundleName</key>\n\t<string>LumaLink</string>\n\t<key>CFBundleDisplayName</key>\n\t<string>LumaLink</string>\n\
                 \t<key>CFBundleIdentifier</key>\n\t<string>{APP_ID}</string>\n\t<key>CFBundleVersion</key>\n\t<string>{numbers}</string>\n\
                 \t<key>CFBundleShortVersionString</key>\n\t<string>{numbers}</string>\n\t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n\
                 \t<key>CFBundleExecutable</key>\n\t<string>LumaLink</string>\n\t<key>CFBundleIconFile</key>\n\t<string>LumaLink</string>\n\
                 \t<key>LSMinimumSystemVersion</key>\n\t<string>11.0</string>\n\t<key>NSHighResolutionCapable</key>\n\t<true/>\n\
                 \t<key>NSHumanReadableCopyright</key>\n\t<string>{}</string>\n</dict>\n</plist>\n",
                xml(&lumalink_core::text::say_in(0, "LumaLink.Setup.Tagline", &[]))
            )
        }

        /// The bundle's program: the folder's window, with what it was given.
        pub fn launcher(folder: &Path) -> String {
            format!("#!/bin/sh\nexec {} \"$@\"\n", sh_quoted(&folder.join(GUI).to_string_lossy()))
        }

        /// Writes the bundle at `bundle`; returns its files, relative to the bundle's parent.
        fn write_bundle(bundle: &Path, folder: &Path) -> Result<Vec<String>> {
            use std::os::unix::fs::PermissionsExt;
            let contents = bundle.join("Contents");
            let files: [(&str, Vec<u8>, u32); 3] = [
                ("Contents/Info.plist", info_plist().into_bytes(), 0o644),
                ("Contents/MacOS/LumaLink", launcher(folder).into_bytes(), 0o755),
                ("Contents/Resources/LumaLink.icns", icns(), 0o644),
            ];
            std::fs::create_dir_all(contents.join("MacOS"))?;
            std::fs::create_dir_all(contents.join("Resources"))?;
            let mut written = Vec::new();
            for (relative, bytes, mode) in files {
                let file = bundle.join(relative);
                std::fs::write(&file, bytes).with_context(|| file.display().to_string())?;
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode))?;
                written.push(format!("{BUNDLE}/{relative}"));
            }
            Ok(written)
        }

        /// Whether `bundle` is one this Setup wrote (its identifier is LumaLink's).
        fn ours(bundle: &Path) -> bool {
            std::fs::read_to_string(bundle.join("Contents").join("Info.plist")).is_ok_and(|t| t.contains(&format!("<string>{APP_ID}</string>")))
        }

        pub fn register(folder: &Path, menu: bool, on_desktop: bool) -> Result<Vec<String>> {
            let own = folder.join(BUNDLE);
            let written = write_bundle(&own, folder)?;
            let copy = applications();
            if menu {
                if present(&copy) && !ours(&copy) {
                    bail!(say!("LumaLink.Setup.CannotWrite", folder = copy.display(), why = say!("LumaLink.Setup.NotOurs")));
                }
                let _ = std::fs::remove_dir_all(&copy);
                write_bundle(&copy, folder)?;
            } else if ours(&copy) {
                let _ = std::fs::remove_dir_all(&copy);
            }
            let link = desktop_link();
            let ours_link = std::fs::read_link(&link).is_ok_and(|t| t == own);
            if on_desktop {
                if ours_link || !present(&link) {
                    let _ = std::fs::remove_file(&link);
                    std::os::unix::fs::symlink(&own, &link).with_context(|| link.display().to_string())?;
                }
            } else if ours_link {
                let _ = std::fs::remove_file(&link);
            }
            Ok(written)
        }

        pub fn unregister(folder: &Path) {
            if ours(&applications()) {
                let _ = std::fs::remove_dir_all(applications());
            }
            if std::fs::read_link(desktop_link()).is_ok_and(|t| t == folder.join(BUNDLE)) {
                let _ = std::fs::remove_file(desktop_link());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn quoting_keeps_any_path_whole() {
            assert_eq!(sh_quoted("/home/a b/it's"), r"'/home/a b/it'\''s'");
            assert_eq!(fish_quoted(r"/x/it's\"), r"'/x/it\'s\\'");
        }

        /// The env file, run by sh with PATH as it is and with the folder on it already: the folder once, first.
        #[test]
        fn the_env_file_puts_the_folder_on_path_once() {
            let dir = tempfile::tempdir().unwrap();
            let folder = dir.path().join("Luma Link's");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("env"), env_script(&folder)).unwrap();
            let run = |path: &str| {
                let out = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!(". {} && . {} && printf %s \"$PATH\"", sh_quoted(&folder.join("env").to_string_lossy()), sh_quoted(&folder.join("env").to_string_lossy())))
                    .env("PATH", path)
                    .output()
                    .unwrap();
                String::from_utf8(out.stdout).unwrap()
            };
            let ours = folder.to_string_lossy().into_owned();
            assert_eq!(run("/usr/bin:/bin"), format!("{ours}:/usr/bin:/bin"));
            assert_eq!(run(&format!("/usr/bin:{ours}:/bin")), format!("/usr/bin:{ours}:/bin"));
        }

        #[test]
        fn marked_lines_are_added_once_and_removed_alone() {
            let old = "export A=1\n# mine\n";
            let once = with_marked(old, ". '/x/env'");
            assert_eq!(once, format!("export A=1\n# mine\n. '/x/env' {MARK}\n"));
            assert_eq!(with_marked(&once, ". '/x/env'"), once, "again: the same file");
            assert_eq!(without_marked(&once), old);
            assert_eq!(with_marked("no newline", ". '/y/env'"), format!("no newline\n. '/y/env' {MARK}\n"));
        }

        #[test]
        fn a_startup_file_setup_made_goes_with_its_line_and_a_link_is_written_through() {
            let dir = tempfile::tempdir().unwrap();
            let made = dir.path().join(".profile");
            std::fs::write(&made, with_marked("", ". '/x/env'")).unwrap();
            strip_marked(&made, true).unwrap();
            assert!(!made.exists());
            let theirs = dir.path().join(".zshenv");
            std::fs::write(&theirs, with_marked("", ". '/x/env'")).unwrap();
            strip_marked(&theirs, false).unwrap();
            assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "", "a file of the user's stays, empty as it was");
            let kept = dir.path().join("dotfiles-bashrc");
            std::fs::write(&kept, with_marked("alias l=ls\n", ". '/x/env'")).unwrap();
            let link = dir.path().join(".bashrc");
            std::os::unix::fs::symlink(&kept, &link).unwrap();
            strip_marked(&link, false).unwrap();
            assert!(link.is_symlink());
            assert_eq!(std::fs::read_to_string(&kept).unwrap(), "alias l=ls\n");
            strip_marked(&dir.path().join("not-there"), true).unwrap();
        }

        #[cfg(not(target_os = "macos"))]
        #[test]
        fn the_desktop_entry_quotes_its_program_and_names_the_type() {
            let entry = desktop::entry(Path::new("/home/a/my $tools/lumalink"), false);
            assert!(entry.contains(r#"Exec="/home/a/my \\$tools/lumalink/lumalink-gui" %F"#), "{entry}");
            assert!(entry.contains("MimeType=application/x-nebula-luma;") && entry.contains("NoDisplay=true"), "{entry}");
            assert!(entry.contains("Comment[de]=") && entry.contains("Comment[ja]="), "{entry}");
            assert!(!desktop::entry(Path::new("/x"), true).contains("NoDisplay"));
            assert_eq!(desktop::exec_argument(Path::new("/a/100%")), r#""/a/100%%""#);
            let types = desktop::type_definition();
            assert!(types.contains("<glob pattern=\"*.luma\"/>") && types.contains("xml:lang=\"zh_TW\""), "{types}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M-163: only plain relative lines of the install's list are taken as its files; an absolute path, a drive, `..`
    /// or `.` never is, so uninstall cannot remove a file outside the install folder.
    #[test]
    fn the_install_list_names_only_files_inside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("LumaLink");
        std::fs::create_dir_all(folder.join("toolchain")).unwrap();
        std::fs::write(folder.join("toolchain").join("clang"), "x").unwrap();
        let outside = dir.path().join("keep.txt");
        std::fs::write(&outside, "x").unwrap();
        let absolute = outside.to_string_lossy().into_owned();
        let list = format!("toolchain/clang\n{absolute}\n../keep.txt\n./toolchain/clang\nC:keep.txt\n/keep.txt\n\n");
        std::fs::write(folder.join(MANIFEST), list).unwrap();
        let mut found: Vec<String> = own_files(&folder).iter().map(|f| f.strip_prefix(&folder).unwrap().to_string_lossy().replace('\\', "/")).collect();
        found.sort();
        assert_eq!(found, [MANIFEST.to_string(), "toolchain/clang".to_string()]);
    }

    #[test]
    fn a_file_put_over_its_place_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let (new, target) = (dir.path().join("a.exe.new"), dir.path().join("a.exe"));
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        put(&new, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!new.exists());
    }

    /// The antivirus scan of a file just written holds it with no sharing for a moment: the move waits for it.
    #[cfg(windows)]
    #[test]
    fn a_new_file_held_for_a_moment_is_waited_for() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (new, target) = (dir.path().join("a.exe.new"), dir.path().join("a.exe"));
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&new).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            drop(held);
        });
        put(&new, &target).unwrap();
        release.join().unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        // The old file was moved aside on the way; the next Setup removes it.
        remove_set_aside(dir.path());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// A new file that never frees leaves the old one in its place.
    #[cfg(windows)]
    #[test]
    fn a_failed_put_leaves_the_old_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (new, target) = (dir.path().join("a.exe.new"), dir.path().join("a.exe"));
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&new).unwrap();
        assert!(put(&new, &target).is_err());
        drop(held);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    }
}
