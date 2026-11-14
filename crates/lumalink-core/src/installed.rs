//! The game installed on this computer: every command that reads the game finds it here when the modder
//! names none. The order is the launcher's own (installer/src/Launcher/Launcher.cs `LaunchSession`,
//! Common/NebulaPaths.cs):
//!
//! 1. the content folder the player chose (`content` in `<data>/settings.json`) when it holds `game.pak`;
//! 2. on Windows, `content` under the install root the last install recorded (`HKCU\Software\Nebula`, `InstallRoot`);
//! 3. `content` under the default install root (`%LOCALAPPDATA%\Programs\Nebula`; elsewhere the data folder).
//!
//! The data folder is the game's (galaxy/platform_paths.h): `%LOCALAPPDATA%\Nebula` on Windows,
//! `~/Library/Application Support/Nebula` on Apple's systems, else `$XDG_DATA_HOME/Nebula`, else
//! `~/.local/share/Nebula`. `NEBULA_DATA` names another (a test's private folder), as the game's `--data` does;
//! `NEBULA_INSTALL_ROOT` names the install root in place of 2 and 3. `run` gives the game the data folder found, so
//! the game and LumaLink read the same mods.
//!
//! The version that runs is `current.json`'s under the install root: `versions/<version>/` holds the game's program and
//! its self-description folder `luma` (the function rows, `luma_types.json`, `luma_functions.json` and the placement
//! facts), which the rules on a mod's code read.

use std::path::{Path, PathBuf};

/// Where the installed game is. Each part is there when found; [`Installed::find`] gives one only with its game.pak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The game: `game.pak`.
    pub pak: PathBuf,
    /// Nebula's data folder (settings, saves, mods).
    pub data: PathBuf,
    /// The install root (current.json, versions, content); none for a content folder chosen outside one.
    pub root: Option<PathBuf>,
    /// The version that runs: `versions/<version>` under the root.
    pub version: Option<PathBuf>,
}

/// The program that runs the game in a version folder.
#[cfg(windows)]
pub const PROGRAM: &str = "NebulaNative.exe";
#[cfg(not(windows))]
pub const PROGRAM: &str = "NebulaNative";

impl Installed {
    /// The installed game; none when no game.pak is found where the launcher looks.
    pub fn find() -> Option<Installed> {
        let data = data_folder()?;
        // NEBULA_INSTALL_ROOT names the install root in place of the registered and default ones (a test's private
        // install, beside its NEBULA_DATA).
        if let Some(root) = std::env::var_os("NEBULA_INSTALL_ROOT").map(PathBuf::from).filter(|p| p.is_absolute()) {
            return Self::find_in(&data, None, &root);
        }
        Self::find_in(&data, registered_root().as_deref(), &default_root(&data))
    }

    /// [`find`](Self::find) with its places given (tests give private ones).
    pub fn find_in(data: &Path, registered: Option<&Path>, default: &Path) -> Option<Installed> {
        let chosen = chosen_content(data);
        let roots: Vec<&Path> = registered.into_iter().chain(std::iter::once(default)).collect();
        // The root whose version runs: the registered one, else the default, whichever has a current.json.
        let root = roots.iter().find(|r| r.join("current.json").is_file()).map(|r| r.to_path_buf());
        let pak = chosen
            .map(|c| c.join("game.pak"))
            .or_else(|| roots.iter().map(|r| r.join("content").join("game.pak")).find(|p| p.is_file()))?;
        let version = root.as_deref().and_then(current_version);
        Some(Installed { pak, data: data.to_path_buf(), root, version })
    }

    /// The self-description folder of the version that runs, when it is there.
    pub fn self_description(&self) -> Option<PathBuf> {
        let folder = self.version.as_ref()?.join("luma");
        folder.is_dir().then_some(folder)
    }

    /// The program of the version that runs, when it is there.
    pub fn program(&self) -> Option<PathBuf> {
        let exe = self.version.as_ref()?.join(PROGRAM);
        exe.is_file().then_some(exe)
    }

    /// The player's saves (NebulaPaths.Saves).
    pub fn saves(&self) -> PathBuf {
        self.data.join("saves")
    }
}

/// Nebula's data folder on this system, as the game finds it; none when the system names no home.
pub fn data_folder() -> Option<PathBuf> {
    let named = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    if let Some(data) = named("NEBULA_DATA") {
        return Some(data);
    }
    if cfg!(windows) {
        return named("LOCALAPPDATA").map(|l| l.join("Nebula"));
    }
    if cfg!(target_vendor = "apple") {
        return named("HOME").map(|h| h.join("Library").join("Application Support").join("Nebula"));
    }
    named("XDG_DATA_HOME").map(|x| x.join("Nebula")).or_else(|| named("HOME").map(|h| h.join(".local").join("share").join("Nebula")))
}

/// The default install root: `%LOCALAPPDATA%\Programs\Nebula` on Windows (NebulaPaths.DefaultInstallRoot); elsewhere
/// the data folder, where a release unpacked by hand keeps its `content` and `versions`.
fn default_root(data: &Path) -> PathBuf {
    if cfg!(windows) {
        if let Some(parent) = data.parent() {
            return parent.join("Programs").join("Nebula");
        }
    }
    data.to_path_buf()
}

/// The content folder the player chose in the launcher, when it holds game.pak.
fn chosen_content(data: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(data.join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let content = PathBuf::from(value.get("content")?.as_str()?);
    content.join("game.pak").is_file().then_some(content)
}

/// `versions/<version>` of current.json's version, when that folder is there.
fn current_version(root: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(root.join("current.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let version = value.get("version")?.as_str()?;
    // A version is a name, never a path.
    if version.is_empty() || version.contains(['/', '\\']) || version.contains("..") {
        return None;
    }
    let folder = root.join("versions").join(version);
    folder.is_dir().then_some(folder)
}

/// The install root the last install recorded (`HKCU\Software\Nebula`, `InstallRoot`), read with the system's own
/// `reg` (no registry library); none elsewhere, or when there is none.
fn registered_root() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("reg").args(["query", r"HKCU\Software\Nebula", "/v", "InstallRoot"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    registry_value(&String::from_utf8_lossy(&out.stdout))
}

/// The value of `reg query`'s `InstallRoot    REG_SZ    <path>` line.
pub fn registry_value(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("InstallRoot")?.trim_start();
        let path = rest.strip_prefix("REG_EXPAND_SZ").or_else(|| rest.strip_prefix("REG_SZ"))?.trim();
        (!path.is_empty()).then(|| PathBuf::from(path))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pak(folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join("game.pak"), b"NPAK").unwrap();
    }

    #[test]
    fn the_chosen_content_comes_first_then_the_registered_root_then_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let (data, registered, default) = (dir.path().join("data"), dir.path().join("reg"), dir.path().join("default"));
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(Installed::find_in(&data, Some(&registered), &default), None);
        pak(&default.join("content"));
        assert_eq!(Installed::find_in(&data, Some(&registered), &default).unwrap().pak, default.join("content").join("game.pak"));
        pak(&registered.join("content"));
        assert_eq!(Installed::find_in(&data, Some(&registered), &default).unwrap().pak, registered.join("content").join("game.pak"));
        let chosen = dir.path().join("chosen");
        pak(&chosen);
        std::fs::write(data.join("settings.json"), serde_json::json!({ "content": chosen }).to_string()).unwrap();
        assert_eq!(Installed::find_in(&data, Some(&registered), &default).unwrap().pak, chosen.join("game.pak"));
        // A chosen folder without game.pak is passed over, as the launcher passes it over.
        std::fs::remove_file(chosen.join("game.pak")).unwrap();
        assert_eq!(Installed::find_in(&data, Some(&registered), &default).unwrap().pak, registered.join("content").join("game.pak"));
    }

    #[test]
    fn the_version_that_runs_is_current_json_s() {
        let dir = tempfile::tempdir().unwrap();
        let (data, root) = (dir.path().join("data"), dir.path().join("root"));
        pak(&root.join("content"));
        std::fs::create_dir_all(root.join("versions").join("2.0.0").join("luma")).unwrap();
        std::fs::write(root.join("versions").join("2.0.0").join(PROGRAM), b"").unwrap();
        std::fs::write(root.join("current.json"), r#"{"version":"2.0.0","previous":null}"#).unwrap();
        let found = Installed::find_in(&data, None, &root).unwrap();
        assert_eq!(found.version, Some(root.join("versions").join("2.0.0")));
        assert_eq!(found.self_description(), Some(root.join("versions").join("2.0.0").join("luma")));
        assert_eq!(found.program(), Some(root.join("versions").join("2.0.0").join(PROGRAM)));
        assert_eq!(found.saves(), data.join("saves"));
        std::fs::write(root.join("current.json"), r#"{"version":"../x"}"#).unwrap();
        assert_eq!(Installed::find_in(&data, None, &root).unwrap().version, None);
    }

    #[test]
    fn reg_query_s_line_is_read() {
        let text = "\r\nHKEY_CURRENT_USER\\Software\\Nebula\r\n    InstallRoot    REG_SZ    C:\\Users\\a b\\Programs\\Nebula\r\n\r\n";
        assert_eq!(registry_value(text), Some(PathBuf::from(r"C:\Users\a b\Programs\Nebula")));
        assert_eq!(registry_value("ERROR: none"), None);
    }
}
