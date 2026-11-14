//! `.luma` files (docs/modding/format/luma.md): written from a mod folder, listed and opened, every one checked by the
//! game's own validator (`luma-validate`) before it is written and again as written.
//!
//! The zip is written here, not by a zip library, so that the same folder gives the same bytes on every host
//! (docs/modding/language/compiled-form.md, section 1.2): entries in one order (`luma.json`, then every other file by its name's
//! bytes), no folder entries, every time 1980-01-01 00:00, no attributes and no extra field but Zip64's, the
//! "made by" field fixed, deflate by the one `miniz_oxide` version Cargo.lock names.

use crate::say;
use anyhow::{bail, Context, Result};
use luma_validate::zip::{self as lzip, Bytes, Entry, EntryKind, FileBytes, Unpacked};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Endings whose data is already compressed: stored, since deflate would only cost time.
const STORED_ENDINGS: &[&str] = &[".png", ".jpg", ".jpeg", ".webp", ".ogg", ".opus", ".webm", ".mp4", ".zip", ".luma", ".ktx2"];
/// Files above this are stored and streamed, never held in memory whole.
const STREAM_ABOVE: u64 = 64 << 20;
/// docs/modding/format/luma.md, section 2: a file at most 4 GiB unpacked.
const MAX_FILE: u64 = 4 << 30;
const DEFLATE_LEVEL: u8 = 9;

/// One file a folder gives a `.luma`: its name in the `.luma` and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderFile {
    pub name: String,
    pub path: PathBuf,
}

/// Every file under `folder`, named as it will be in the `.luma` (`/` separators), in the order written: `luma.json`
/// first, then by the name's bytes. A link, or a name that is not UTF-8, is an error naming it: the validator would
/// refuse the `.luma`, and a folder walk that skipped them would hide the reason.
pub fn folder_files(folder: &Path) -> Result<Vec<FolderFile>> {
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).with_context(|| say!("LumaLink.CannotRead", path = dir.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| say!("LumaLink.CannotRead", path = dir.display()))?;
            let path = entry.path();
            let kind = entry.file_type().with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
            let relative = path.strip_prefix(folder).expect("under the folder");
            let Some(name) = relative.to_str() else {
                bail!(say!("LumaLink.NameNotUtf8", path = path.display()));
            };
            let name = name.replace('\\', "/");
            if kind.is_symlink() {
                bail!(say!("LumaLink.IsLink", name = name));
            } else if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                out.push(FolderFile { name, path });
            } else {
                bail!(say!("LumaLink.NotFileOrFolder", name = name));
            }
        }
    }
    out.sort_by(|a, b| (a.name != "luma.json", a.name.as_bytes()).cmp(&(b.name != "luma.json", b.name.as_bytes())));
    Ok(out)
}

struct Written {
    name: Vec<u8>,
    method: u16,
    crc: u32,
    packed: u64,
    unpacked: u64,
    offset: u64,
}

fn needs_zip64(w: &Written) -> bool {
    w.packed >= 0xFFFF_FFFF || w.unpacked >= 0xFFFF_FFFF || w.offset >= 0xFFFF_FFFF
}

/// What `write` made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteSummary {
    pub files: usize,
    pub unpacked: u64,
    pub bytes: u64,
}

/// Writes `files` as a `.luma` to `out` (a new file; nothing else is touched). Each file is read once; a file whose
/// size changes while it is read is an error naming it.
pub fn write(files: &[FolderFile], out: &Path) -> Result<WriteSummary> {
    let file = fs::File::create(out).with_context(|| say!("LumaLink.CannotWrite", path = out.display()))?;
    let mut sink = Counting { inner: std::io::BufWriter::with_capacity(1 << 20, file), at: 0 };
    let mut written = Vec::with_capacity(files.len());
    let mut unpacked_total = 0u64;
    for f in files {
        let size = fs::metadata(&f.path).with_context(|| say!("LumaLink.CannotRead", path = f.path.display()))?.len();
        if size > MAX_FILE {
            bail!(say!("LumaLink.TooBig", name = f.name, size = size));
        }
        let lower = f.name.to_ascii_lowercase();
        let store = size > STREAM_ABOVE || STORED_ENDINGS.iter().any(|e| lower.ends_with(e));
        let offset = sink.at;
        if store {
            let crc = crc_of(&f.path, size)?;
            let w = Written { name: f.name.as_bytes().to_vec(), method: 0, crc, packed: size, unpacked: size, offset };
            local_header(&mut sink, &w)?;
            copy_exact(&f.path, size, &mut sink)?;
            written.push(w);
        } else {
            let data = fs::read(&f.path).with_context(|| say!("LumaLink.CannotRead", path = f.path.display()))?;
            if data.len() as u64 != size {
                bail!(say!("LumaLink.ChangedWhileRead", name = f.name));
            }
            let crc = crc32fast::hash(&data);
            let packed = miniz_oxide::deflate::compress_to_vec(&data, DEFLATE_LEVEL);
            let (method, body): (u16, &[u8]) = if packed.len() < data.len() { (8, &packed) } else { (0, &data) };
            let w = Written { name: f.name.as_bytes().to_vec(), method, crc, packed: body.len() as u64, unpacked: size, offset };
            local_header(&mut sink, &w)?;
            sink.write_all(body)?;
            written.push(w);
        }
        unpacked_total += size;
    }
    let directory_at = sink.at;
    for w in &written {
        central_header(&mut sink, w)?;
    }
    let directory_size = sink.at - directory_at;
    end_records(&mut sink, written.len() as u64, directory_at, directory_size)?;
    sink.inner.flush()?;
    let bytes = sink.at;
    sink.inner.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    Ok(WriteSummary { files: written.len(), unpacked: unpacked_total, bytes })
}

struct Counting<W: Write> {
    inner: W,
    at: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.at += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn crc_of(path: &Path, size: u64) -> Result<u32> {
    let mut file = fs::File::open(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    if total != size {
        bail!(say!("LumaLink.ChangedWhileRead", name = path.display()));
    }
    Ok(hasher.finalize())
}

fn copy_exact(path: &Path, size: u64, sink: &mut impl Write) -> Result<()> {
    let mut file = fs::File::open(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
    let copied = std::io::copy(&mut (&mut file).take(size), sink)?;
    let mut more = [0u8; 1];
    if copied != size || file.read(&mut more)? != 0 {
        bail!(say!("LumaLink.ChangedWhileRead", name = path.display()));
    }
    Ok(())
}

const UTF8_FLAG: u16 = 1 << 11;
/// 1980-01-01 00:00 in MS-DOS form.
const DOS_DATE: u16 = (1 << 5) | 1;

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn le64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn local_header(sink: &mut impl Write, w: &Written) -> Result<()> {
    let big = needs_zip64(w);
    let mut h = Vec::with_capacity(30 + w.name.len() + 20);
    le32(&mut h, 0x0403_4b50);
    le16(&mut h, if big { 45 } else { 20 });
    le16(&mut h, UTF8_FLAG);
    le16(&mut h, w.method);
    le16(&mut h, 0);
    le16(&mut h, DOS_DATE);
    le32(&mut h, w.crc);
    le32(&mut h, if big { 0xFFFF_FFFF } else { w.packed as u32 });
    le32(&mut h, if big { 0xFFFF_FFFF } else { w.unpacked as u32 });
    le16(&mut h, w.name.len() as u16);
    le16(&mut h, if big { 20 } else { 0 });
    h.extend_from_slice(&w.name);
    if big {
        le16(&mut h, 0x0001);
        le16(&mut h, 16);
        le64(&mut h, w.unpacked);
        le64(&mut h, w.packed);
    }
    sink.write_all(&h)?;
    Ok(())
}

fn central_header(sink: &mut impl Write, w: &Written) -> Result<()> {
    let big = needs_zip64(w);
    let mut h = Vec::with_capacity(46 + w.name.len() + 28);
    le32(&mut h, 0x0201_4b50);
    le16(&mut h, if big { 45 } else { 20 }); // made by: MS-DOS host, spec version; no host's own attributes
    le16(&mut h, if big { 45 } else { 20 });
    le16(&mut h, UTF8_FLAG);
    le16(&mut h, w.method);
    le16(&mut h, 0);
    le16(&mut h, DOS_DATE);
    le32(&mut h, w.crc);
    le32(&mut h, if big { 0xFFFF_FFFF } else { w.packed as u32 });
    le32(&mut h, if big { 0xFFFF_FFFF } else { w.unpacked as u32 });
    le16(&mut h, w.name.len() as u16);
    le16(&mut h, if big { 28 } else { 0 });
    le16(&mut h, 0); // comment
    le16(&mut h, 0); // disk
    le16(&mut h, 0); // internal attributes
    le32(&mut h, 0); // external attributes
    le32(&mut h, if big { 0xFFFF_FFFF } else { w.offset as u32 });
    h.extend_from_slice(&w.name);
    if big {
        le16(&mut h, 0x0001);
        le16(&mut h, 24);
        le64(&mut h, w.unpacked);
        le64(&mut h, w.packed);
        le64(&mut h, w.offset);
    }
    sink.write_all(&h)?;
    Ok(())
}

fn end_records(sink: &mut Counting<impl Write>, count: u64, at: u64, size: u64) -> Result<()> {
    let big = count >= 0xFFFF || at >= 0xFFFF_FFFF || size >= 0xFFFF_FFFF;
    let mut h = Vec::with_capacity(98);
    if big {
        let record_at = sink.at;
        le32(&mut h, 0x0606_4b50);
        le64(&mut h, 44);
        le16(&mut h, 45);
        le16(&mut h, 45);
        le32(&mut h, 0);
        le32(&mut h, 0);
        le64(&mut h, count);
        le64(&mut h, count);
        le64(&mut h, size);
        le64(&mut h, at);
        le32(&mut h, 0x0706_4b50);
        le32(&mut h, 0);
        le64(&mut h, record_at);
        le32(&mut h, 1);
    }
    le32(&mut h, 0x0605_4b50);
    le16(&mut h, 0);
    le16(&mut h, 0);
    le16(&mut h, if big { 0xFFFF } else { count as u16 });
    le16(&mut h, if big { 0xFFFF } else { count as u16 });
    le32(&mut h, if big { 0xFFFF_FFFF } else { size as u32 });
    le32(&mut h, if big { 0xFFFF_FFFF } else { at as u32 });
    le16(&mut h, 0);
    sink.write_all(&h)?;
    Ok(())
}

/// One entry of a `.luma`, as `list` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub stored: bool,
    pub packed: u64,
    pub unpacked: u64,
}

/// A `.luma` opened once: its directory read once and its entries found by name, so reading all of them costs the
/// file once (a `.luma` may hold tens of thousands of entries; opening it again for each made a check quadratic).
/// Of two entries with one name the first is the one read (the validator refuses the file for it, L103).
pub struct Reader {
    path: PathBuf,
    bytes: FileBytes,
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

impl Reader {
    /// The `.luma` at `path`. A directory that does not read is an error with the validator's reason (`lumalink check`
    /// names every problem; this only reads).
    pub fn open(path: &Path) -> Result<Self> {
        let mut bytes = FileBytes::open(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
        let entries = lzip::read_directory(&mut bytes).map_err(|_| anyhow::anyhow!(say!("LumaLink.NotReadableLuma", path = path.display())))?;
        let mut by_name = HashMap::with_capacity(entries.len());
        for (at, entry) in entries.iter().enumerate() {
            if let Some(name) = &entry.name {
                by_name.entry(name.clone()).or_insert(at);
            }
        }
        Ok(Self { path: path.to_path_buf(), bytes, entries, by_name })
    }

    /// The file entries, in the file's order.
    pub fn list(&self) -> Vec<Listed> {
        self.files()
            .map(|e| Listed {
                name: e.name.clone().unwrap_or_else(|| String::from_utf8_lossy(&e.raw_name).into_owned()),
                stored: e.method == Some(lzip::Method::Stored),
                packed: e.packed,
                unpacked: e.unpacked,
            })
            .collect()
    }

    /// Whether an entry has this name.
    pub fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// One entry's bytes, checking its size and CRC.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let at = *self.by_name.get(name).with_context(|| say!("LumaLink.NoEntry", luma = self.path.display(), entry = name))?;
        unpack_checked(&mut self.bytes, &self.entries[at], &self.path)
    }

    fn files(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == EntryKind::File)
    }
}

/// The entries of a `.luma`, in the file's order. A directory that does not read is an error with the validator's
/// reason (`lumalink check` names every problem; this only lists).
pub fn list(path: &Path) -> Result<Vec<Listed>> {
    Ok(Reader::open(path)?.list())
}

/// Reads one entry's bytes, checking its size and CRC. For more than one entry, [`Reader`] reads the file once.
pub fn read_entry(path: &Path, name: &str) -> Result<Vec<u8>> {
    Reader::open(path)?.read(name)
}

fn unpack_checked(bytes: &mut dyn Bytes, entry: &Entry, path: &Path) -> Result<Vec<u8>> {
    let name = entry.name.as_deref().unwrap_or("?");
    match lzip::unpack(bytes, entry, true, entry.unpacked) {
        Unpacked::Done { bytes: Some(data), size, crc_ok: true } if size == entry.unpacked => Ok(data),
        Unpacked::Done { .. } | Unpacked::OverLimit => bail!(say!("LumaLink.EntryDiffers", luma = path.display(), entry = name)),
        Unpacked::Broken => bail!(say!("LumaLink.EntryBroken", luma = path.display(), entry = name)),
    }
}

/// Writes the entries of a `.luma` whose names are in `names` (all when empty) under `into`. The `.luma` must pass
/// the validator's container rules (L100 to L107) first, so every name is one that is safe to write on every system;
/// the caller runs the validator and passes its verdict.
pub fn extract(path: &Path, names: &[String], into: &Path) -> Result<Vec<String>> {
    let mut bytes = FileBytes::open(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
    let entries = lzip::read_directory(&mut bytes).map_err(|_| anyhow::anyhow!(say!("LumaLink.NotReadableLuma", path = path.display())))?;
    let mut done = Vec::new();
    // A name asked for is an entry, or a folder: the entries under it.
    let under = |name: &str, wanted: &str| name == wanted || name.strip_prefix(wanted).is_some_and(|rest| rest.starts_with('/'));
    for wanted in names {
        if !entries.iter().any(|e| e.name.as_deref().is_some_and(|n| under(n, wanted))) {
            bail!(say!("LumaLink.NoEntry", luma = path.display(), entry = wanted));
        }
    }
    for entry in entries.iter().filter(|e| e.kind == EntryKind::File) {
        let Some(name) = entry.name.as_deref() else { continue };
        if !names.is_empty() && !names.iter().any(|w| under(name, w)) {
            continue;
        }
        let data = unpack_checked(&mut bytes, entry, path)?;
        let target = into.join(name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| say!("LumaLink.CannotMake", path = parent.display()))?;
        }
        if target.exists() {
            bail!(say!("LumaLink.AlreadyThere", path = target.display()));
        }
        fs::write(&target, &data).with_context(|| say!("LumaLink.CannotWrite", path = target.display()))?;
        done.push(name.to_string());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, data) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        dir
    }

    #[test]
    fn the_order_is_luma_json_then_names_by_bytes() {
        // By bytes, not by a locale: `Z` (0x5A) before `a` (0x61).
        let dir = folder(&[("layers/base/files/a.txt", b"a"), ("luma.json", b"{}"), ("layers/base/files/Z.txt", b"Z"), ("media/a.png", b"p")]);
        let names: Vec<String> = folder_files(dir.path()).unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["luma.json", "layers/base/files/Z.txt", "layers/base/files/a.txt", "media/a.png"]);
    }

    #[test]
    fn what_is_written_reads_back_and_is_the_same_every_time() {
        let text = "Kuribo ".repeat(1000);
        let dir = folder(&[("luma.json", b"{\"id\": \"x\"}"), ("layers/base/files/a.txt", text.as_bytes()), ("media/icon.png", b"\x89PNG not really"), ("layers/base/files/empty", b"")]);
        let files = folder_files(dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let (one, two) = (out.path().join("one.luma"), out.path().join("two.luma"));
        let summary = write(&files, &one).unwrap();
        write(&files, &two).unwrap();
        assert_eq!(fs::read(&one).unwrap(), fs::read(&two).unwrap());
        assert_eq!(summary.files, 4);
        let listed = list(&one).unwrap();
        assert_eq!(listed.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["luma.json", "layers/base/files/a.txt", "layers/base/files/empty", "media/icon.png"]);
        let a = listed.iter().find(|l| l.name.ends_with("a.txt")).unwrap();
        assert!(!a.stored && a.packed < a.unpacked, "text is deflated: {a:?}");
        assert!(listed.iter().find(|l| l.name.ends_with(".png")).unwrap().stored, "a picture is stored");
        assert_eq!(read_entry(&one, "layers/base/files/a.txt").unwrap(), text.as_bytes());
        let into = out.path().join("open");
        let got = extract(&one, &["layers".to_string()], &into).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(fs::read(into.join("layers/base/files/a.txt")).unwrap(), text.as_bytes());
        assert!(extract(&one, &["layers".to_string()], &into).is_err(), "never writes over a file");
        assert!(extract(&one, &["nothing".to_string()], &into).is_err());
    }

    #[test]
    fn the_validator_reads_what_is_written_entry_for_entry() {
        let dir = folder(&[("luma.json", b"{}"), ("layers/base/files/x.bin", &[7u8; 5000])]);
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("m.luma");
        write(&folder_files(dir.path()).unwrap(), &path).unwrap();
        let mut bytes = FileBytes::open(&path).unwrap();
        let entries = lzip::read_directory(&mut bytes).unwrap();
        assert_eq!(entries.len(), 2);
        for e in &entries {
            assert!(!e.local_name_differs && !e.encrypted && e.kind == EntryKind::File);
            assert!(matches!(lzip::unpack(&mut bytes, e, false, e.unpacked), Unpacked::Done { crc_ok: true, .. }));
        }
        assert!(lzip::overlaps(&entries).is_empty());
    }

    #[test]
    fn zip64_records_are_written_and_read_when_needed() {
        // The directory records go Zip64 at 65,535 entries; one file per entry, written fast because each is empty.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("luma.json"), b"{}").unwrap();
        let files_dir = dir.path().join("layers/base/files");
        fs::create_dir_all(&files_dir).unwrap();
        for i in 0..65_600 {
            fs::write(files_dir.join(format!("{i:05}")), b"").unwrap();
        }
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("many.luma");
        let summary = write(&folder_files(dir.path()).unwrap(), &path).unwrap();
        assert_eq!(summary.files, 65_601);
        assert_eq!(list(&path).unwrap().len(), 65_601);
    }
}
