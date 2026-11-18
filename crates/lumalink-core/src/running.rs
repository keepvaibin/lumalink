//! The jobs LumaLink's window runs, each named in a file of its own while it runs and held locked. A job the system
//! ended with LumaLink (memory ran out, a fault no handler catches) leaves its file unlocked, and the next start finds
//! it and says which job on which mod stopped, where the window would otherwise have closed with no word.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The folder of the job files in LumaLink's data folder (see [`crate::update::own_data_files`]).
pub const RUNNING_FOLDER: &str = "running";

/// One job named while it runs; its file goes when this is dropped (the job ended, well or not).
pub struct Running {
    file: Option<(fs::File, PathBuf)>,
}

impl Running {
    /// Names job `job` (its words) on `target` in `<data>/running/<process>-<n>.txt`, locked while this lives. Nothing
    /// is named with no data folder, or when the file cannot be written: the job runs all the same.
    pub fn start(data: Option<&Path>, n: u64, job: &str, target: &Path) -> Running {
        let file = data.and_then(|data| {
            let folder = data.join(RUNNING_FOLDER);
            fs::create_dir_all(&folder).ok()?;
            let path = folder.join(format!("{}-{n}.txt", std::process::id()));
            let mut file = fs::OpenOptions::new().create(true).truncate(true).write(true).read(true).open(&path).ok()?;
            file.try_lock().ok()?;
            file.write_all(format!("{job}\n{}", target.display()).as_bytes()).ok()?;
            file.sync_all().ok()?;
            Some((file, path))
        });
        Running { file }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some((file, path)) = self.file.take() {
            drop(file);
            let _ = fs::remove_file(path);
        }
    }
}

/// The jobs a LumaLink that stopped left named (each one's words and target), their files removed. A job of a
/// LumaLink still running holds its file locked and is not one of them.
pub fn left_over(data: Option<&Path>) -> Vec<(String, PathBuf)> {
    let Some(data) = data else { return Vec::new() };
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(data.join(RUNNING_FOLDER)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| is_job_file(p)).collect();
    files.sort();
    for path in files {
        let Ok(mut file) = fs::OpenOptions::new().read(true).write(true).open(&path) else { continue };
        if file.try_lock().is_err() {
            continue;
        }
        // Read through the handle that holds the lock: on Windows the lock keeps every other handle out.
        let mut text = String::new();
        let _ = file.read_to_string(&mut text);
        drop(file);
        let _ = fs::remove_file(&path);
        if let Some((job, target)) = text.split_once('\n') {
            out.push((job.to_string(), PathBuf::from(target)));
        }
    }
    out
}

/// Whether `path` is a job file: `<digits>-<digits>.txt`.
pub fn is_job_file(path: &Path) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".txt")).and_then(|n| n.split_once('-')).is_some_and(|(p, n)| digits(p) && digits(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_ended_leaves_nothing_and_a_job_cut_short_is_found_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = Some(dir.path());
        {
            let _job = Running::start(data, 1, "Pack", Path::new("C:/mods/moon"));
            // Running: held, so not left over.
            assert!(left_over(data).is_empty());
        }
        assert_eq!(fs::read_dir(dir.path().join(RUNNING_FOLDER)).unwrap().count(), 0, "an ended job leaves no file");
        // A job whose process the system ended: its file stays, and no one holds it.
        let path = dir.path().join(RUNNING_FOLDER).join("4242-2.txt");
        fs::write(&path, "Convert\nC:/mods/sky").unwrap();
        fs::write(dir.path().join(RUNNING_FOLDER).join("notes.txt"), "not a job").unwrap();
        let _held = Running::start(data, 3, "Check", Path::new("C:/mods/held"));
        assert_eq!(left_over(data), [("Convert".to_string(), PathBuf::from("C:/mods/sky"))]);
        assert!(!path.exists(), "said once");
        assert!(left_over(data).is_empty());
        assert!(left_over(None).is_empty());
    }
}
