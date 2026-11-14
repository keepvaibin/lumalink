//! The player's imported game (`game.pak`, Nebula's asset store): what paths exist, for a mod that names a file by name
//! alone, and which game it is. Only read; nothing of it goes into a package.

use crate::say;
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const NODE: usize = 40;

/// The paths in a store, from its index. The store names a disc's archive as a folder without its ending
/// (`ObjectData/Kuribo.arc` is `ObjectData/Kuribo`, store::store_path); every question here is asked in those names.
pub struct GameStore {
    pub path: PathBuf,
    pub game_id: String,
    /// Lower-case store paths without a leading '/': files and folders, the insides of archives too.
    paths: BTreeSet<String>,
    /// Outside archives, a lower-case store name's paths, each with whether it is an archive.
    by_name: BTreeMap<String, Vec<(String, bool)>>,
}

/// A disc or mod path as the store names it, lower-case, without a leading '/'.
fn key(path: &str) -> String {
    nebula_import::cook::store::store_path(path.trim_start_matches('/')).to_ascii_lowercase()
}

/// Whether a file name is an archive's on the disc (the store drops the ending).
fn is_archive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.len() > 4 && (lower.ends_with(".arc") || lower.ends_with(".szs"))
}

/// A store's header: the magic, then the version Nebula's importer writes (nebula_import::cook::store::VERSION, the
/// game's store_version.inc). A store of another version holds its files in another form, or lacks some: the game
/// refuses it, and so does LumaLink, by its version, rather than failing later on a file the old form does not have.
pub fn header(pak: &mut impl Read, path: &Path) -> Result<[u8; 64]> {
    use nebula_import::cook::store::{MAGIC, VERSION};
    let mut header = [0u8; 64];
    pak.read_exact(&mut header).with_context(|| say!("LumaLink.Store.TooShort", path = path.display()))?;
    if &header[..8] != MAGIC {
        bail!(say!("LumaLink.Store.Not", path = path.display()));
    }
    let found = u32::from_le_bytes(header[8..12].try_into().unwrap());
    if found != VERSION {
        bail!(say!("LumaLink.Store.Version", path = path.display(), found = found, wanted = VERSION));
    }
    Ok(header)
}

/// The sizes of a store's index and names, when the file can hold them: a damaged header may say any size, and the
/// room is only made for what is there.
fn parts_fit(pak: &File, count: usize, names_size: u64) -> Result<(usize, usize)> {
    let length = pak.metadata()?.len();
    let nodes_size = count.checked_mul(NODE).filter(|&size| size as u64 <= length).with_context(|| say!("LumaLink.Store.IndexLarge"))?;
    let names_size = usize::try_from(names_size).ok().filter(|&size| size as u64 <= length).with_context(|| say!("LumaLink.Store.NamesLarge"))?;
    Ok((nodes_size, names_size))
}

impl GameStore {
    /// Every file of a store with where its bytes are: (path as the store names it, position, size), in index order.
    pub fn files(path: &Path) -> Result<Vec<(String, u64, u64)>> {
        Self::walk(path, false)
    }

    /// As `files`, with each archive's folder named as on the disc (`ObjectData/Kinopio.arc/kinopio.bdl`): as a mod's
    /// layer names it.
    pub fn disc_files(path: &Path) -> Result<Vec<(String, u64, u64)>> {
        Self::walk(path, true)
    }

    fn walk(path: &Path, disc: bool) -> Result<Vec<(String, u64, u64)>> {
        let mut pak = File::open(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
        let header = header(&mut pak, path)?;
        let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
        let count = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let (nodes_at, names_at, names_size) = (word(16), word(32), word(40));
        let (nodes_size, names_size) = parts_fit(&pak, count, names_size)?;
        let mut nodes = vec![0u8; nodes_size];
        pak.seek(SeekFrom::Start(nodes_at))?;
        pak.read_exact(&mut nodes).with_context(|| say!("LumaLink.Store.IndexCut"))?;
        let mut names = vec![0u8; names_size];
        pak.seek(SeekFrom::Start(names_at))?;
        pak.read_exact(&mut names).with_context(|| say!("LumaLink.Store.NamesCut"))?;
        let mut full: Vec<String> = Vec::with_capacity(count);
        let mut files = Vec::new();
        for i in 0..count {
            let node = &nodes[i * NODE..(i + 1) * NODE];
            let position = u64::from_le_bytes(node[0..8].try_into().unwrap());
            let size = u64::from_le_bytes(node[8..16].try_into().unwrap());
            let name_at = u32::from_le_bytes(node[16..20].try_into().unwrap()) as usize;
            let parent = u32::from_le_bytes(node[20..24].try_into().unwrap()) as usize;
            let flags = u32::from_le_bytes(node[24..28].try_into().unwrap());
            let folder = flags & 1 != 0;
            let tail = names.get(name_at..).unwrap_or(&[]);
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            let mut name = String::from_utf8_lossy(&tail[..end]).to_string();
            if disc && flags & 2 != 0 {
                name.push_str(".arc");
            }
            let path = match full.get(parent).map(String::as_str) {
                _ if i == 0 => String::new(),
                Some("") | None => name,
                Some(parent) => format!("{parent}/{name}"),
            };
            if i != 0 && !folder {
                files.push((path.clone(), position, size));
            }
            full.push(path);
        }
        Ok(files)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let mut pak = File::open(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
        let header = header(&mut pak, path)?;
        let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
        let count = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let (nodes_at, names_at, names_size) = (word(16), word(32), word(40));
        let game_id = String::from_utf8_lossy(&header[48..54]).to_string();

        let (nodes_size, names_size) = parts_fit(&pak, count, names_size)?;
        let mut nodes = vec![0u8; nodes_size];
        pak.seek(SeekFrom::Start(nodes_at))?;
        pak.read_exact(&mut nodes).with_context(|| say!("LumaLink.Store.IndexCut"))?;
        let mut names = vec![0u8; names_size];
        pak.seek(SeekFrom::Start(names_at))?;
        pak.read_exact(&mut names).with_context(|| say!("LumaLink.Store.NamesCut"))?;

        let mut full: Vec<String> = Vec::with_capacity(count);
        // Whether a node is an archive, or inside one.
        let mut in_archive: Vec<bool> = Vec::with_capacity(count);
        let mut paths = BTreeSet::new();
        let mut by_name: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
        for i in 0..count {
            let node = &nodes[i * NODE..(i + 1) * NODE];
            let name_at = u32::from_le_bytes(node[16..20].try_into().unwrap()) as usize;
            let parent = u32::from_le_bytes(node[20..24].try_into().unwrap()) as usize;
            let archive = u32::from_le_bytes(node[24..28].try_into().unwrap()) & 2 != 0;
            let tail = names.get(name_at..).unwrap_or(&[]);
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            let name = String::from_utf8_lossy(&tail[..end]).to_string();
            if i == 0 {
                full.push(String::new());
                in_archive.push(false);
                continue;
            }
            let path = match full.get(parent).map(String::as_str) {
                Some("") | None => name.clone(),
                Some(parent) => format!("{parent}/{name}"),
            };
            let inside = in_archive.get(parent).copied().unwrap_or(false);
            paths.insert(path.to_ascii_lowercase());
            if !inside {
                by_name.entry(name.to_ascii_lowercase()).or_default().push((path.clone(), archive));
            }
            full.push(path);
            in_archive.push(inside || archive);
        }
        Ok(Self { path: path.to_path_buf(), game_id, paths, by_name })
    }

    /// A store for the tests, from disc paths (`ObjectData/Kuribo.arc`), named as an imported store names them.
    pub fn from_paths(game_id: &str, list: &[&str]) -> Self {
        let mut by_name: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
        for path in list {
            let parts: Vec<&str> = path.split('/').collect();
            // Inside an archive: some part before the last is one.
            if parts[..parts.len() - 1].iter().any(|p| is_archive_name(p)) {
                continue;
            }
            let store = nebula_import::cook::store::store_path(path);
            let name = store.rsplit('/').next().unwrap_or(&store).to_ascii_lowercase();
            by_name.entry(name).or_default().push((store.clone(), is_archive_name(parts[parts.len() - 1])));
        }
        Self { path: PathBuf::new(), game_id: game_id.to_string(), paths: list.iter().map(|p| key(p)).collect(), by_name }
    }

    /// Whether the game has this disc or store path.
    pub fn has(&self, path: &str) -> bool {
        self.paths.contains(&key(path))
    }

    /// A top-level folder of the game, in any case.
    pub fn has_folder(&self, name: &str) -> bool {
        self.paths.contains(&name.to_ascii_lowercase())
    }

    /// Where a file of this disc name is, outside archives, as disc paths (an archive gets back the ending it was asked
    /// with: `IceMario.arc` is `ObjectData/IceMario.arc`).
    pub fn named(&self, name: &str) -> Vec<String> {
        let archive = is_archive_name(name);
        let stem = if archive { &name[..name.len() - 4] } else { name };
        self.by_name
            .get(&stem.to_ascii_lowercase())
            .map(|paths| {
                paths
                    .iter()
                    .filter(|(_, is_archive)| *is_archive == archive)
                    .map(|(path, _)| if archive { format!("{path}{}", &name[name.len() - 4..]) } else { path.clone() })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store whose header says more than the file holds is refused, never made room for.
    #[test]
    fn a_damaged_store_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for (count, names) in [(u32::MAX, 0u64), (1, u64::MAX), (1, 1 << 40)] {
            let mut header = [0u8; 64];
            header[..8].copy_from_slice(b"NBSTORE\0");
            header[8..12].copy_from_slice(&nebula_import::cook::store::VERSION.to_le_bytes());
            header[12..16].copy_from_slice(&count.to_le_bytes());
            header[16..24].copy_from_slice(&64u64.to_le_bytes());
            header[32..40].copy_from_slice(&64u64.to_le_bytes());
            header[40..48].copy_from_slice(&names.to_le_bytes());
            let path = dir.path().join("store.pak");
            std::fs::write(&path, header).unwrap();
            assert!(GameStore::open(&path).is_err());
            assert!(GameStore::files(&path).is_err());
        }
    }

    #[test]
    fn a_store_answers_by_path_and_name_in_any_case() {
        let store = GameStore::from_paths("RMGE01", &["ObjectData", "ObjectData/Kuribo.arc", "ObjectData/Kuribo.arc/x.bdl"]);
        assert!(store.has("/objectdata/KURIBO.arc"));
        assert!(!store.has("/StageData/a.arc"));
        assert!(store.has_folder("objectdata"));
        assert_eq!(store.named("kuribo.ARC"), ["ObjectData/Kuribo.ARC"]);
        assert!(store.named("x.bdl").is_empty());
        assert!(store.named("Kuribo").is_empty());
        // As an imported store names them: the archive is a folder without its ending.
        assert!(store.has("ObjectData/Kuribo") && store.has("objectdata/kuribo/X.BDL"));
    }

    #[test]
    fn a_file_that_is_no_store_is_said_so() {
        let dir = std::env::temp_dir().join(format!("lumalink_store_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not.pak");
        std::fs::write(&file, vec![0u8; 100]).unwrap();
        assert!(GameStore::open(&file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
