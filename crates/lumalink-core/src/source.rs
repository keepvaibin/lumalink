//! A mod as published: a folder, or a `.zip`, `.7z` or `.rar` archive.

use crate::say;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// How much a mod may unpack to (a guard against a broken or hostile archive).
const MAX_UNPACKED: u64 = 64 * 1024 * 1024 * 1024;

/// Names no folder on Windows can hold, with or without an ending.
fn is_device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.as_bytes()[3].is_ascii_digit())
}

/// A path inside an archive as a safe relative path: nothing that leaves the destination, reaches a device or a file's
/// hidden stream, on any system the archive may be opened on.
pub(crate) fn safe_relative(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') {
        return None;
    }
    let path = Path::new(&name);
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(p) => {
                let text = p.to_string_lossy();
                if text.contains(':') || text.chars().any(char::is_control) || text.chars().all(|c| c == '.' || c == ' ') || is_device_name(&text) {
                    return None;
                }
                out.push(p);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Writes one entry, and no more of it than is left of what the archive may unpack to, whatever it says of itself.
fn write_limited(from: &mut dyn Read, target: &Path, left: &mut u64) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| say!("LumaLink.CannotMake", path = parent.display()))?;
    }
    let mut to = std::io::BufWriter::new(fs::File::create(target).with_context(|| say!("LumaLink.CannotMake", path = target.display()))?);
    let copied = std::io::copy(&mut from.take(*left + 1), &mut to).with_context(|| say!("LumaLink.Source.CannotUnpack", path = target.display()))?;
    to.flush().with_context(|| say!("LumaLink.CannotWrite", path = target.display()))?;
    if copied > *left {
        bail!(say!("LumaLink.Source.TooBig", size = MAX_UNPACKED >> 30));
    }
    *left -= copied;
    Ok(())
}

/// Unpacks a zip. Links in it are left out: they would be a way out of the folder, and no mod needs one.
pub(crate) fn unzip(archive: &Path, into: &Path) -> Result<()> {
    let file = fs::File::open(archive).with_context(|| say!("LumaLink.CannotRead", path = archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).with_context(|| say!("LumaLink.Source.NotReadable", path = archive.display(), kind = "zip"))?;
    let mut left = MAX_UNPACKED;
    for i in 0..zip.len() {
        // LZMA first: the library hands this method's four-byte zip header to the decoder as if it were LZMA data.
        let lzma = zip.by_index_raw(i)?.compression() == zip::CompressionMethod::Lzma;
        if lzma {
            let mut entry = zip.by_index_raw(i)?;
            let name = entry.name().to_string();
            let Some(relative) = safe_relative(&name) else {
                bail!(say!("LumaLink.Source.LeavesFolder", path = name));
            };
            let target = into.join(relative);
            let (size, crc) = (entry.size(), entry.crc32());
            if size > left {
                bail!(say!("LumaLink.Source.TooBig", size = MAX_UNPACKED >> 30));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_context(|| say!("LumaLink.CannotMake", path = parent.display()))?;
            }
            // Straight to the file, never whole in memory: an entry may state any size.
            let file = fs::File::create(&target).with_context(|| say!("LumaLink.CannotMake", path = target.display()))?;
            let mut to = Checked { to: std::io::BufWriter::new(file), crc: crc32fast::Hasher::new(), written: 0, size };
            unzip_lzma(&mut entry, size, &mut to).with_context(|| say!("LumaLink.Source.CannotUnpack", path = name))?;
            to.flush().with_context(|| say!("LumaLink.CannotWrite", path = target.display()))?;
            if to.written != size || to.crc.finalize() != crc {
                bail!(say!("LumaLink.Source.Damaged", path = name));
            }
            left -= size;
            continue;
        }
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        let Some(relative) = safe_relative(&name) else {
            bail!(say!("LumaLink.Source.LeavesFolder", path = name));
        };
        let target = into.join(relative);
        if entry.is_symlink() {
            continue;
        }
        if entry.is_dir() {
            fs::create_dir_all(&target).with_context(|| say!("LumaLink.CannotMake", path = target.display()))?;
            continue;
        }
        write_limited(&mut entry, &target, &mut left)?;
    }
    Ok(())
}

/// What an LZMA entry unpacks to on its way to the file: its CRC-32 taken, and nothing past the size the zip records.
struct Checked {
    to: std::io::BufWriter<fs::File>,
    crc: crc32fast::Hasher,
    written: u64,
    size: u64,
}

impl Write for Checked {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() as u64 > self.size - self.written {
            return Err(std::io::Error::other("more than the archive says the file holds"));
        }
        let n = self.to.write(buf)?;
        self.crc.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.to.flush()
    }
}

/// A zip entry's LZMA: version (2 bytes), size of the properties (2), the properties, then the stream. The stream may or
/// may not end in a marker, so the size comes from the zip's own record.
fn unzip_lzma(packed: &mut dyn Read, size: u64, to: &mut Checked) -> Result<()> {
    let mut head = [0u8; 4];
    packed.read_exact(&mut head).with_context(|| say!("LumaLink.Source.LzmaShort"))?;
    if u16::from_le_bytes([head[2], head[3]]) != 5 {
        bail!(say!("LumaLink.Source.NotLzma"));
    }
    let options = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(size)),
        memlimit: None,
        allow_incomplete: false,
    };
    let mut stream = std::io::BufReader::new(packed);
    lzma_rs::lzma_decompress_with_options(&mut stream, to, &options).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(())
}

fn un7z(archive: &Path, into: &Path) -> Result<()> {
    let mut left = MAX_UNPACKED;
    sevenz_rust2::decompress_file_with_extract_fn(archive, into, |entry, reader, _| {
        // The library joins the archive's names as they are: each goes through the same check as a zip's.
        let Some(relative) = safe_relative(entry.name()) else {
            return Err(sevenz_rust2::Error::other(say!("LumaLink.Source.LeavesFolder", path = entry.name())));
        };
        let target = into.join(relative);
        if entry.is_directory() {
            fs::create_dir_all(&target)?;
        } else {
            write_limited(reader, &target, &mut left).map_err(|e| sevenz_rust2::Error::other(format!("{e:#}")))?;
        }
        Ok(true)
    })
    .with_context(|| say!("LumaLink.Source.NotReadable", path = archive.display(), kind = "7z"))
}

/// The commands that can read a `.rar`, in the order to try: the system's own first, never a program of that name that
/// happens to come first on the path. Windows 10 and later and macOS have bsdtar as `tar`; Linux has it as `bsdtar`.
fn rar_readers() -> Vec<PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        vec![root.join("System32").join("tar.exe")]
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/usr/bin/tar"), PathBuf::from("/usr/bin/bsdtar")]
    } else {
        vec![PathBuf::from("/usr/bin/bsdtar"), PathBuf::from("/bin/bsdtar")]
    }
}

/// A `.rar` through the system's bsdtar, which keeps paths inside the folder and won't write through a link. What it
/// says it holds is checked first, and what it wrote after.
fn unrar(archive: &Path, into: &Path) -> Result<()> {
    let mut last = say!("LumaLink.Source.NoRarReader");
    for reader in rar_readers() {
        let Ok(listing) = std::process::Command::new(&reader).arg("-tvf").arg(archive).output() else { continue };
        if !listing.status.success() {
            last = String::from_utf8_lossy(&listing.stderr).trim().to_string();
            continue;
        }
        let stated: u64 = String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().nth(4).and_then(|size| size.parse::<u64>().ok()))
            .fold(0, u64::saturating_add);
        if stated > MAX_UNPACKED {
            bail!(say!("LumaLink.Source.TooBig", size = MAX_UNPACKED >> 30));
        }
        let output = std::process::Command::new(&reader).arg("-xf").arg(archive).arg("-C").arg(into).output().with_context(|| say!("LumaLink.Source.CannotStart", path = reader.display()))?;
        if !output.status.success() {
            bail!(say!("LumaLink.Source.CannotUnpackWhy", path = archive.display(), why = String::from_utf8_lossy(&output.stderr).trim()));
        }
        let root = fs::canonicalize(into)?;
        let mut total = 0u64;
        let mut links = Vec::new();
        for entry in walkdir::WalkDir::new(into) {
            let entry = entry?;
            if !fs::canonicalize(entry.path())?.starts_with(&root) {
                bail!(say!("LumaLink.Source.LeavesFolder", path = entry.path().display()));
            }
            if entry.path_is_symlink() {
                links.push(entry.path().to_path_buf());
            } else if entry.file_type().is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
        // Links are left out, as from a zip: no mod needs one, and one to a folder above it would never end.
        for link in links {
            fs::remove_file(&link).or_else(|_| fs::remove_dir(&link)).with_context(|| say!("LumaLink.Source.CannotRemove", path = link.display()))?;
        }
        if total > MAX_UNPACKED {
            bail!(say!("LumaLink.Source.TooBig", size = MAX_UNPACKED >> 30));
        }
        return Ok(());
    }
    bail!(say!("LumaLink.Source.CannotUnpackWhy", path = archive.display(), why = last))
}

/// A mod's name from its file or folder name.
pub fn mod_name(input: &Path) -> String {
    input
        .file_stem()
        .or_else(|| input.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "mod".to_string())
}

/// Whether a file name is an archive this tool unpacks.
pub fn is_archive(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("zip" | "7z" | "rar")
    )
}

/// A disc image of the game.
pub fn is_disc(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("iso" | "wbfs" | "rvz" | "wia" | "ciso" | "gcz" | "nfs")
    )
}

/// A folder holding a whole game, as an emulator extracts one.
fn is_extracted_disc(path: &Path) -> bool {
    path.join("sys").join("main.dol").is_file() && path.join("files").is_dir()
        || path.join("DATA").join("sys").join("main.dol").is_file() && path.join("DATA").join("files").is_dir()
}

/// The game as a disc or a folder: to compare a mod that's a whole changed game with.
static BASE_GAME: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// The imported game, whose record of the disc's files (DiscFiles.tsv) a whole changed game is read against.
static STORE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// The disc files Setup keeps beside the imported game, as the base when none was named.
pub fn base_from_store(store: &Path) {
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(store.to_path_buf());
    let inputs = store.parent().map(|p| p.join("game-inputs"));
    if let Some(inputs) = inputs.filter(|p| p.join("sys").join("main.dol").is_file()) {
        let mut base = BASE_GAME.lock().unwrap_or_else(|e| e.into_inner());
        if base.is_none() {
            *base = Some(inputs);
        }
    }
}

/// What `--base` names, when it's a whole game.
pub fn set_base_game(path: Option<PathBuf>) {
    *BASE_GAME.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

/// The player's game, file by file (lower-case path: size, XXH64): the record the import kept, or, for a game imported
/// before Nebula kept one, the game's own disc named with --base, read once.
pub(crate) fn original_files() -> Result<std::collections::HashMap<String, (u64, u64)>> {
    use nebula_import::cook;
    if let Some(store) = STORE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        if let Ok(text) = cook::store::read_file(&store, cook::DISC_FILES_PATH) {
            return Ok(cook::read_disc_files(&String::from_utf8_lossy(&text)));
        }
    }
    let Some(base) = BASE_GAME.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        bail!(say!("LumaLink.Source.WholeGame"));
    };
    let mut game = nebula_import::ChangedGame::open(&base).with_context(|| say!("LumaLink.Source.NotThisGame", path = base.display()))?;
    let mut files = std::collections::HashMap::new();
    game.visit_files(&mut |path, bytes| {
        files.insert(path.to_ascii_lowercase(), (bytes.len() as u64, cook::texture::xxh64(&bytes)));
        Ok(())
    })?;
    files.insert("sys/main.dol".to_string(), (game.dol.len() as u64, cook::texture::xxh64(&game.dol)));
    Ok(files)
}

/// A mod that's a whole changed game: only the files that differ from the player's, in the folders of the disc, with its
/// program beside them when that differs too. Files only a console reads are not taken.
fn changes_from_game(input: &Path, work: &Path) -> Result<PathBuf> {
    use nebula_import::cook;
    let mut changed = nebula_import::ChangedGame::open(input).with_context(|| format!("{}", input.display()))?;
    let original = original_files().with_context(|| format!("{}", input.display()))?;
    let into = work.join("changes");
    fs::create_dir_all(&into)?;
    let same = |path: &str, bytes: &[u8]| {
        original.get(&path.to_ascii_lowercase()).is_some_and(|(size, hash)| *size == bytes.len() as u64 && *hash == cook::texture::xxh64(bytes))
    };
    let mut count = 0usize;
    changed.visit_files(&mut |path, bytes| {
        if cook::console_only(path) || same(path, &bytes) {
            return Ok(());
        }
        let Some(relative) = safe_relative(path) else {
            return Ok(());
        };
        let target = into.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &bytes).with_context(|| say!("LumaLink.CannotWrite", path = target.display()))?;
        count += 1;
        Ok(())
    })?;
    if !same("sys/main.dol", &changed.dol) {
        fs::write(into.join("main.dol"), &changed.dol)?;
        // The program it's compared with: the game's own disc when named, else the one Setup keeps (plan::base_program).
        if let Some(base) = BASE_GAME.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            if let Ok(game) = nebula_import::ChangedGame::open(&base) {
                let kept = work.join("original.dol");
                fs::write(&kept, &game.dol)?;
                crate::plan::set_base_program(Some(kept));
            }
        }
        count += 1;
    }
    if count == 0 {
        bail!(say!("LumaLink.Source.SameAsGame", path = input.display()));
    }
    Ok(into)
}

/// Opens a mod: an archive unpacks under `work`, a folder is used in place and never written to.
pub fn open(input: &Path, work: &Path) -> Result<PathBuf> {
    if input.is_dir() {
        if is_extracted_disc(input) {
            return changes_from_game(input, work);
        }
        return Ok(input.to_path_buf());
    }
    if !input.is_file() {
        bail!(say!("LumaLink.Source.NotExist", path = input.display()));
    }
    if is_disc(input) {
        return changes_from_game(input, work);
    }
    if matches!(input.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("xdelta" | "vcdiff" | "bps" | "ips" | "ppf")) {
        bail!(say!("LumaLink.Source.DiscPatch", path = input.display()));
    }
    if !is_archive(input) {
        bail!(say!("LumaLink.Source.NotAMod", path = input.display()));
    }
    // The file's name, when it's a name: `...zip` is a real file, and its stem is the folder above.
    let into = work.join(safe_relative(&mod_name(input)).filter(|p| p.components().count() == 1).unwrap_or_else(|| PathBuf::from("mod")));
    if into.exists() {
        fs::remove_dir_all(&into).with_context(|| say!("LumaLink.Source.CannotRemove", path = into.display()))?;
    }
    fs::create_dir_all(&into).with_context(|| say!("LumaLink.CannotMake", path = into.display()))?;
    match input.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("zip") => unzip(input, &into)?,
        Some("7z") => un7z(input, &into)?,
        _ => unrar(input, &into)?,
    }
    Ok(into)
}

#[cfg(test)]
mod tests {
    /// A zip with one LZMA entry, written as 7-Zip writes one: zip header, properties, stream with an end marker. The
    /// zip's record says the entry unpacks to `stated` bytes.
    fn lzma_zip(text: &[u8], stated: u32) -> Vec<u8> {
        let mut lzma = Vec::new();
        lzma_rs::lzma_compress(&mut &text[..], &mut lzma).unwrap();
        // lzma_compress writes properties (5), size (8), stream.
        let mut data = vec![0x15, 0x07, 5, 0];
        data.extend_from_slice(&lzma[..5]);
        data.extend_from_slice(&lzma[13..]);
        let name = b"a/b.xml";
        let crc = crc32fast::hash(text);
        let mut record = Vec::new();
        record.extend_from_slice(&63u16.to_le_bytes());
        record.extend_from_slice(&2u16.to_le_bytes());
        record.extend_from_slice(&14u16.to_le_bytes());
        record.extend_from_slice(&[0, 0, 0x21, 0]);
        record.extend_from_slice(&crc.to_le_bytes());
        record.extend_from_slice(&(data.len() as u32).to_le_bytes());
        record.extend_from_slice(&stated.to_le_bytes());
        record.extend_from_slice(&(name.len() as u16).to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes());
        let mut zip = b"PK".to_vec();
        zip.extend_from_slice(&record);
        zip.extend_from_slice(name);
        zip.extend_from_slice(&data);
        let central = zip.len() as u32;
        zip.extend_from_slice(b"PK");
        zip.extend_from_slice(&63u16.to_le_bytes());
        zip.extend_from_slice(&record);
        zip.extend_from_slice(&[0u8; 10]);
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(name);
        let size = zip.len() as u32 - central;
        zip.extend_from_slice(b"PK");
        zip.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&central.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        zip
    }

    #[test]
    fn unpacks_lzma_entries() {
        let text = b"<wiidisc version=\"1\">the same words, the same words, the same words</wiidisc>".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.zip");
        std::fs::write(&path, lzma_zip(&text, text.len() as u32)).unwrap();
        let out = dir.path().join("out");
        super::unzip(&path, &out).unwrap();
        assert_eq!(std::fs::read(out.join("a/b.xml")).unwrap(), text);
    }

    /// An LZMA entry that unpacks to more or less than its record says is refused, not trusted.
    #[test]
    fn an_lzma_entry_is_held_to_its_size() {
        let text = b"<wiidisc version=\"1\">the same words, the same words, the same words</wiidisc>".to_vec();
        let dir = tempfile::tempdir().unwrap();
        for stated in [text.len() as u32 - 1, text.len() as u32 + 1, u32::MAX] {
            let path = dir.path().join(format!("{stated}.zip"));
            std::fs::write(&path, lzma_zip(&text, stated)).unwrap();
            assert!(super::unzip(&path, &dir.path().join(format!("out{stated}"))).is_err(), "{stated}");
        }
    }

    use super::*;

    #[test]
    fn a_path_cannot_leave_the_folder() {
        assert_eq!(safe_relative("a/b.arc"), Some(PathBuf::from("a/b.arc")));
        assert_eq!(safe_relative("a\\b.arc"), Some(PathBuf::from("a/b.arc")));
        assert_eq!(safe_relative("./a"), Some(PathBuf::from("a")));
        assert_eq!(safe_relative("../a"), None);
        assert_eq!(safe_relative("a/../../b"), None);
        assert_eq!(safe_relative("/a"), None);
        assert_eq!(safe_relative("C:\\a"), None);
        assert_eq!(safe_relative("\\a"), None);
        assert_eq!(safe_relative("a:stream"), None);
        assert_eq!(safe_relative("a/CON"), None);
        assert_eq!(safe_relative("aux.txt"), None);
        assert_eq!(safe_relative("com7.bin"), None);
        assert_eq!(safe_relative("a/.../b"), None);
        assert_eq!(safe_relative("a/b\u{1}c"), None);
        assert_eq!(safe_relative("console.arc"), Some(PathBuf::from("console.arc")));
    }

    #[test]
    fn a_zip_unpacks_and_a_bad_one_stops() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("My Mod.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("ObjectData/a.arc", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();
        let opened = open(&archive, dir.path()).unwrap();
        assert_eq!(fs::read(opened.join("ObjectData/a.arc")).unwrap(), b"hello");
        assert_eq!(mod_name(&archive), "My Mod");

        let bad = dir.path().join("bad.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&bad).unwrap());
        zip.start_file("../evil.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        assert!(open(&bad, dir.path()).is_err());
        assert!(!dir.path().join("evil.txt").exists());
    }

    #[test]
    fn a_file_named_after_the_folder_above_stays_inside() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let keep = dir.path().join("keep.txt");
        fs::write(&keep, b"x").unwrap();
        let archive = dir.path().join("...zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("a.bin", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();
        let opened = open(&archive, &work).unwrap();
        assert!(opened.starts_with(&work));
        assert!(keep.is_file());
    }

    #[test]
    fn what_an_entry_really_holds_is_what_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut left = 4u64;
        assert!(write_limited(&mut &b"12345"[..], &dir.path().join("big"), &mut left).is_err());
        let mut left = 5u64;
        write_limited(&mut &b"12345"[..], &dir.path().join("fits"), &mut left).unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn a_link_in_a_zip_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("link.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.add_symlink("out", "../..", zip::write::SimpleFileOptions::default()).unwrap();
        zip.start_file("in.bin", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let into = dir.path().join("into");
        unzip(&archive, &into).unwrap();
        assert!(into.join("in.bin").is_file());
        assert!(fs::symlink_metadata(into.join("out")).is_err());
    }

    #[test]
    fn a_folder_is_used_in_place_and_others_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(open(dir.path(), dir.path()).unwrap(), dir.path());
        let text = dir.path().join("a.txt");
        fs::write(&text, b"x").unwrap();
        assert!(open(&text, dir.path()).is_err());
        assert!(open(&dir.path().join("none"), dir.path()).is_err());
    }

    /// An extracted game folder: boot header with `id`, a program, and a file table of `files` (each in its folder).
    fn game_folder(root: &std::path::Path, id: &str, dol: &[u8], files: &[(&str, &str, &[u8])]) {
        use std::fs;
        let sys = root.join("sys");
        fs::create_dir_all(&sys).unwrap();
        let mut boot = vec![0u8; 0x440];
        boot[..6].copy_from_slice(id.as_bytes());
        fs::write(sys.join("boot.bin"), boot).unwrap();
        fs::write(sys.join("main.dol"), dol).unwrap();
        // The file table: the root, then each folder followed by its one file; names after the entries.
        let mut entries: Vec<[u32; 3]> = Vec::new();
        let mut names = Vec::new();
        let name = |text: &str, names: &mut Vec<u8>| {
            let at = names.len() as u32;
            names.extend_from_slice(text.as_bytes());
            names.push(0);
            at
        };
        let count = 1 + files.len() as u32 * 2;
        entries.push([0x0100_0000, 0, count]);
        for (i, (folder, file, bytes)) in files.iter().enumerate() {
            let folder_index = 1 + i as u32 * 2;
            entries.push([0x0100_0000 | name(folder, &mut names), 0, folder_index + 2]);
            entries.push([name(file, &mut names), 0x1000 * (i as u32 + 1), bytes.len() as u32]);
            let dir = root.join("files").join(folder);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(file), bytes).unwrap();
        }
        let mut fst = Vec::new();
        for entry in entries {
            for word in entry {
                fst.extend_from_slice(&word.to_be_bytes());
            }
        }
        fst.extend_from_slice(&names);
        fs::write(sys.join("fst.bin"), fst).unwrap();
    }

    #[test]
    fn a_whole_changed_game_gives_only_what_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (original, changed, work) = (dir.path().join("original"), dir.path().join("changed"), dir.path().join("work"));
        game_folder(&original, "RMGE01", b"program", &[("StageData", "A.arc", b"same"), ("ObjectData", "B.arc", b"old")]);
        game_folder(&changed, "RMGE01", b"program 2", &[("StageData", "A.arc", b"same"), ("ObjectData", "B.arc", b"new!")]);
        super::set_base_game(Some(original.clone()));
        let into = super::changes_from_game(&changed, &work).unwrap();
        super::set_base_game(None);
        crate::plan::set_base_program(None);
        assert!(!into.join("StageData/A.arc").exists());
        assert_eq!(std::fs::read(into.join("ObjectData/B.arc")).unwrap(), b"new!");
        assert_eq!(std::fs::read(into.join("main.dol")).unwrap(), b"program 2");
        let other = dir.path().join("other");
        game_folder(&other, "SB4E01", b"x", &[("StageData", "A.arc", b"x")]);
        let error = format!("{:#}", super::changes_from_game(&other, &work).unwrap_err());
        assert!(error.contains("Super Mario Galaxy 2"), "{error}");
    }
}
