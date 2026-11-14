//! The check of a mod's effects (docs/modding/content/effects.md, section 7) before it is packed or published: every layer's
//! `assets/effects.json`, cooked as the game's start check cooks them (nebula-import's effects cook, layers in the
//! order luma.json gives). With the player's game given, `like`, `replace` and texture names are its particles'; with
//! none, any name stands for one of the game's. What the cook refuses or notes is the validator's L804 on the last
//! layer's file, said in the player's language as every other finding.

use crate::luma_file;
use anyhow::Result;
use luma_validate::Finding;
use nebula_import::cook::effects;
use nebula_import::cook::particle_json::{names_index, Game};
use nebula_import::cook::particles::{read_nptc, ParticleIr};
use nebula_import::cook::refuse;
use std::collections::HashMap;
use std::path::Path;

/// A folder's or a `.luma`'s files by name.
trait Files {
    fn read(&self, name: &str) -> Option<Vec<u8>>;
}

struct Folder<'a>(&'a Path);

impl Files for Folder<'_> {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(name)).ok()
    }
}

/// A `.luma`, opened once for all of its layers' files.
struct Luma(std::cell::RefCell<luma_file::Reader>);

impl Files for Luma {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let mut luma = self.0.borrow_mut();
        if luma.has(name) {
            luma.read(name).ok()
        } else {
            None
        }
    }
}

/// The mod's layers in luma.json's order (`base` when it lists none).
fn layers(files: &dyn Files) -> Vec<String> {
    let listed = files
        .read("luma.json")
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("layers").and_then(|l| l.as_array()).cloned())
        .map(|l| l.iter().filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(str::to_string)).collect::<Vec<_>>());
    listed.unwrap_or_else(|| vec!["base".to_string()])
}

/// The findings of the mod's effects at `source` (a folder or a `.luma`), with the game's store when given.
pub fn findings(source: &Path, store: Option<&Path>, mod_id: Option<&str>) -> Result<Vec<Finding>> {
    let folder;
    let luma;
    let files: &dyn Files = if source.is_dir() {
        folder = Folder(source);
        &folder
    } else {
        luma = Luma(std::cell::RefCell::new(luma_file::Reader::open(source)?));
        &luma
    };
    let layers = layers(files);
    let sources: Vec<(String, Vec<u8>)> = layers
        .iter()
        .filter_map(|layer| {
            let name = format!("layers/{layer}/assets/effects.json");
            files.read(&name).map(|bytes| (name, bytes))
        })
        .collect();
    let Some((last, _)) = sources.last() else { return Ok(Vec::new()) };
    let last = last.clone();
    let (particles, names) = match store {
        Some(store) => {
            use nebula_import::cook::store::read_file;
            (read_nptc(&read_file(store, effects::PARTICLES)?)?, names_index(&read_file(store, effects::PARTICLE_NAMES)?)?)
        }
        None => (ParticleIr::default(), HashMap::new()),
    };
    let game = Game { particles: &particles, names: &names, any_name: store.is_none() };
    // A texture's file: the last layer that has it.
    let mut image = |path: &str| layers.iter().rev().find_map(|layer| files.read(&format!("layers/{layer}/assets/{path}")));
    let bytes: Vec<Vec<u8>> = sources.into_iter().map(|(_, b)| b).collect();
    refuse::take_notes();
    let mut out = match effects::cook(&bytes, &mut image, &game) {
        Ok(_) => Vec::new(),
        Err(error) => match refuse::refusal_of(&error) {
            Some(finding) => vec![finding.clone()],
            None => return Err(error),
        },
    };
    out.extend(refuse::take_notes());
    for finding in &mut out {
        finding.file = last.clone();
        finding.mod_id = mod_id.map(str::to_string);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mods_effects_are_cooked_in_layer_order_and_said_on_the_last_layer() {
        let dir = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        put("luma.json", r#"{"layers":[{"name":"base"},{"name":"night","when":{}}]}"#);
        put("layers/base/assets/effects.json", r#"{"effects":{"aura":{"particles":[{"like":"Glow"}]}}}"#);
        assert!(findings(dir.path(), None, Some("gold.mario")).unwrap().is_empty());
        // The later layer's aura is used: its fault is said, on its file.
        put("layers/night/assets/effects.json", r#"{"effects":{"aura":{"particles":[{"textures":["Ring"]}]}}}"#);
        let found = findings(dir.path(), None, Some("gold.mario")).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].code, found[0].file.as_str(), found[0].mod_id.as_deref()), (804, "layers/night/assets/effects.json", Some("gold.mario")));
        let words = luma_validate::reason(&found[0], "UsEnglish");
        assert!(words.contains("/effects/aura/particles/0/textures") && words.contains("sprite"), "{words}");
        // A texture's file the mod lacks.
        put("layers/night/assets/effects.json", r#"{"textures":{"Gold":"gold.png"},"effects":{"aura":{"particles":[{"textures":{"sprite":"Gold"}}]}}}"#);
        let found = findings(dir.path(), None, None).unwrap();
        assert!(luma_validate::reason(&found[0], "UsEnglish").contains("gold.png"), "{found:?}");
    }
}
