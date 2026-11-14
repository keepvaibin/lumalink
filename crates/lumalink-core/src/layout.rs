//! Finds what a mod holds, however its folders are arranged: mods are published with a top folder or without, with
//! `riivolution/` beside their files, with the disc's folders loose, or with both.

use crate::riivolution;
use crate::texture_pack::{self, PackInfo};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// The disc's top-level folders in every release.
pub const DISC_FOLDERS: &[&str] = &[
    "AudioRes", "LayoutData", "MapPartsData", "MovieData", "ObjectData", "ParticleData", "StageData", "SystemData",
    "UsEnglish", "UsFrench", "UsSpanish", "EuEnglish", "EuFrench", "EuGerman", "EuItalian", "EuSpanish", "EuDutch",
    "JpJapanese", "KrKorean", "CnSimpChinese", "TwTradChinese", "HomeButton2",
];

/// Language folders: the disc of each region has only some of them.
pub const LANGUAGE_FOLDERS: &[&str] = &[
    "UsEnglish", "UsFrench", "UsSpanish", "EuEnglish", "EuFrench", "EuGerman", "EuItalian", "EuSpanish", "EuDutch",
    "JpJapanese", "KrKorean", "CnSimpChinese", "TwTradChinese",
];

const NOTE_EXTENSIONS: &[&str] = &["txt", "md", "pdf", "doc", "docx", "rtf", "png", "jpg", "jpeg", "gif", "webp", "url", "html", "htm", "nfo", "ini", "json", "xml", "csv", "bat", "sh", "exe"];

pub fn is_disc_folder(name: &str) -> bool {
    DISC_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(name))
}

pub fn is_note(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| NOTE_EXTENSIONS.contains(&e.as_str()))
}

fn child_any_case(dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

fn has_disc_folder(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|e| e.path().is_dir() && is_disc_folder(&e.file_name().to_string_lossy()))
        })
        .unwrap_or(false)
}

/// Whether a folder of this name holds another game's files (SB4, SB4E01; this game's start RMG).
pub fn is_other_game_folder(name: &str) -> bool {
    (name.len() == 3 || name.len() == 6)
        && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && name.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        && !name.starts_with("RMG")
}

/// One `.xml` patch file and the SD card root its paths start from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PatchFile {
    pub xml: PathBuf,
    pub sd_root: PathBuf,
    pub riivolution_dir: PathBuf,
}

/// What was found in a mod.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Layout {
    pub root: PathBuf,
    /// Riivolution patch files.
    pub patches: Vec<PatchFile>,
    /// A folder laid out as the disc is.
    pub files_root: Option<PathBuf>,
    /// Files with no place of their own: each goes where the game has a file of its name.
    pub loose: Vec<PathBuf>,
    pub pack: Option<PackInfo>,
    /// Compiled code for the Wii (`CustomCode_*.bin`, `.gct`, `.dol`, `.rel`).
    pub wii_code: Vec<PathBuf>,
}

impl Layout {
    pub fn of(root: &Path) -> Layout {
        let mut layout = Layout { root: root.to_path_buf(), ..Default::default() };
        // riivolution/ anywhere a few folders down: archives have their own top folders.
        for entry in WalkDir::new(root).max_depth(4).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_dir() || !entry.file_name().to_string_lossy().eq_ignore_ascii_case("riivolution") {
                continue;
            }
            let riivolution_dir = entry.path().to_path_buf();
            let sd_root = riivolution_dir.parent().unwrap_or(root).to_path_buf();
            let mut xmls: Vec<PathBuf> = fs::read_dir(&riivolution_dir)
                .map(|e| {
                    e.filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")))
                        .collect()
                })
                .unwrap_or_default();
            xmls.sort();
            for xml in xmls {
                layout.patches.push(PatchFile { xml, sd_root: sd_root.clone(), riivolution_dir: riivolution_dir.clone() });
            }
        }
        for entry in WalkDir::new(root).max_depth(8).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let extension = name.rsplit('.').next().unwrap_or("");
            if (name.starts_with("customcode") && name.ends_with(".bin")) || matches!(extension, "gct" | "dol" | "rel") || (extension == "ini" && name.len() >= 3 && name.starts_with("rmg")) {
                layout.wii_code.push(entry.path().to_path_buf());
            }
        }
        layout.wii_code.sort();
        if let Some(files) = child_any_case(root, "files").filter(|p| p.is_dir()) {
            layout.files_root = Some(files);
        } else if has_disc_folder(root) {
            layout.files_root = Some(root.to_path_buf());
        } else if layout.patches.is_empty() {
            // Disc folders further down: the one folder that has them, or the deepest common place.
            let mut with: Vec<PathBuf> = WalkDir::new(root)
                .min_depth(1)
                .max_depth(4)
                .into_iter()
                .filter_entry(|e| !is_other_game_folder(&e.file_name().to_string_lossy()))
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_dir() && has_disc_folder(e.path()))
                .map(|e| e.path().to_path_buf())
                .collect();
            with.sort_by_key(|p| p.components().count());
            layout.files_root = with.into_iter().next();
        }
        layout.pack = texture_pack::scan(root);
        if layout.patches.is_empty() && layout.files_root.is_none() {
            let mut loose: Vec<PathBuf> = WalkDir::new(root)
                .max_depth(3)
                .into_iter()
                .filter_entry(|e| !is_other_game_folder(&e.file_name().to_string_lossy()))
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .map(|e| e.path().to_path_buf())
                .filter(|p| !is_note(p) && !p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("tex1_")))
                .filter(|p| !layout.wii_code.contains(p))
                .collect();
            loose.sort();
            layout.loose = loose;
        }
        layout
    }

    /// The saved choices beside a patch file, for this disc.
    pub fn saved_choices(patch: &PatchFile, game_id: &str) -> std::collections::BTreeMap<String, usize> {
        let name = format!("{}.xml", game_id.get(..4).unwrap_or(game_id));
        child_any_case(&patch.riivolution_dir, "config")
            .and_then(|config| child_any_case(&config, &name))
            .and_then(|file| fs::read(file).ok())
            .map(|bytes| riivolution::parse_config(&riivolution::decode(&bytes)))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn riivolution_inside_a_top_folder() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Mod v1/riivolution/mod.xml");
        touch(dir.path(), "Mod v1/riivolution/config/RMGE.xml");
        touch(dir.path(), "Mod v1/ModFiles/StageData/a.arc");
        let layout = Layout::of(dir.path());
        assert_eq!(layout.patches.len(), 1);
        assert_eq!(layout.patches[0].sd_root, dir.path().join("Mod v1"));
        assert!(layout.files_root.is_none());
        assert!(layout.loose.is_empty());
    }

    #[test]
    fn disc_folders_at_the_top_or_below() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "ObjectData/a.arc");
        assert_eq!(Layout::of(dir.path()).files_root.as_deref(), Some(dir.path()));
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "top/game/ObjectData/a.arc");
        assert_eq!(Layout::of(dir.path()).files_root, Some(dir.path().join("top/game")));
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "files/ObjectData/a.arc");
        assert_eq!(Layout::of(dir.path()).files_root, Some(dir.path().join("files")));
    }

    #[test]
    fn another_games_folder_is_not_this_games_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "SB4/Neo/LayoutData/Buttons/a.png");
        touch(dir.path(), "SB4/Neo/ObjectData/b.arc");
        let layout = Layout::of(dir.path());
        assert!(layout.files_root.is_none());
        assert!(layout.loose.is_empty());
        assert!(is_other_game_folder("SB4E01") && is_other_game_folder("SB4"));
        assert!(!is_other_game_folder("RMGE01") && !is_other_game_folder("RMG") && !is_other_game_folder("Neo"));
    }

    #[test]
    fn loose_files_notes_and_code() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Kuribo.arc");
        touch(dir.path(), "readme.txt");
        touch(dir.path(), "preview.png");
        touch(dir.path(), "code/RMGE01.gct");
        touch(dir.path(), "CustomCode/CustomCode_USA.bin");
        let layout = Layout::of(dir.path());
        assert_eq!(layout.loose, [dir.path().join("Kuribo.arc")]);
        assert_eq!(layout.wii_code.len(), 2);
    }
}
