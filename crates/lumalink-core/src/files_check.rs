//! The check of a mod's files before it is packed or published: every `layers/<l>/files/<path>` (at `<path>`) and every
//! `layers/<l>/assets/<path>` (at `Luma/<id>/<path>`) cooked alone against the player's game by the function the
//! game's start cooks each file with (nebula-import's overlay::cook_source): models, animation files, collision,
//! cutscene cameras, pictures, streams, tables, layouts, sounds, fonts, movies, screen passes, and files of the game's
//! own forms. A file another file reads (a sound, font or movie beside its `.json`) makes nothing by itself there. A
//! `.luma`'s layer files and its luma.json are read from it in place as a folder (nebula-import's beside::packed), so
//! the files a source names beside it, and the mod's id and layers a layout or a surface shader looks up, are read as
//! from a mod folder. What the cook refuses or notes is the validator's L804 on that file, said in the player's
//! language as every other finding. Without the game the validator leaves these rules unchecked (L302, L303), and so does this.

use crate::luma_file;
use anyhow::Result;
use luma_validate::Finding;
use nebula_import::cook::audio_data::WavePlace;
use nebula_import::cook::beside;
use nebula_import::cook::overlay::{cook_source, is_sound_archive, mod_wave_layout};
use nebula_import::cook::overlay_patches::placement_args_finding;
use nebula_import::cook::refuse;
use nebula_import::store_view::PakStore;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;

/// The findings of the mod's source files at `source` (a folder or a `.luma`) against the game's store `store`.
pub fn findings(source: &Path, store: Option<&Path>, mod_id: Option<&str>) -> Result<Vec<Finding>> {
    let Some(store) = store else { return Ok(Vec::new()) };
    if source.is_dir() {
        return cooked_files(source, walkdir(source)?, store, mod_id);
    }
    // A .luma is read in place: its layer files stand under an empty folder, read from the file as the cook asks for
    // them (beside::packed), so nothing is written and each entry costs the same however many there are.
    let root = tempfile::tempdir()?;
    let luma = Rc::new(RefCell::new(luma_file::Reader::open(source)?));
    let mut names: Vec<String> = luma
        .borrow()
        .list()
        .into_iter()
        .map(|entry| entry.name)
        // Only layer files are cooked; luma.json is there for the cook to find the mod's folder by (a layout's own font,
        // a material's surface shader), as in a mod folder. A name that would leave the folder is the container rules'
        // (L1xx).
        .filter(|name| {
            name == "luma.json"
                || (name.starts_with("layers/") && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..") && !name.contains(['\\', ':']))
        })
        .collect();
    names.sort();
    names.dedup();
    let reader = Rc::clone(&luma);
    let _packed = beside::packed(root.path(), names.clone(), move |name| reader.borrow_mut().read(name).ok());
    cooked_files(root.path(), names, store, mod_id)
}

/// The findings of the files `names` (relative to `source`, in name order), each read through `beside`.
fn cooked_files(source: &Path, names: Vec<String>, store: &Path, mod_id: Option<&str>) -> Result<Vec<Finding>> {
    let mut out = Vec::new();
    let mut game: Option<PakStore> = None;
    // A wave archive follows the wave systems of the sound archive its mod brings (AudioRes/SMR.szs: its layer's, else
    // the base layer's), not the game's, as the game's start lays it out (overlay::write_overlay). A sound archive whose
    // wave systems do not read is said on each wave archive it would lay out.
    let mut sounds: HashMap<String, Result<BTreeMap<String, Vec<WavePlace>>, Finding>> = HashMap::new();
    for name in &names {
        let mut parts = name.splitn(4, '/');
        if let (Some("layers"), Some(layer), Some("files"), Some(path)) = (parts.next(), parts.next(), parts.next(), parts.next()) {
            if is_sound_archive(path) {
                refuse::take_notes();
                let layout = mod_wave_layout(&beside::read(&source.join(name))?);
                refuse::take_notes();
                let layout = match layout {
                    Ok(waves) => Ok(waves),
                    Err(error) => match refuse::refusal_of(&error) {
                        Some(finding) => Err(finding.clone()),
                        None => return Err(error),
                    },
                };
                sounds.insert(layer.to_string(), layout);
            }
        }
    }
    for name in names {
        let mut parts = name.splitn(4, '/');
        let (Some("layers"), Some(layer), Some(place), Some(path)) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
        // A patch of a stage's placement table whose rows give arguments by name: applied to the game's table as the
        // start applies it (docs/modding/content/stages.md, section 1). Not JSON, or any other fault, is the validator's.
        if place == "patches" {
            let Some(table) = path.strip_suffix(".json") else { continue };
            let Ok(patch) = serde_json::from_slice::<serde_json::Value>(&beside::read(&source.join(&name))?) else { continue };
            if game.is_none() {
                game = Some(PakStore::open(store)?);
            }
            let Some(pak) = game.as_ref() else { continue };
            if let Some(mut finding) = placement_args_finding(pak, table, mod_id.unwrap_or("mod"), &name, &patch)? {
                finding.file = name.clone();
                finding.mod_id = mod_id.map(str::to_string);
                out.push(finding);
            }
            continue;
        }
        let game_path = match place {
            "files" => path.to_string(),
            // A layer's assets are the mod's own files in the store (Luma/<id>/<path>).
            "assets" => format!("Luma/{}/{path}", mod_id.unwrap_or("mod")),
            _ => continue,
        };
        let at = source.join(&name);
        let bytes = beside::read(&at)?;
        let sound = if place == "files" && game_path.to_ascii_lowercase().ends_with(".aw") { sounds.get(layer).or_else(|| sounds.get("base")) } else { None };
        refuse::take_notes();
        let mut found = match sound {
            Some(Err(finding)) => vec![finding.clone()],
            Some(Ok(waves)) => cooked(cook_source(&game_path, &at, &bytes, Some(store), Some(waves)))?,
            None => cooked(cook_source(&game_path, &at, &bytes, Some(store), None))?,
        };
        found.extend(refuse::take_notes());
        // What the cook made is the game's to record at its start, not this check's.
        refuse::take_facts();
        for finding in &mut found {
            finding.file = name.clone();
            finding.mod_id = mod_id.map(str::to_string);
        }
        out.extend(found);
    }
    Ok(out)
}

/// What cook_source said of a file: nothing, or its refusal (a fault that is no refusal is this check's own error).
fn cooked(result: Result<usize>) -> Result<Vec<Finding>> {
    match result {
        Ok(_) => Ok(Vec::new()),
        Err(error) => match refuse::refusal_of(&error) {
            Some(finding) => Ok(vec![finding.clone()]),
            None => Err(error),
        },
    }
}

/// A folder's files, relative and with `/` between parts, in name order.
fn walkdir(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_files_are_cooked_against_the_game_and_said_on_their_file() {
        let dir = tempfile::tempdir().unwrap();
        let put = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        put("luma.json", br#"{"layers":[{"name":"base"}]}"#);
        // An animation file outside any archive: refused by name, on its file.
        put("layers/base/files/ObjectData/Walk.anim.gltf", br#"{"asset":{"version":"2.0"}}"#);
        // A source kind the cook places as it is (the validator's L304 names where it goes) and a sound file a
        // .sound.json would read: nothing from the cook.
        put("layers/base/files/Fonts/Title.ttf", b"font");
        put("layers/base/assets/Song.wav", b"");
        // Files in assets/ are cooked as the game's start cooks them: an animation file outside an archive, an empty
        // model and an empty file of no source kind are each refused.
        put("layers/base/assets/Kuribo.anim.gltf", b"{}");
        put("layers/base/assets/Ball.glb", b"");
        put("layers/base/assets/Notes.bin", b"");
        // Without the game nothing is said (the validator leaves L302 and L303 unchecked).
        assert!(findings(dir.path(), None, Some("walk")).unwrap().is_empty());
        let store = dir.path().join("game.pak");
        std::fs::write(&store, b"not read: the file is refused before the store is opened").unwrap();
        let found = findings(dir.path(), Some(&store), Some("walk")).unwrap();
        let said: Vec<(u16, &str, Option<&str>)> = found.iter().map(|f| (f.code, f.file.as_str(), f.mod_id.as_deref())).collect();
        assert_eq!(
            said,
            [
                (804, "layers/base/assets/Ball.glb", Some("walk")),
                (804, "layers/base/assets/Kuribo.anim.gltf", Some("walk")),
                (804, "layers/base/assets/Notes.bin", Some("walk")),
                (804, "layers/base/files/ObjectData/Walk.anim.gltf", Some("walk")),
            ],
            "{found:?}"
        );
        assert!(luma_validate::reason(&found[3], "UsEnglish").contains("archive"), "{}", luma_validate::reason(&found[3], "UsEnglish"));
        assert!(luma_validate::reason(&found[2], "UsEnglish").contains("empty"), "{}", luma_validate::reason(&found[2], "UsEnglish"));
    }

    /// A 16-bit mono 48 kHz WAV of `frames` silent frames.
    fn wav(frames: u32) -> Vec<u8> {
        let mut out = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        for v in [16u32, 0x0001_0001, 48000, 96000, 0x0010_0002] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(frames * 2).to_le_bytes());
        out.resize(out.len() + frames as usize * 2, 0);
        let n = out.len() as u32 - 8;
        out[4..8].copy_from_slice(&n.to_le_bytes());
        out
    }

    #[test]
    fn sound_effects_in_assets_are_cooked_with_the_files_beside_them_in_a_folder_or_a_luma() {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("mod");
        let put = |name: &str, bytes: &[u8]| {
            let path = mod_dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        put("luma.json", br#"{"layers":[{"name":"base"}]}"#);
        put("layers/base/assets/sfx/Jump.sound.json", br#"{"kind":"effect","file":"jump.wav"}"#);
        put("layers/base/assets/sfx/jump.wav", &wav(4800));
        put("layers/base/assets/sfx/Ding.sound.json", br#"{"kind":"effect","file":"ding.wav"}"#);
        // A sound effect at a game path: refused by name (it belongs in assets/).
        put("layers/base/files/AudioRes/Jump.sound.json", br#"{"kind":"effect","file":"jump.wav"}"#);
        // A font whose font file the mod lacks, and a font at a game path.
        put("layers/base/assets/fonts/Title.font.json", br#"{"file":"Title.ttf"}"#);
        put("layers/base/files/LayoutData/Title.font.json", br#"{"file":"Title.ttf"}"#);
        let store = dir.path().join("game.pak");
        std::fs::write(&store, b"not read").unwrap();
        let luma = dir.path().join("sfx.luma");
        crate::luma_file::write(&crate::luma_file::folder_files(&mod_dir).unwrap(), &luma).unwrap();
        for source in [mod_dir.as_path(), luma.as_path()] {
            let found = findings(source, Some(&store), Some("sfx.mod")).unwrap();
            let said: Vec<(u16, &str, String)> = found.iter().map(|f| (f.code, f.file.as_str(), luma_validate::reason(f, "UsEnglish"))).collect();
            assert_eq!(said.len(), 4, "{said:?}");
            assert_eq!((said[0].0, said[0].1), (804, "layers/base/assets/fonts/Title.font.json"), "{said:?}");
            assert!(said[0].2.contains("Title.ttf"), "{said:?}");
            assert_eq!((said[1].0, said[1].1), (804, "layers/base/assets/sfx/Ding.sound.json"), "{said:?}");
            assert!(said[1].2.contains("ding.wav"), "{said:?}");
            assert_eq!((said[2].0, said[2].1), (804, "layers/base/files/AudioRes/Jump.sound.json"), "{said:?}");
            assert!(said[2].2.contains("assets"), "{said:?}");
            assert_eq!((said[3].0, said[3].1), (804, "layers/base/files/LayoutData/Title.font.json"), "{said:?}");
            assert!(said[3].2.contains("assets"), "{said:?}");
        }
    }

    /// A sound archive of one wave system: `Wave_0.aw` holds one PCM16 wave of 2 bytes at offset 2.
    fn sound_archive() -> Vec<u8> {
        let mut out = vec![0u8; 0xE0];
        let mut put = |at: usize, value: u32| out[at..at + 4].copy_from_slice(&value.to_be_bytes());
        put(4, 0xE0);
        put(0x10, 0x20);
        put(0x24, 1);
        put(0x28, 0x40);
        put(0xB0, 1);
        put(0xB4, 0xC0);
        put(0xC8, 2);
        put(0xCC, 2);
        put(0xDC, 1);
        out[..4].copy_from_slice(b"WSYS");
        out[0x40..0x49].copy_from_slice(b"Wave_0.aw");
        out[0xC1] = 3;
        out
    }

    #[test]
    fn wave_archives_are_laid_out_by_the_sound_archive_the_mod_brings_in_a_folder_or_a_luma() {
        // The corpus's Peach and Rosalina mods bring AudioRes/SMR.szs and wave archives shorter than the game's: laid
        // out by the game's wave systems they would be waves past their end. The mod's own sound archive lays them out,
        // the base layer's for a layer that brings none, as the game's start does (overlay::write_overlay).
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("mod");
        let put = |name: &str, bytes: &[u8]| {
            let path = mod_dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        put("luma.json", br#"{"layers":[{"name":"base"},{"name":"short"}]}"#);
        put("layers/base/files/AudioRes/Waves/Wave_0.aw", &[1, 2, 3, 4]);
        put("layers/short/files/AudioRes/Waves/Wave_0.aw", &[1, 2]);
        // The game's store has no wave layout to read: only the mod's sound archive can lay the waves out.
        let store = dir.path().join("game.pak");
        std::fs::write(&store, b"not read").unwrap();
        let said = |source: &Path| -> Vec<(String, String)> {
            findings(source, Some(&store), Some("voice")).unwrap().iter().map(|f| (f.file.clone(), luma_validate::reason(f, "UsEnglish"))).collect()
        };
        let without = said(&mod_dir);
        assert_eq!(without.len(), 2, "{without:?}");
        assert!(without.iter().all(|(_, why)| why.contains("wave layout is not known")), "{without:?}");
        put("layers/base/files/AudioRes/SMR.szs", &sound_archive());
        let luma = dir.path().join("voice.luma");
        crate::luma_file::write(&crate::luma_file::folder_files(&mod_dir).unwrap(), &luma).unwrap();
        for source in [mod_dir.as_path(), luma.as_path()] {
            let found = said(source);
            assert_eq!(found.len(), 1, "{found:?}");
            assert_eq!(found[0].0, "layers/short/files/AudioRes/Waves/Wave_0.aw");
            assert!(found[0].1.contains("past the file's end"), "{found:?}");
        }
    }

    #[test]
    fn a_luma_of_many_entries_is_read_in_place_as_its_folder() {
        // Found by fuzzing the container: a .luma of 20,000 extra entries was unpacked to disk before its files were
        // cooked, over 30 s. Read in place, it says what its folder says, buffers beside a model included.
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("mod");
        let put = |name: &str, bytes: &[u8]| {
            let path = mod_dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        put("luma.json", br#"{"layers":[{"name":"base"}]}"#);
        let model = |uri: &str| format!(r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"uri":"{uri}","byteLength":4}}]}}"#);
        put("layers/base/assets/Ball.gltf", model("parts/Ball.bin").as_bytes());
        put("layers/base/assets/parts/Ball.bin", b"\0\0\0\0");
        put("layers/base/assets/Hole.gltf", model("parts/None.bin").as_bytes());
        for i in 0..2000 {
            put(&format!("layers/base/files/f{i}.bin"), b"");
        }
        let store = dir.path().join("game.pak");
        std::fs::write(&store, b"not read").unwrap();
        let luma = dir.path().join("many.luma");
        crate::luma_file::write(&crate::luma_file::folder_files(&mod_dir).unwrap(), &luma).unwrap();
        let said = |source: &Path| -> Vec<(u16, String, String)> {
            findings(source, Some(&store), Some("many")).unwrap().iter().map(|f| (f.code, f.file.clone(), luma_validate::reason(f, "UsEnglish"))).collect()
        };
        let (folder, packed) = (said(&mod_dir), said(&luma));
        assert_eq!(folder, packed);
        assert_eq!(packed.iter().filter(|(code, file, _)| *code == 804 && file.starts_with("layers/base/files/f")).count(), 2000);
        let on = |file: &str| packed.iter().filter(|(_, f, _)| f == file).map(|(_, _, why)| why.clone()).collect::<Vec<_>>().join(" | ");
        assert!(on("layers/base/assets/Hole.gltf").contains("parts/None.bin"), "{}", on("layers/base/assets/Hole.gltf"));
        assert!(!on("layers/base/assets/Ball.gltf").contains("parts/Ball.bin"), "{}", on("layers/base/assets/Ball.gltf"));
    }
}
