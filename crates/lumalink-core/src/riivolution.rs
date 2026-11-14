//! Riivolution patch files: the `<wiidisc>` XML most Wii mods are published with, and the choices saved beside them.

use crate::say;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FilePatch {
    pub disc: String,
    pub external: String,
    pub resize: bool,
    pub create: bool,
    pub offset: u64,
    pub file_offset: u64,
    pub length: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FolderPatch {
    pub disc: String,
    pub external: String,
    pub resize: bool,
    pub create: bool,
    pub recursive: bool,
    pub length: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SavePatch {
    pub external: String,
    pub clone: bool,
}

/// A change to the console's memory: code or data of the Wii program.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MemoryPatch {
    pub offset: u32,
    pub value: Vec<u8>,
    pub value_file: String,
    pub original: Vec<u8>,
    pub ocarina: bool,
    pub search: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Patch {
    pub id: String,
    pub root: String,
    pub files: Vec<FilePatch>,
    pub folders: Vec<FolderPatch>,
    pub saves: Vec<SavePatch>,
    pub memory: Vec<MemoryPatch>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PatchRef {
    pub id: String,
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Choice {
    pub name: String,
    pub patches: Vec<PatchRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct OptionEntry {
    pub id: String,
    pub name: String,
    pub choices: Vec<Choice>,
    /// 0: off; otherwise the choice, counted from 1.
    pub selected: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Section {
    pub name: String,
    pub options: Vec<OptionEntry>,
}

/// What a patch file says it is for.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Target {
    pub game: Option<String>,
    pub developer: Option<String>,
    pub disc: Option<i64>,
    pub version: Option<i64>,
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Disc {
    pub target: Target,
    pub root: String,
    pub sections: Vec<Section>,
    pub patches: Vec<Patch>,
}

/// A number as Riivolution writes one: decimal, or hexadecimal with 0x.
fn number(text: &str) -> Option<u64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

fn boolean(text: Option<&str>, default: bool) -> bool {
    match text.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        Some("true" | "yes" | "1") => true,
        Some("false" | "no" | "0") => false,
        _ => default,
    }
}

/// Hexadecimal bytes, with or without 0x; an odd count of digits or another character gives none.
pub fn hex_bytes(text: &str) -> Vec<u8> {
    let text = text.trim();
    let text = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    if text.len() % 2 != 0 || !text.is_ascii() {
        return Vec::new();
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap_or_default()
}

fn params_of(node: roxmltree::Node, mut params: BTreeMap<String, String>) -> BTreeMap<String, String> {
    for param in node.children().filter(|c| c.has_tag_name("param")) {
        params.insert(param.attribute("name").unwrap_or("").to_string(), param.attribute("value").unwrap_or("").to_string());
    }
    params
}

/// Text in the encodings patch files turn up in: UTF-8 with or without a mark, UTF-16 with one, else Latin-1.
pub fn decode(bytes: &[u8]) -> String {
    if let [0xFF, 0xFE, rest @ ..] = bytes {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let [0xFE, 0xFF, rest @ ..] = bytes {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

pub fn parse(text: &str) -> Result<Disc> {
    let document = roxmltree::Document::parse_with_options(
        text,
        // Some mods carry a DTD. Nested entities can't make more than a mod's XML ever needs.
        roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: 2_000_000 },
    )
    .map_err(|e| anyhow!(say!("LumaLink.Riivolution.NotXml", why = e)))?;
    let root = document.root_element();
    if !root.has_tag_name("wiidisc") {
        bail!(say!("LumaLink.Riivolution.NotPatch"));
    }
    let version = root.attribute("version").and_then(number).with_context(|| say!("LumaLink.Riivolution.NoVersion"))?;
    if version != 1 {
        bail!(say!("LumaLink.Riivolution.Version", version = version));
    }
    let mut disc = Disc { root: root.attribute("root").unwrap_or("").to_string(), ..Default::default() };
    if let Some(id) = root.children().find(|c| c.has_tag_name("id")) {
        disc.target.game = id.attribute("game").map(str::to_string);
        disc.target.developer = id.attribute("developer").map(str::to_string);
        disc.target.disc = id.attribute("disc").and_then(number).map(|n| n as i64);
        disc.target.version = id.attribute("version").and_then(number).map(|n| n as i64);
        disc.target.regions = id
            .children()
            .filter(|c| c.has_tag_name("region"))
            .filter_map(|r| r.attribute("type").map(str::to_string))
            .collect();
    }
    if let Some(options) = root.children().find(|c| c.has_tag_name("options")) {
        for s in options.children().filter(|c| c.has_tag_name("section")) {
            let mut section = Section { name: s.attribute("name").unwrap_or("").to_string(), options: Vec::new() };
            for o in s.children().filter(|c| c.has_tag_name("option")) {
                let option_params = params_of(o, BTreeMap::new());
                let mut option = OptionEntry {
                    id: o.attribute("id").unwrap_or("").to_string(),
                    name: o.attribute("name").unwrap_or("").to_string(),
                    choices: Vec::new(),
                    selected: o.attribute("default").and_then(number).unwrap_or(0) as usize,
                };
                for c in o.children().filter(|c| c.has_tag_name("choice")) {
                    let choice_params = params_of(c, option_params.clone());
                    option.choices.push(Choice {
                        name: c.attribute("name").unwrap_or("").to_string(),
                        patches: c
                            .children()
                            .filter(|p| p.has_tag_name("patch"))
                            .map(|p| PatchRef {
                                id: p.attribute("id").unwrap_or("").to_string(),
                                params: params_of(p, choice_params.clone()),
                            })
                            .collect(),
                    });
                }
                // A default past the last choice chooses none, as Riivolution shows it.
                if option.selected > option.choices.len() {
                    option.selected = 0;
                }
                section.options.push(option);
            }
            disc.sections.push(section);
        }
        // A macro: an option cloned under another name, with other parameters.
        for m in options.children().filter(|c| c.has_tag_name("macros")) {
            let wanted = m.attribute("id").unwrap_or("");
            for section in &mut disc.sections {
                if let Some(option) = section.options.iter().find(|o| o.id == wanted).cloned() {
                    let mut clone = option;
                    clone.name = m.attribute("name").unwrap_or("").to_string();
                    for choice in &mut clone.choices {
                        for patch in &mut choice.patches {
                            patch.params = params_of(m, std::mem::take(&mut patch.params));
                        }
                    }
                    section.options.push(clone);
                }
            }
        }
    }
    for p in root.children().filter(|c| c.has_tag_name("patch")) {
        let mut patch = Patch {
            id: p.attribute("id").unwrap_or("").to_string(),
            root: p.attribute("root").map(str::to_string).unwrap_or_else(|| disc.root.clone()),
            ..Default::default()
        };
        for n in p.children().filter(|c| c.is_element()) {
            let text = |name: &str| n.attribute(name).unwrap_or("").to_string();
            let count = |name: &str| n.attribute(name).and_then(number).unwrap_or(0);
            match n.tag_name().name() {
                "file" => patch.files.push(FilePatch {
                    disc: text("disc"),
                    external: text("external"),
                    resize: boolean(n.attribute("resize"), true),
                    create: boolean(n.attribute("create"), false),
                    offset: count("offset"),
                    file_offset: count("fileoffset"),
                    length: count("length"),
                }),
                "folder" => patch.folders.push(FolderPatch {
                    disc: text("disc"),
                    external: text("external"),
                    resize: boolean(n.attribute("resize"), true),
                    create: boolean(n.attribute("create"), false),
                    recursive: boolean(n.attribute("recursive"), true),
                    length: count("length"),
                }),
                "savegame" => patch.saves.push(SavePatch { external: text("external"), clone: boolean(n.attribute("clone"), true) }),
                "memory" => patch.memory.push(MemoryPatch {
                    offset: count("offset") as u32,
                    value: hex_bytes(&text("value")),
                    value_file: text("valuefile"),
                    original: hex_bytes(&text("original")),
                    ocarina: boolean(n.attribute("ocarina"), false),
                    search: boolean(n.attribute("search"), false),
                }),
                _ => {}
            }
        }
        disc.patches.push(patch);
    }
    Ok(disc)
}

impl Target {
    /// Whether a patch file is for this disc (a six-letter id such as RMGE01).
    pub fn accepts(&self, game_id: &str) -> bool {
        if game_id.len() != 6 {
            return false;
        }
        if self.game.as_deref().is_some_and(|g| !game_id.starts_with(g)) {
            return false;
        }
        if self.developer.as_deref().is_some_and(|d| game_id.get(4..6) != Some(d)) {
            return false;
        }
        self.regions.is_empty() || self.regions.iter().any(|r| game_id.get(3..4) == Some(r.as_str()))
    }
}

impl OptionEntry {
    /// A question that only asks which version of the game this is: nobody needs to answer it.
    pub fn asks_for_region(&self) -> bool {
        self.choices.len() >= 2 && self.choices.iter().all(|c| choice_region(&c.name).is_some())
    }
}

/// The region a choice's name means: NTSC, PAL, JP and the many ways mods write them.
pub fn choice_region(name: &str) -> Option<char> {
    let lower = name.to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for (i, word) in words.iter().enumerate() {
        let next = words.get(i + 1).copied().unwrap_or("");
        match *word {
            "ntscj" | "jp" | "jpn" | "japan" | "jap" | "japanese" => return Some('J'),
            "ntsc" if next == "j" => return Some('J'),
            "ntscu" | "ntsc" | "usa" | "us" | "na" | "america" | "americas" => return Some('E'),
            "pal" | "eur" | "eu" | "europe" | "uk" => return Some('P'),
            "kor" | "korea" | "kr" | "ntsck" => return Some('K'),
            _ => {}
        }
    }
    None
}

impl OptionEntry {
    /// An option whose every choice names a region: the choice for this disc, from 1.
    pub fn region_choice(&self, game_id: &str) -> Option<usize> {
        let wanted = game_id.chars().nth(3)?;
        if self.choices.len() < 2 {
            return None;
        }
        let regions: Vec<char> = self.choices.iter().map(|c| choice_region(&c.name)).collect::<Option<_>>()?;
        regions.iter().position(|&r| r == wanted).map(|i| i + 1)
    }

    /// How Riivolution's saved choices name this option.
    pub fn key(&self, section: &Section) -> String {
        if self.id.is_empty() {
            format!("{}{}", section.name, self.name)
        } else {
            self.id.clone()
        }
    }
}

impl Disc {
    /// Choices a person would not need to be asked: a region option takes this disc's region, and a mod's one option
    /// with one choice is the mod itself. Saved choices (`riivolution/config/<id>.xml`) come first.
    pub fn choose_defaults(&mut self, game_id: &str, saved: &BTreeMap<String, usize>) {
        let total: usize = self.sections.iter().map(|s| s.options.len()).sum();
        let keys: Vec<Vec<String>> = self.sections.iter().map(|s| s.options.iter().map(|o| o.key(s)).collect()).collect();
        for (s, section) in self.sections.iter_mut().enumerate() {
            for (o, option) in section.options.iter_mut().enumerate() {
                if let Some(region) = option.region_choice(game_id) {
                    option.selected = region;
                } else if let Some(&choice) = saved.get(&keys[s][o]) {
                    option.selected = choice.min(option.choices.len());
                } else if total == 1 && option.choices.len() == 1 && option.selected == 0 {
                    option.selected = 1;
                }
            }
        }
    }

    /// The patches of the selected choices, placeholders filled.
    pub fn active_patches(&self, game_id: &str) -> Vec<Patch> {
        let mut active = Vec::new();
        for option in self.sections.iter().flat_map(|s| &s.options) {
            let Some(choice) = option.selected.checked_sub(1).and_then(|i| option.choices.get(i)) else {
                continue;
            };
            for reference in &choice.patches {
                let Some(patch) = self.patches.iter().find(|p| p.id == reference.id) else {
                    continue;
                };
                let mut with: Vec<(String, String)> = vec![
                    ("{$__gameid}".into(), game_id.get(..3).unwrap_or("").into()),
                    ("{$__region}".into(), game_id.get(3..4).unwrap_or("").into()),
                    ("{$__maker}".into(), game_id.get(4..6).unwrap_or("").into()),
                ];
                with.extend(reference.params.iter().map(|(k, v)| (format!("{{${k}}}"), v.clone())));
                let fill = |text: &str| with.iter().fold(text.to_string(), |t, (from, to)| t.replace(from, to));
                let mut patch = patch.clone();
                patch.root = fill(&patch.root);
                for f in &mut patch.files {
                    f.disc = fill(&f.disc);
                    f.external = fill(&f.external);
                }
                for f in &mut patch.folders {
                    f.disc = fill(&f.disc);
                    f.external = fill(&f.external);
                }
                for s in &mut patch.saves {
                    s.external = fill(&s.external);
                }
                active.push(patch);
            }
        }
        active
    }
}

/// Riivolution's saved choices (`<riivolution version="2">`), by option key.
pub fn parse_config(text: &str) -> BTreeMap<String, usize> {
    let mut saved = BTreeMap::new();
    let Ok(document) = roxmltree::Document::parse(text) else {
        return saved;
    };
    let root = document.root_element();
    if root.has_tag_name("riivolution") {
        for option in root.children().filter(|c| c.has_tag_name("option")) {
            if let Some(id) = option.attribute("id") {
                saved.insert(id.to_string(), option.attribute("default").and_then(number).unwrap_or(0) as usize);
            }
        }
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<wiidisc version="1" root="/base">
  <id game="RMG" developer="01"><region type="E"/><region type="P"/></id>
  <options>
    <section name="Mod">
      <option name="Select Your Region:">
        <choice name="NTSC"><patch id="all"/><patch id="us"><param name="lang" value="UsEnglish"/></patch></choice>
        <choice name="PAL"><patch id="all"/></choice>
      </option>
      <option id="extra" name="Extra" default="1">
        <param name="fromoption" value="1"/>
        <choice name="On"><patch id="extra"/></choice>
      </option>
    </section>
    <macros id="extra" name="Extra copy"><param name="more" value="2"/></macros>
  </options>
  <patch id="all"><folder disc="/StageData" external="/Mod/StageData" create="true"/></patch>
  <patch id="us" root="/other">
    <file disc="/{$lang}/a.arc" external="{$__gameid}{$__region}{$__maker}.arc" offset="0x10"/>
    <savegame external="/save/{$__gameid}" clone="false"/>
    <memory offset="0x80001800" value="38600001" original="60000000"/>
    <memory valuefile="code.gct" ocarina="true"/>
  </patch>
  <patch id="extra"/>
</wiidisc>"#;

    #[test]
    fn a_patch_file_reads_whole() {
        let disc = parse(SAMPLE).unwrap();
        assert_eq!(disc.root, "/base");
        assert_eq!(disc.target.game.as_deref(), Some("RMG"));
        assert_eq!(disc.target.regions, ["E", "P"]);
        assert_eq!(disc.sections[0].options.len(), 3);
        assert_eq!(disc.sections[0].options[2].name, "Extra copy");
        assert_eq!(disc.sections[0].options[2].choices[0].patches[0].params["more"], "2");
        assert_eq!(disc.sections[0].options[2].choices[0].patches[0].params["fromoption"], "1");
        let us = &disc.patches[1];
        assert_eq!(us.root, "/other");
        assert_eq!(us.files[0].offset, 0x10);
        assert!(us.files[0].resize);
        assert_eq!(us.memory[0].offset, 0x8000_1800);
        assert_eq!(us.memory[0].value, [0x38, 0x60, 0, 1]);
        assert!(us.memory[1].ocarina);
        assert_eq!(us.memory[1].value_file, "code.gct");
        assert_eq!(disc.patches[0].root, "/base");
    }

    #[test]
    fn a_region_option_takes_the_disc_and_placeholders_fill() {
        let mut disc = parse(SAMPLE).unwrap();
        disc.choose_defaults("RMGE01", &BTreeMap::new());
        assert_eq!(disc.sections[0].options[0].selected, 1);
        let active = disc.active_patches("RMGE01");
        assert_eq!(active.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["all", "us", "extra", "extra"]);
        assert_eq!(active[1].files[0].disc, "/UsEnglish/a.arc");
        assert_eq!(active[1].files[0].external, "RMGE01.arc");
        assert_eq!(active[1].saves[0].external, "/save/RMG");
        disc.choose_defaults("RMGP01", &BTreeMap::new());
        assert_eq!(disc.sections[0].options[0].selected, 2);
    }

    #[test]
    fn a_target_refuses_another_game() {
        let disc = parse(SAMPLE).unwrap();
        assert!(disc.target.accepts("RMGE01"));
        assert!(!disc.target.accepts("RMGJ01"));
        assert!(!disc.target.accepts("SB4E01"));
        assert!(!disc.target.accepts("RMGE02"));
    }

    #[test]
    fn saved_choices_and_a_lone_switch() {
        let text = r#"<wiidisc version="1"><options><section name="S"><option name="On"><choice name="Enabled"><patch id="p"/></choice></option></section></options><patch id="p"/></wiidisc>"#;
        let mut lone = parse(text).unwrap();
        lone.choose_defaults("RMGE01", &BTreeMap::new());
        assert_eq!(lone.sections[0].options[0].selected, 1);
        let saved = parse_config(r#"<riivolution version="2"><option id="SOn" default="0"/></riivolution>"#);
        let mut off = parse(text).unwrap();
        off.sections[0].options[0].selected = 1;
        off.choose_defaults("RMGE01", &saved);
        assert_eq!(off.sections[0].options[0].selected, 0);
    }

    #[test]
    fn numbers_regions_and_encodings() {
        assert_eq!(number("0x10"), Some(16));
        assert_eq!(number(" 12 "), Some(12));
        assert_eq!(hex_bytes("0x38600001"), [0x38, 0x60, 0, 1]);
        assert!(hex_bytes("386").is_empty());
        assert_eq!(choice_region("NTSC-J (Japan)"), Some('J'));
        assert_eq!(choice_region("USA"), Some('E'));
        assert_eq!(choice_region("PAL"), Some('P'));
        assert_eq!(choice_region("Korea"), Some('K'));
        assert_eq!(choice_region("Enabled"), None);
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(decode(&[0xFF, 0xFE, b'a', 0]), "a");
        assert_eq!(decode(&[0xE9]), "\u{e9}");
    }

    #[test]
    fn a_default_past_the_choices_is_off() {
        let text = r#"<wiidisc version="1"><options><section name="S"><option name="A" default="7"><choice name="x"><patch id="p"/></choice></option></section></options><patch id="p"/></wiidisc>"#;
        assert_eq!(parse(text).unwrap().sections[0].options[0].selected, 0);
    }

    #[test]
    fn what_is_not_a_patch_is_said_so() {
        assert!(parse("<other/>").is_err());
        assert!(parse("<wiidisc version=\"2\"/>").is_err());
        assert!(parse("not xml").is_err());
    }
}
