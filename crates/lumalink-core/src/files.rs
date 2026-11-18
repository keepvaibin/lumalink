//! Files LumaLink writes whole or not at all: each is written under a temporary name beside where it goes, flushed to
//! the disk, then moved over the old one. A run stopped midway leaves the old file whole; a write that fails leaves
//! nothing of the new one.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A file being written at a temporary path. Dropped before [`Part::finish`], it is removed: an error on the way
/// leaves nothing behind.
pub struct Part {
    path: Option<PathBuf>,
}

impl Part {
    /// Takes charge of `path`, where the caller writes the file (made by the caller, or by [`Part::create`]).
    pub fn at(path: PathBuf) -> Part {
        Part { path: Some(path) }
    }

    /// A new file at `path`, in charge of a [`Part`].
    pub fn create(path: PathBuf) -> io::Result<(Part, fs::File)> {
        let file = fs::File::create(&path)?;
        Ok((Part::at(path), file))
    }

    pub fn path(&self) -> &Path {
        self.path.as_deref().unwrap_or(Path::new(""))
    }

    /// Moves the written file over `to`. On an error the temporary file is removed.
    pub fn finish(mut self, to: &Path) -> io::Result<()> {
        let path = self.path.take().unwrap_or_default();
        fs::rename(&path, to).inspect_err(|_| {
            let _ = fs::remove_file(&path);
        })
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// The temporary path [`write_whole`] uses for `file`: `.<name>.<process>.part` beside it.
pub fn part_path(file: &Path) -> PathBuf {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let folder = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    folder.join(format!(".{name}.{}.part", std::process::id()))
}

/// Writes `bytes` as `file`, whole or not at all.
pub fn write_whole(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let (part, mut out) = Part::create(part_path(file))?;
    out.write_all(bytes)?;
    out.sync_all()?;
    drop(out);
    part.finish(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_dropped_before_finish_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m-1.0.0.luma.part");
        {
            let (part, mut file) = Part::create(path.clone()).unwrap();
            file.write_all(b"half").unwrap();
            assert_eq!(part.path(), path);
            // An error here: the caller returns with `?`.
        }
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_finished_part_replaces_the_file_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("luma.json");
        fs::write(&file, "old").unwrap();
        write_whole(&file, b"new").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_failed_move_leaves_the_old_file_and_no_part() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file goes: the move fails.
        let file = dir.path().join("luma.json");
        fs::create_dir(&file).unwrap();
        fs::write(file.join("inside"), "kept").unwrap();
        assert!(write_whole(&file, b"new").is_err());
        assert_eq!(fs::read_to_string(file.join("inside")).unwrap(), "kept");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no .part left");
    }
}
