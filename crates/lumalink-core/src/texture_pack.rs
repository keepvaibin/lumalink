//! Texture packs: Dolphin's (`tex1_<width>x<height>[_m]_<hash>[_<palette hash>|_$]_<format>[_arb][_mip<n>].(png|dds)` files
//! in a game-id folder, loose, or in a resource pack with a `manifest.json`) and Nebula's own (files named for the game's
//! textures, `<model>/<texture>.png`). A `.luma` carries Nebula's names: Dolphin names are converted against the player's
//! imported game (its names table, NebulaData/TextureNames.tsv), the same way Nebula converts a pack it is given
//! (runtime dolphin_pack.cpp).

use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PackInfo {
    pub folder: PathBuf,
    pub textures: usize,
    pub mips: usize,
    pub wildcards: usize,
    pub png: usize,
    pub dds: usize,
    /// Texture files whose names this game's packs do not use.
    pub other_games: BTreeMap<String, usize>,
    pub title: String,
    /// The pack's authors, one a name (a Dolphin resource pack's manifest.json lists them).
    pub authors: Vec<String>,
    pub description: String,
    pub version: String,
    /// The manifest's `website`, when it is a web address.
    pub homepage: String,
    /// The PNG a Dolphin resource pack shows for itself: `logo.png` beside its manifest.json.
    pub logo: Option<PathBuf>,
}

/// The first part of sixteen hexadecimal digits of a Dolphin name: the source image's hash.
pub fn hash_of_name(stem: &str) -> Option<u64> {
    let rest = stem.to_ascii_lowercase();
    let rest = rest.strip_prefix("tex1_")?;
    rest.split('_')
        .find(|part| part.len() == 16 && part.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|part| u64::from_str_radix(part, 16).ok())
}

/// A Dolphin texture name's parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DolphinName {
    pub width: u32,
    pub height: u32,
    /// The source texture's format: 0-6, 8 (C4), 9 (C8), 10 (C14X2), 14 (CMPR).
    pub format: u8,
    /// `_mip<n>`: a level of its own (1-31), 0 for the base.
    pub level: u8,
    /// `_m`
    pub mipmapped: bool,
    /// `_$`: any palette.
    pub any_palette: bool,
    /// `_arb`: levels whose contents differ from the base's.
    pub arbitrary: bool,
    pub hash: u64,
    pub palette: u64,
}

pub fn palettized(format: u8) -> bool {
    matches!(format, 8..=10)
}

fn decimal(text: &str, limit: u32) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|v| *v <= limit)
}

fn hex16(text: &str) -> Option<u64> {
    (text.len() == 16 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u64::from_str_radix(text, 16).ok()).flatten()
}

/// A file stem (no folder, no extension) as a Dolphin name; None when it is not one. A letter typed after the format
/// number ("_14v", a slip in some packs) reads as the number.
pub fn parse_dolphin_name(stem: &str) -> Option<DolphinName> {
    if !stem.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("tex1_")) {
        return None;
    }
    let parts: Vec<&str> = stem[5..].split('_').collect();
    if parts.len() > 8 {
        return None;
    }
    let mut name = DolphinName::default();
    let mut at = 0;
    let (w, h) = parts[at].split_once('x')?;
    at += 1;
    name.width = decimal(w, u32::MAX).filter(|v| *v != 0)?;
    name.height = decimal(h, u32::MAX).filter(|v| *v != 0)?;
    if parts.get(at) == Some(&"m") {
        name.mipmapped = true;
        at += 1;
    }
    name.hash = hex16(parts.get(at)?)?;
    at += 1;
    if parts.get(at) == Some(&"$") {
        name.any_palette = true;
        at += 1;
    } else if parts.get(at).is_some_and(|p| p.len() == 16) {
        name.palette = hex16(parts[at])?;
        at += 1;
    }
    let token = parts.get(at)?;
    at += 1;
    let format = match decimal(token, 255) {
        Some(f) => f,
        None => {
            let digits = token.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || digits == token.len() || !token[digits..].bytes().all(|b| b.is_ascii_alphabetic()) {
                return None;
            }
            decimal(&token[..digits], 255)?
        }
    };
    if !(format <= 6 || matches!(format, 8 | 9 | 10 | 14)) {
        return None;
    }
    name.format = format as u8;
    if !palettized(name.format) && (name.any_palette || name.palette != 0) {
        return None;
    }
    if parts.get(at) == Some(&"arb") {
        name.arbitrary = true;
        at += 1;
    }
    if let Some(tail) = parts.get(at) {
        let level = tail.strip_prefix("mip").filter(|l| !l.is_empty()).and_then(|l| decimal(l, 31)).filter(|l| *l != 0)?;
        name.level = level as u8;
        at += 1;
    }
    (at == parts.len()).then_some(name)
}

/// A row of the game's texture names (NebulaData/TextureNames.tsv in the player's imported game).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextureRow {
    /// `<model>/<texture>` or `<file>`: what a pack names it by.
    pub name: String,
    /// XXH64 of the source image's base level (a Dolphin name's hash).
    pub image: u64,
    /// Of the palette entries the source used, 0 without a palette.
    pub palette: u64,
    pub width: u32,
    pub height: u32,
    pub format: u8,
}

/// The rows of a names table; a line that does not read is skipped.
pub fn read_names(text: &str) -> Vec<TextureRow> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut cells = line.split('\t');
            let name = cells.next()?.to_string();
            let image = u64::from_str_radix(cells.next()?, 16).ok()?;
            let palette = u64::from_str_radix(cells.next()?, 16).ok()?;
            let width = cells.next()?.parse().ok()?;
            let height = cells.next()?.parse().ok()?;
            let format = cells.next()?.parse().ok()?;
            (!name.is_empty()).then_some(TextureRow { name, image, palette, width, height, format })
        })
        .collect()
}

/// The names table of the player's imported game; empty for a game imported before Nebula named its textures.
pub fn store_names(store: &Path) -> Vec<TextureRow> {
    nebula_import::cook::store::read_file(store, nebula_import::cook::texture::NAMES_PATH)
        .map(|bytes| read_names(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

/// Whether a game texture is the one a Dolphin name stands for.
pub fn matches(name: &DolphinName, row: &TextureRow) -> bool {
    row.image == name.hash
        && row.width == name.width
        && row.height == name.height
        && row.format == name.format
        && (!palettized(name.format) || name.any_palette || row.palette == name.palette)
}

/// The file a converted texture is, as Nebula converts it: `<name>[_arb][_mip<n>]<extension>`.
pub fn converted_file(row: &TextureRow, arbitrary: bool, level: u8, extension: &str) -> String {
    let mut out = row.name.clone();
    if arbitrary {
        out.push_str("_arb");
    }
    if level != 0 {
        out.push_str(&format!("_mip{level}"));
    }
    out.push_str(extension);
    out
}

/// What converting a pack into a `.luma` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversion {
    /// Files written in all.
    pub written: usize,
    /// Dolphin files that stood for a game texture.
    pub dolphin: usize,
    /// Files written under the game's names from them (one file may stand for several textures).
    pub textures: usize,
    /// Names an earlier file had given.
    pub taken: usize,
    /// Dolphin files kept under their own names: none of the player's textures (another release or edit of the game), or
    /// no names table to convert by. Nebula converts them on each player's game when the mod is installed.
    pub kept: Vec<String>,
    /// Whether the player's game gave names to convert by.
    pub named: bool,
}

/// Copies a pack's files that `wanted` takes into `out`: Dolphin names converted to the game's (`names`), the pack's other
/// files as they are. A Dolphin file standing for no texture of `names` is kept as it is, so no file is lost: Nebula tries
/// it again on the player's own game.
pub fn convert_into(root: &Path, names: &[TextureRow], out: &Path, wanted: impl Fn(&Path) -> bool) -> Result<Conversion> {
    struct Source {
        path: PathBuf,
        relative: String,
        extension: String,
        name: Option<DolphinName>,
    }
    let mut conversion = Conversion { named: !names.is_empty(), ..Default::default() };
    let mut sources = Vec::new();
    let mut given = BTreeSet::new();
    let walk = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| !(e.file_type().is_dir() && e.file_name() == ".dolphin"));
    for entry in walk {
        let entry = entry?;
        if !entry.file_type().is_file() || !wanted(entry.path()) {
            continue;
        }
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path()).to_string_lossy().replace('\\', "/");
        let extension = entry.path().extension().map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase())).unwrap_or_default();
        let stem = entry.path().file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if (extension == ".png" || extension == ".dds") && stem.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("tex1_")) {
            sources.push(Source { path: entry.path().to_path_buf(), relative, extension, name: parse_dolphin_name(&stem) });
            continue;
        }
        // A file of the pack's own naming: as it is.
        let target = out.join(&relative);
        fs::create_dir_all(target.parent().unwrap_or(out))?;
        fs::copy(entry.path(), &target)?;
        given.insert(relative.to_ascii_lowercase());
        conversion.written += 1;
    }
    // A palette named before "any palette", a DDS before a PNG, then by path: the first file to give a name keeps it.
    sources.sort_by(|a, b| {
        let any = |s: &Source| s.name.is_some_and(|n| n.any_palette);
        any(a).cmp(&any(b)).then_with(|| (b.extension == ".dds").cmp(&(a.extension == ".dds"))).then_with(|| a.relative.cmp(&b.relative))
    });
    let mut by_image: Vec<&TextureRow> = names.iter().collect();
    by_image.sort_by_key(|r| r.image);
    for source in &sources {
        let mut matched = false;
        if let Some(name) = &source.name {
            let first = by_image.partition_point(|r| r.image < name.hash);
            for row in by_image[first..].iter().take_while(|r| r.image == name.hash) {
                if !matches(name, row) {
                    continue;
                }
                matched = true;
                let file = converted_file(row, name.arbitrary, name.level, &source.extension);
                if !given.insert(file.to_ascii_lowercase()) {
                    conversion.taken += 1;
                    continue;
                }
                let target = out.join(&file);
                fs::create_dir_all(target.parent().unwrap_or(out))?;
                fs::copy(&source.path, &target)?;
                conversion.textures += 1;
                conversion.written += 1;
            }
        }
        if matched {
            conversion.dolphin += 1;
        } else {
            let target = out.join(&source.relative);
            fs::create_dir_all(target.parent().unwrap_or(out))?;
            fs::copy(&source.path, &target)?;
            conversion.kept.push(source.relative.clone());
            conversion.written += 1;
        }
    }
    Ok(conversion)
}

/// A game id a folder's name means: RMGE01, RMG, or another game's.
fn game_folder(name: &str) -> Option<&str> {
    ((name.len() == 3 || name.len() == 6) && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) && name.bytes().next().is_some_and(|b| b.is_ascii_uppercase())).then_some(name)
}

fn manifest_list(text: &str, field: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let names = match value.get(field) {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect(),
        _ => Vec::new(),
    };
    names.into_iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()
}

/// A PNG by its first bytes, whatever its name says.
fn is_png(path: &Path) -> bool {
    let mut head = [0u8; 8];
    std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok() && head == *b"\x89PNG\r\n\x1a\n"
}

fn manifest_field(text: &str, field: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return String::new();
    };
    match value.get(field) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

/// What a folder holds of a texture pack, if anything.
pub fn scan(root: &Path) -> Option<PackInfo> {
    let mut info = PackInfo { folder: root.to_path_buf(), ..Default::default() };
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "manifest.json" {
            let text = crate::riivolution::decode(&std::fs::read(entry.path()).unwrap_or_default());
            info.title = manifest_field(&text, "name");
            info.authors = manifest_list(&text, "authors");
            info.description = manifest_field(&text, "description");
            info.version = manifest_field(&text, "version");
            let website = manifest_field(&text, "website");
            if website.starts_with("https://") || website.starts_with("http://") {
                info.homepage = website;
            }
            if let Some(folder) = entry.path().parent() {
                info.logo = std::fs::read_dir(folder)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("logo.png")) && is_png(p));
            }
            continue;
        }
        if !name.starts_with("tex1_") || !(name.ends_with(".png") || name.ends_with(".dds")) {
            continue;
        }
        let stem = &name[..name.len() - 4];
        let of_game = entry
            .path()
            .ancestors()
            .skip(1)
            .take_while(|a| *a != root)
            .filter_map(|a| a.file_name()?.to_str())
            .find_map(game_folder)
            .map(str::to_string);
        if let Some(game) = of_game {
            if !game.starts_with("RMG") {
                *info.other_games.entry(game).or_default() += 1;
                continue;
            }
        }
        if name.ends_with(".png") {
            info.png += 1;
        } else {
            info.dds += 1;
        }
        if stem.contains("_mip") {
            info.mips += 1;
            continue;
        }
        if stem.contains('$') {
            info.wildcards += 1;
        }
        info.textures += 1;
    }
    // Anything at all under another game's folder says who the pack is for, whatever its files are named.
    for entry in walkdir::WalkDir::new(root).max_depth(3).into_iter().filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().is_dir() && crate::layout::is_other_game_folder(&name) {
            let files = walkdir::WalkDir::new(entry.path())
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png") || x.eq_ignore_ascii_case("dds"))
                })
                .count();
            if files != 0 {
                info.other_games.insert(name, files);
            }
        }
    }
    (info.textures > 0 || !info.other_games.is_empty()).then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_name_gives_its_hash() {
        assert_eq!(hash_of_name("tex1_64x64_m_0123456789abcdef_14"), Some(0x0123456789abcdef));
        assert_eq!(hash_of_name("TEX1_32x32_FEDCBA9876543210_ABCDEF0123456789_9"), Some(0xfedcba9876543210));
        assert_eq!(hash_of_name("tex1_64x64_14"), None);
        assert_eq!(hash_of_name("other"), None);
    }

    #[test]
    fn a_pack_is_counted_and_another_games_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let ours = dir.path().join("Load/Textures/RMGE01");
        let theirs = dir.path().join("Load/Textures/SB4E01");
        fs::create_dir_all(&ours).unwrap();
        fs::create_dir_all(&theirs).unwrap();
        fs::write(ours.join("tex1_8x8_0123456789abcdef_14.png"), b"").unwrap();
        fs::write(ours.join("tex1_8x8_0123456789abcdee_14_mip1.png"), b"").unwrap();
        fs::write(ours.join("tex1_8x8_$_14.dds"), b"").unwrap();
        fs::write(theirs.join("tex1_8x8_0123456789abcdef_14.png"), b"").unwrap();
        fs::write(dir.path().join("manifest.json"), br#"{"name":"HD","authors":["a","b"],"version":"1.0","website":"https://example.com/hd"}"#).unwrap();
        fs::write(dir.path().join("Logo.png"), b"\x89PNG\r\n\x1a\n....").unwrap();
        let info = scan(dir.path()).unwrap();
        assert_eq!(info.textures, 2);
        assert_eq!(info.mips, 1);
        assert_eq!(info.wildcards, 1);
        assert_eq!(info.png, 2);
        assert_eq!(info.dds, 1);
        assert_eq!(info.other_games["SB4E01"], 1);
        assert_eq!(info.title, "HD");
        assert_eq!(info.authors, ["a", "b"]);
        assert_eq!(info.homepage, "https://example.com/hd");
        assert_eq!(info.logo.as_deref().and_then(Path::file_name).unwrap(), "Logo.png");
        fs::write(dir.path().join("Logo.png"), b"not a png").unwrap();
        fs::write(dir.path().join("manifest.json"), br#"{"name":"HD","authors":"one","website":"example.com"}"#).unwrap();
        let info = scan(dir.path()).unwrap();
        assert_eq!(info.authors, ["one"]);
        assert!(info.homepage.is_empty() && info.logo.is_none(), "a website that is no address, a logo that is no PNG");
    }

    #[test]
    fn dolphin_names_read_as_dolphin_writes_them() {
        let name = parse_dolphin_name("tex1_64x32_m_0123456789abcdef_fedcba9876543210_9_arb_mip2").unwrap();
        assert_eq!(
            name,
            DolphinName { width: 64, height: 32, format: 9, level: 2, mipmapped: true, any_palette: false, arbitrary: true, hash: 0x0123456789abcdef, palette: 0xfedcba9876543210 }
        );
        assert!(parse_dolphin_name("TEX1_8x8_0123456789ABCDEF_$_8").unwrap().any_palette);
        assert_eq!(parse_dolphin_name("tex1_8x8_0123456789abcdef_14v").unwrap().format, 14, "a letter after the format");
        for bad in [
            "tex1_8x8_0123456789abcdef_7",       // no format 7
            "tex1_8x8_0123456789abcdef_$_14",    // a palette on CMPR
            "tex1_0x8_0123456789abcdef_14",      // no size
            "tex1_8x8_0123456789abcde_14",       // 15 digits
            "tex1_8x8_0123456789abcdef_14_mip0", // level 0 is the base
            "tex1_8x8_0123456789abcdef_14_mip32",
            "tex1_8x8_0123456789abcdef_14_x",
            "tex1_8x8_0123456789abcdef",
            "other",
        ] {
            assert_eq!(parse_dolphin_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_names_table_reads_and_a_store_without_one_gives_none() {
        let rows = read_names("name\timage\tpalette\twidth\theight\tformat\tfile\nKuribo/Body\t0123456789abcdef\t0000000000000000\t32\t16\t14\tObjectData/Kuribo.arc\nbad line\n");
        assert_eq!(rows, vec![TextureRow { name: "Kuribo/Body".into(), image: 0x0123456789abcdef, palette: 0, width: 32, height: 16, format: 14 }]);
        let dir = tempfile::tempdir().unwrap();
        assert!(store_names(&dir.path().join("missing.pak")).is_empty());
    }

    #[test]
    fn a_dolphin_pack_converts_to_the_games_names_and_keeps_what_matches_nothing() {
        let row = |name: &str, image: u64, palette: u64, format: u8| TextureRow { name: name.into(), image, palette, width: 8, height: 8, format };
        let names = vec![
            row("Kuribo/KuriboBody", 0xa, 0, 14),
            row("KuriboMini/KuriboBody", 0xa, 0, 14),
            row("IconA", 0xb, 0x1, 9),
            row("IconB", 0xb, 0x2, 9),
        ];
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        let game = pack.join("Load/Textures/RMGE01");
        fs::create_dir_all(game.join("sub")).unwrap();
        fs::create_dir_all(pack.join(".dolphin")).unwrap();
        fs::write(game.join("tex1_8x8_000000000000000a_14.png"), b"body png").unwrap();
        fs::write(game.join("tex1_8x8_000000000000000a_14.dds"), b"body dds").unwrap();
        fs::write(game.join("tex1_8x8_000000000000000a_14_mip1.png"), b"body1").unwrap();
        fs::write(game.join("sub/tex1_8x8_000000000000000b_$_9.png"), b"icon any").unwrap();
        fs::write(game.join("tex1_8x8_000000000000000b_0000000000000001_9.png"), b"icon a").unwrap();
        fs::write(game.join("tex1_8x8_00000000000000ff_14.png"), b"none").unwrap();
        fs::write(game.join("Coin.png"), b"native").unwrap();
        fs::write(pack.join(".dolphin/tex1_8x8_000000000000000a_14.png"), b"old").unwrap();
        let out = dir.path().join("out");
        let done = convert_into(&pack, &names, &out, |_| true).unwrap();
        let read = |p: &str| fs::read_to_string(out.join(p)).unwrap();
        assert_eq!(read("Kuribo/KuriboBody.dds"), "body dds", "a DDS first");
        assert_eq!(read("KuriboMini/KuriboBody.dds"), "body dds", "one file, every texture it stands for");
        assert_eq!(read("Kuribo/KuriboBody.png"), "body png");
        assert_eq!(read("Kuribo/KuriboBody_mip1.png"), "body1");
        assert_eq!(read("IconA.png"), "icon a", "a named palette before any palette");
        assert_eq!(read("IconB.png"), "icon any");
        assert_eq!(read("Load/Textures/RMGE01/Coin.png"), "native", "the pack's own names as they are");
        assert_eq!(read("Load/Textures/RMGE01/tex1_8x8_00000000000000ff_14.png"), "none", "kept for the player's own game");
        assert!(!out.join(".dolphin").exists(), "a converted pack's originals are not read");
        assert_eq!(done.kept, vec!["Load/Textures/RMGE01/tex1_8x8_00000000000000ff_14.png".to_string()]);
        assert_eq!((done.dolphin, done.textures, done.taken, done.written), (5, 8, 1, 10), "IconA from $ is taken");
        assert!(done.named);

        // Without names every Dolphin file is kept as it is.
        let out = dir.path().join("raw");
        let done = convert_into(&pack, &[], &out, |_| true).unwrap();
        assert!(!done.named && done.kept.len() == 6 && done.textures == 0 && done.written == 7);
    }

    #[test]
    fn a_folder_without_textures_is_no_pack() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), b"").unwrap();
        assert!(scan(dir.path()).is_none());
    }
}
