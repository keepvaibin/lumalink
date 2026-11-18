//! LumaLink's updates, the way Nebula's launcher does them. The releases are the GitHub releases of
//! [`REPOSITORY`]; a beta updates to betas only and a release to releases only.
//!
//! - **Checking.** [`newer_to_offer`] reads the release list at most once an hour ([`CHECK_INTERVAL`]) and keeps the
//!   answer in `update-check.json` in [`data_folder`]; a check that fails (offline, rate limit, a malformed list)
//!   keeps the last answer. Only an installed LumaLink checks by itself ([`installed_copy`]), and only while
//!   [`Settings::check_for_updates`] is on; `LUMALINK_NO_UPDATE_CHECK` turns it off for one process.
//! - **Offering.** A newer release is offered unless the person skipped it or a newer one ([`offered`]).
//! - **Installing.** [`download`] fetches the release's Setup for this system (`LumaLink-Setup.exe`,
//!   `LumaLink-Setup-<platform>` on Linux and macOS) and its `.sig`, and keeps the Setup only when its RSA-SHA256
//!   signature verifies against the release key built into LumaLink (`installer/release-public-key.xml`, Nebula's) and
//!   the version written inside its signed bytes is the release's and newer than this one ([`setup_version`]).
//!   [`start_setup`] runs it; it waits for this program to end, installs over the old files and keeps the person's
//!   choices.
//!
//! Requests go through the system's own HTTP stack: WinHTTP on Windows (the system's proxy settings and certificate
//! store, as Nebula's .NET requests), `curl` elsewhere. The signature is checked with Windows' CNG, and elsewhere with
//! the SHA-256 and RSA in [`signature`].

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// This LumaLink's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The GitHub repository whose releases are LumaLink's.
pub const REPOSITORY: &str = "keepvaibin/lumalink";
/// The least time between two checks, in seconds.
pub const CHECK_INTERVAL: u64 = 60 * 60;
/// The longest a check waits for the release list.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// The release key the Setup's signature is checked with (the public half of the key Nebula's releases are signed
/// with).
pub const RELEASE_KEY: &str = include_str!("../../../installer/release-public-key.xml");
/// The Setup's file name in an installed LumaLink's folder, and a download's.
pub const SETUP_NAME: &str = if cfg!(windows) { "LumaLink-Setup.exe" } else { "LumaLink-Setup" };

/// The platform part of a Linux or macOS Setup's name in a release (`LumaLink-Setup-<platform>`).
pub const PLATFORM: &str = if cfg!(target_os = "macos") {
    if cfg!(target_arch = "aarch64") { "macos-arm64" } else { "macos-x64" }
} else if cfg!(target_arch = "aarch64") {
    "linux-arm64"
} else {
    "linux-x64"
};

/// A published release newer than this LumaLink.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub tag: String,
    /// The release's notes (Markdown).
    pub notes: String,
    /// The release's page.
    pub page: String,
    /// The Setup for this system.
    pub package_url: String,
    /// The Setup's signature.
    #[serde(default)]
    pub signature_url: Option<String>,
    /// The package's size in bytes as the list gives it; 0 when it gives none.
    #[serde(default)]
    pub package_size: u64,
}

/// The release list's address: `LUMALINK_RELEASES_URL` (a test server, a fork), else the repository's.
pub fn releases_url() -> String {
    std::env::var("LUMALINK_RELEASES_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30"))
}

/// Whether `version` is a release's version as LumaLink's tags give it: `MAJOR.MINOR.PATCH` or
/// `MAJOR.MINOR.PATCH-beta.N`, each part digits. Nothing else is taken from a release list: the version names the
/// download's folder (`update-<version>`), so a tag with a `/` or `..` would put it elsewhere (N-4039).
pub fn is_version(version: &str) -> bool {
    let digits = |p: &str| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit());
    let (numbers, beta) = match version.split_once("-beta.") {
        Some((n, b)) => (n, Some(b)),
        None => (version, None),
    };
    let parts: Vec<&str> = numbers.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| digits(p)) && beta.is_none_or(digits)
}

/// Whether `version` is a beta (`1.2.0-beta.3`).
pub fn is_beta(version: &str) -> bool {
    version.to_ascii_lowercase().contains("-beta.")
}

/// Compares two versions `MAJOR.MINOR.PATCH[-label.N]`; a release sorts after its betas.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(version: &str) -> [u64; 5] {
        let mut key = [0u64; 5];
        let (numbers, label) = match version.split_once('-') {
            Some((n, l)) => (n, Some(l)),
            None => (version, None),
        };
        for (i, part) in numbers.split('.').take(3).enumerate() {
            key[i] = part.parse().unwrap_or(0);
        }
        key[3] = u64::from(label.is_none());
        if let Some(label) = label {
            key[4] = label.rsplit('.').next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
        key
    }
    key(a).cmp(&key(b))
}

/// The Setup a release of `version` carries for this system: `LumaLink-Setup.exe` or `LumaLink-Beta-Setup.exe` on
/// Windows x64, `LumaLink-Setup-windows-arm64.exe` on Windows on ARM, `LumaLink-Setup-<platform>` or
/// `LumaLink-Beta-Setup-<platform>` elsewhere.
pub fn package_name(version: &str) -> String {
    let setup = if is_beta(version) { "LumaLink-Beta-Setup" } else { "LumaLink-Setup" };
    if cfg!(all(windows, target_arch = "aarch64")) {
        format!("{setup}-windows-arm64.exe")
    } else if cfg!(windows) {
        format!("{setup}.exe")
    } else {
        format!("{setup}-{PLATFORM}")
    }
}

/// The newest release in the list `json` (GitHub's release list) that is newer than `installed`, of its channel, and
/// carries this system's Setup and its signature over HTTPS; none when there is none.
pub fn select_newer(json: &str, installed: &str) -> Result<Option<Release>> {
    let list: serde_json::Value = serde_json::from_str(json).map_err(|_| anyhow::anyhow!(crate::say!("LumaLink.Update.BadList")))?;
    let Some(list) = list.as_array() else { bail!(crate::say!("LumaLink.Update.BadList")) };
    let beta = is_beta(installed);
    let mut best: Option<Release> = None;
    for release in list {
        let text = |name: &str| release.get(name).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if release.get("draft").and_then(|d| d.as_bool()).unwrap_or(false) {
            continue;
        }
        let tag = text("tag_name");
        let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
        if !is_version(&version) || is_beta(&version) != beta {
            continue;
        }
        let floor = best.as_ref().map_or(installed, |b| b.version.as_str());
        if compare_versions(&version, floor) != std::cmp::Ordering::Greater {
            continue;
        }
        let package = package_name(&version);
        let (mut package_url, mut signature_url, mut package_size) = (None, None, 0);
        for asset in release.get("assets").and_then(|a| a.as_array()).into_iter().flatten() {
            let name = asset.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let url = asset.get("browser_download_url").and_then(|u| u.as_str()).map(str::to_string);
            if name == package {
                package_url = url;
                package_size = asset.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
            } else if name == format!("{package}.sig") {
                signature_url = url;
            }
        }
        let (Some(package_url), Some(signature_url)) = (package_url.filter(|u| secure(u)), signature_url.filter(|u| secure(u))) else { continue };
        best = Some(Release { version, tag, notes: text("body"), page: text("html_url"), package_url, signature_url: Some(signature_url), package_size });
    }
    Ok(best)
}

/// Downloads come over HTTPS only (tests serve 127.0.0.1 over HTTP).
fn secure(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
}

/// Whether `release` is to be offered: newer than this LumaLink, of its channel, and newer than the version the person
/// skipped.
pub fn offered(release: &Release, skipped: Option<&str>) -> bool {
    use std::cmp::Ordering::Greater;
    compare_versions(&release.version, VERSION) == Greater
        && is_beta(&release.version) == is_beta(VERSION)
        && skipped.is_none_or(|s| s.is_empty() || compare_versions(&release.version, s) == Greater)
}

/// LumaLink's own folder for its settings and the last check: `LUMALINK_DATA`, else `%LOCALAPPDATA%\LumaLink`
/// (Windows), `~/Library/Application Support/LumaLink` (macOS), `$XDG_CONFIG_HOME/lumalink` or `~/.config/lumalink`.
pub fn data_folder() -> Option<PathBuf> {
    let named = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    if let Some(data) = named("LUMALINK_DATA") {
        return Some(data);
    }
    if cfg!(windows) {
        return named("LOCALAPPDATA").map(|l| l.join("LumaLink"));
    }
    if cfg!(target_os = "macos") {
        return named("HOME").map(|h| h.join("Library").join("Application Support").join("LumaLink"));
    }
    named("XDG_CONFIG_HOME").map(|x| x.join("lumalink")).or_else(|| named("HOME").map(|h| h.join(".config").join("lumalink")))
}

/// The person's update choices (`settings.json` in [`data_folder`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Look for a newer LumaLink when the window opens and when a command runs.
    #[serde(default = "yes")]
    pub check_for_updates: bool,
    /// The version the person chose to skip.
    #[serde(default)]
    pub skipped_version: Option<String>,
    /// When the command line last told of a newer version (seconds since 1970).
    #[serde(default)]
    pub last_notice: u64,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { check_for_updates: true, skipped_version: None, last_notice: 0 }
    }
}

impl Settings {
    /// The saved settings; the defaults when there are none or they do not read.
    pub fn load() -> Settings {
        data_folder()
            .and_then(|d| std::fs::read(d.join("settings.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let folder = data_folder().ok_or_else(|| anyhow::anyhow!(crate::say!("LumaLink.Update.NoFolder")))?;
        write_atomically(&folder.join("settings.json"), &serde_json::to_vec_pretty(self)?)
    }
}

/// Writes `file` whole or not at all ([`crate::files::write_whole`]), making its folder first.
fn write_atomically(file: &Path, bytes: &[u8]) -> Result<()> {
    let folder = file.parent().context("no folder")?;
    std::fs::create_dir_all(folder).with_context(|| folder.display().to_string())?;
    crate::files::write_whole(file, bytes).with_context(|| file.display().to_string())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The last check's answer: when it was made, and the release it found (none: nothing newer).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Answer {
    checked: u64,
    release: Option<Release>,
}

fn answer_file() -> Option<PathBuf> {
    data_folder().map(|d| d.join("update-check.json"))
}

fn last_answer() -> Option<Answer> {
    serde_json::from_slice(&std::fs::read(answer_file()?).ok()?).ok()
}

/// Whether this program is an installed LumaLink, which checks for updates by itself and installs them: Setup's copy
/// beside it. A build checks only when asked.
pub fn installed_copy() -> bool {
    let Some(folder) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return false };
    folder.join(SETUP_NAME).is_file()
}

/// Whether this process looks for updates by itself: an installed copy, with the setting on, and no
/// `LUMALINK_NO_UPDATE_CHECK`.
pub fn checks_by_itself() -> bool {
    std::env::var_os("LUMALINK_NO_UPDATE_CHECK").is_none_or(|v| v.is_empty()) && installed_copy() && Settings::load().check_for_updates
}

/// Reads the release list now and keeps the answer.
pub fn check_now() -> Result<Option<Release>> {
    let json = http::get_text(&releases_url(), "application/vnd.github+json", CHECK_TIMEOUT)?;
    let release = select_newer(&json, VERSION)?;
    if let Some(file) = answer_file() {
        // An answer that cannot be kept is checked again next time.
        let _ = write_atomically(&file, &serde_json::to_vec(&Answer { checked: now(), release: release.clone() })?);
    }
    Ok(release)
}

/// The newer release to offer now: the last answer while it is under `max_age` seconds old (one dated after now, a
/// clock set wrong, is checked again), else a fresh check (the last answer when the check fails); none when it is not
/// [`offered`]. Blocks for the check: the window and the command line call it on a thread of their own.
pub fn newer_to_offer(max_age: u64) -> Option<Release> {
    let last = last_answer();
    let fresh = last.as_ref().is_some_and(|a| a.checked <= now() && now() - a.checked < max_age);
    let release = if fresh { last.and_then(|a| a.release) } else { check_now().ok().flatten().or_else(|| last.and_then(|a| a.release)) };
    let skipped = Settings::load().skipped_version;
    release.filter(|r| offered(r, skipped.as_deref()))
}

/// The newer release the last check found, without a request (the command line's notice).
pub fn last_found() -> Option<Release> {
    let skipped = Settings::load().skipped_version;
    last_answer().and_then(|a| a.release).filter(|r| offered(r, skipped.as_deref()))
}

/// The least time between two notices of the command line, and between its checks, in seconds.
pub const NOTICE_INTERVAL: u64 = 24 * 60 * 60;

/// Checks again when the last answer is older than `max_age` seconds (the command line's check, on a thread it does
/// not wait for: its answer is told by a later command).
pub fn refresh(max_age: u64) {
    let fresh = last_answer().is_some_and(|a| a.checked <= now() && now() - a.checked < max_age);
    if !fresh {
        let _ = check_now();
    }
}

/// The command line's notice of a newer version: the release the last check found, at most once a day ([`NOTICE_INTERVAL`]).
pub fn notice() -> Option<Release> {
    let release = last_found()?;
    let mut settings = Settings::load();
    let now = now();
    if settings.last_notice <= now && now - settings.last_notice < NOTICE_INTERVAL {
        return None;
    }
    settings.last_notice = now;
    settings.save().ok()?;
    Some(release)
}

/// The version a LumaLink Setup carries in its signed bytes (`LUMALINK-SETUP-VERSION=<version>;`, written by the
/// Setup's build); none without one, or with two that differ.
pub fn setup_version(bytes: &[u8]) -> Option<String> {
    // Kept reversed here, so the program reading it never carries the mark itself.
    let needle: Vec<u8> = b"=NOISREV-PUTES-KNILAMUL".iter().rev().copied().collect();
    let mut found: Option<String> = None;
    let mut at = 0;
    while let Some(i) = bytes[at..].windows(needle.len()).position(|w| w == needle.as_slice()) {
        let start = at + i + needle.len();
        let rest = &bytes[start..bytes.len().min(start + 41)];
        if let Some(end) = rest.iter().position(|b| *b == b';') {
            let version = &rest[..end];
            if !version.is_empty() && version.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'.' || *b == b'-') {
                let version = String::from_utf8_lossy(version).into_owned();
                match &found {
                    Some(other) if *other != version => return None,
                    _ => found = Some(version),
                }
            }
        }
        at = start;
    }
    found
}

/// Standard Base64 (the signature file, the key's numbers), whitespace ignored.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count, mut padding) = (0u32, 0u32, 0usize);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            _ => return None,
        };
        if padding > 0 {
            return None;
        }
        bits = bits << 6 | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    (padding <= 2).then_some(out)
}

/// The modulus and exponent of an RSA public key in .NET's XML form (`<RSAKeyValue><Modulus>`, `<Exponent>`).
fn key_numbers(xml: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let number = |name: &str| doc.descendants().find(|n| n.has_tag_name(name)).and_then(|n| n.text()).and_then(base64);
    let mut modulus = number("Modulus")?;
    let exponent = number("Exponent")?;
    while modulus.first() == Some(&0) {
        modulus.remove(0);
    }
    (!modulus.is_empty() && !exponent.is_empty()).then_some((modulus, exponent))
}

/// Whether `signature` (Base64, as the `.sig` file holds it) is an RSA PKCS#1 v1.5 SHA-256 signature of `file` by
/// the key `key_xml`.
pub fn verify_signature(file: &Path, signature: &str, key_xml: &str) -> bool {
    let (Some(signature), Some((modulus, exponent))) = (base64(signature), key_numbers(key_xml)) else { return false };
    #[cfg(windows)]
    let checked = cng::sha256_of(file).is_some_and(|hash| cng::verify(&modulus, &exponent, &hash, &signature));
    #[cfg(not(windows))]
    let checked = signature::sha256_of(file).is_some_and(|hash| signature::verify(&modulus, &exponent, &hash, &signature));
    checked
}

#[cfg_attr(windows, allow(dead_code))]
pub mod signature {
    //! SHA-256 (FIPS 180-4) and RSA PKCS#1 v1.5 verification (RFC 8017 8.2.2) for the systems without CNG. Only public
    //! data passes through them (the release key, a signature, a download), so they need no constant time.
    use std::io::Read;
    use std::path::Path;

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    /// SHA-256 fed in pieces.
    pub struct Sha256 {
        state: [u32; 8],
        block: [u8; 64],
        filled: usize,
        length: u64,
    }

    impl Default for Sha256 {
        fn default() -> Sha256 {
            Sha256 { state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], block: [0; 64], filled: 0, length: 0 }
        }
    }

    impl Sha256 {
        pub fn update(&mut self, mut data: &[u8]) {
            self.length = self.length.wrapping_add(data.len() as u64);
            while !data.is_empty() {
                let n = (64 - self.filled).min(data.len());
                self.block[self.filled..self.filled + n].copy_from_slice(&data[..n]);
                self.filled += n;
                data = &data[n..];
                if self.filled == 64 {
                    let block = self.block;
                    self.compress(&block);
                    self.filled = 0;
                }
            }
        }

        pub fn finish(mut self) -> [u8; 32] {
            let bits = self.length.wrapping_mul(8);
            self.update(&[0x80]);
            while self.filled != 56 {
                self.update(&[0]);
            }
            self.update(&bits.to_be_bytes());
            let mut out = [0u8; 32];
            for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
                chunk.copy_from_slice(&word.to_be_bytes());
            }
            out
        }

        fn compress(&mut self, block: &[u8; 64]) {
            let mut w = [0u32; 64];
            for (i, chunk) in block.chunks_exact(4).enumerate() {
                w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let choice = (e & f) ^ (!e & g);
                let t1 = h.wrapping_add(s1).wrapping_add(choice).wrapping_add(K[i]).wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let majority = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(majority);
                (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
            }
            for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
                *state = state.wrapping_add(value);
            }
        }
    }

    pub fn sha256_of(file: &Path) -> Option<[u8; 32]> {
        let mut file = std::fs::File::open(file).ok()?;
        let mut hash = Sha256::default();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => return Some(hash.finish()),
                Ok(n) => hash.update(&buffer[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }

    /// A number as 32-bit limbs, the lowest first, `limbs` long (big-endian `bytes`, which must fit).
    fn limbs_of(bytes: &[u8], limbs: usize) -> Vec<u32> {
        let mut out = vec![0u32; limbs];
        for (i, byte) in bytes.iter().rev().enumerate() {
            out[i / 4] |= u32::from(*byte) << (8 * (i % 4));
        }
        out
    }

    /// `a >= b`, `a` at least as long as `b` (its extra top limbs counted).
    fn at_least(a: &[u32], b: &[u32]) -> bool {
        if a[b.len()..].iter().any(|l| *l != 0) {
            return true;
        }
        for i in (0..b.len()).rev() {
            if a[i] != b[i] {
                return a[i] > b[i];
            }
        }
        true
    }

    /// `a -= b` (`a >= b`), the borrow carried through `a`'s top limbs.
    fn subtract(a: &mut [u32], b: &[u32]) {
        let mut borrow = 0u64;
        for (i, limb) in a.iter_mut().enumerate() {
            let take = u64::from(*b.get(i).unwrap_or(&0)) + borrow;
            let value = u64::from(*limb);
            borrow = u64::from(value < take);
            *limb = (value + (borrow << 32) - take) as u32;
        }
    }

    /// `a * b mod n` (all `n.len()` limbs, `a` and `b` below `n`).
    fn multiply_mod(a: &[u32], b: &[u32], n: &[u32]) -> Vec<u32> {
        let len = n.len();
        let mut product = vec![0u32; 2 * len];
        for i in 0..len {
            let mut carry = 0u64;
            for j in 0..len {
                let t = u64::from(a[i]) * u64::from(b[j]) + u64::from(product[i + j]) + carry;
                product[i + j] = t as u32;
                carry = t >> 32;
            }
            product[i + len] = carry as u32;
        }
        // The remainder, one bit of the product at a time from the top: doubled, the bit added, n taken off.
        let mut rest = vec![0u32; len + 1];
        for bit in (0..2 * len * 32).rev() {
            let mut carry = (product[bit / 32] >> (bit % 32)) & 1;
            for limb in rest.iter_mut() {
                let next = *limb >> 31;
                *limb = (*limb << 1) | carry;
                carry = next;
            }
            if at_least(&rest, n) {
                subtract(&mut rest, n);
            }
        }
        rest.truncate(len);
        rest
    }

    /// `base ^ exponent mod n` (the exponent big-endian).
    fn power_mod(base: &[u32], exponent: &[u8], n: &[u32]) -> Vec<u32> {
        let mut result = vec![0u32; n.len()];
        result[0] = 1;
        for byte in exponent {
            for bit in (0..8).rev() {
                result = multiply_mod(&result, &result, n);
                if (byte >> bit) & 1 == 1 {
                    result = multiply_mod(&result, base, n);
                }
            }
        }
        result
    }

    /// DigestInfo of SHA-256 (RFC 8017 9.2, note 1), before the hash.
    const SHA256_INFO: [u8; 19] = [0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20];

    /// Whether `signature` is the RSA PKCS#1 v1.5 signature of the SHA-256 `hash` by the key (`modulus`, `exponent`),
    /// big-endian: the signature raised to the exponent is compared whole with the encoding the hash must have.
    pub fn verify(modulus: &[u8], exponent: &[u8], hash: &[u8; 32], signature: &[u8]) -> bool {
        let k = modulus.len();
        if k < SHA256_INFO.len() + 32 + 11 || signature.len() != k || modulus.first() == Some(&0) || exponent.iter().all(|b| *b == 0) {
            return false;
        }
        let limbs = k.div_ceil(4);
        let n = limbs_of(modulus, limbs);
        let s = limbs_of(signature, limbs);
        if at_least(&s, &n) {
            return false;
        }
        let m = power_mod(&s, exponent, &n);
        let mut encoded = vec![0u8; k];
        for (i, byte) in encoded.iter_mut().rev().enumerate() {
            *byte = (m[i / 4] >> (8 * (i % 4))) as u8;
        }
        let mut expected = vec![0xffu8; k];
        expected[0] = 0;
        expected[1] = 1;
        let tail = k - SHA256_INFO.len() - 32;
        expected[tail - 1] = 0;
        expected[tail..tail + SHA256_INFO.len()].copy_from_slice(&SHA256_INFO);
        expected[k - 32..].copy_from_slice(hash);
        encoded == expected
    }
}

#[cfg(windows)]
mod cng {
    //! Windows' own cryptography (CNG): SHA-256 and RSA PKCS#1 v1.5 verification.
    use std::io::Read;
    use std::path::Path;
    use windows_sys::Win32::Security::Cryptography::*;

    pub fn sha256_of(file: &Path) -> Option<[u8; 32]> {
        let mut file = std::fs::File::open(file).ok()?;
        let mut hash: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo-handle needs no provider; the hash handle is destroyed on every path below.
        if unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut hash, std::ptr::null_mut(), 0, std::ptr::null(), 0, 0) } != 0 {
            return None;
        }
        let mut buffer = vec![0u8; 1 << 20];
        let mut out = [0u8; 32];
        let ok = loop {
            match file.read(&mut buffer) {
                Ok(0) => break unsafe { BCryptFinishHash(hash, out.as_mut_ptr(), 32, 0) } == 0,
                Ok(n) => {
                    if unsafe { BCryptHashData(hash, buffer.as_ptr(), n as u32, 0) } != 0 {
                        break false;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break false,
            }
        };
        unsafe { BCryptDestroyHash(hash) };
        ok.then_some(out)
    }

    pub fn verify(modulus: &[u8], exponent: &[u8], hash: &[u8; 32], signature: &[u8]) -> bool {
        // BCRYPT_RSAKEY_BLOB, then the exponent and the modulus, big-endian.
        let header = [
            BCRYPT_RSAPUBLIC_MAGIC,
            (modulus.len() * 8) as u32,
            exponent.len() as u32,
            modulus.len() as u32,
            0,
            0,
        ];
        let mut blob: Vec<u8> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
        blob.extend_from_slice(exponent);
        blob.extend_from_slice(modulus);
        let mut key: BCRYPT_KEY_HANDLE = std::ptr::null_mut();
        // SAFETY: the blob is a complete public-key blob of the length given; the key is destroyed below.
        if unsafe { BCryptImportKeyPair(BCRYPT_RSA_ALG_HANDLE, std::ptr::null_mut(), BCRYPT_RSAPUBLIC_BLOB, &mut key, blob.as_ptr(), blob.len() as u32, 0) } != 0 {
            return false;
        }
        let padding = BCRYPT_PKCS1_PADDING_INFO { pszAlgId: BCRYPT_SHA256_ALGORITHM };
        let status = unsafe {
            BCryptVerifySignature(key, &padding as *const _ as *const core::ffi::c_void, hash.as_ptr(), 32, signature.as_ptr(), signature.len() as u32, BCRYPT_PAD_PKCS1)
        };
        unsafe { BCryptDestroyKey(key) };
        status == 0
    }
}

/// What [`download`] did not do, in words for the person.
#[derive(Debug)]
pub enum DownloadError {
    /// The person cancelled it.
    Cancelled,
    /// The Setup or its signature did not come (the network, the server).
    Fetch(String),
    /// What came is not a LumaLink Setup signed by the release key, or not of the release's version.
    Refused(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str(&crate::say!("LumaLink.Update.Cancelled")),
            DownloadError::Fetch(why) | DownloadError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for DownloadError {}

/// Downloads `release`'s Setup into the data folder and returns it, once its signature and version are checked;
/// `progress` hears the bytes so far and the whole (0 when unknown). A Setup that fails the check is deleted, fetched
/// once more (a release published again under its version has a new signature), and then refused.
pub fn download(release: &Release, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<PathBuf, DownloadError> {
    use crate::say;
    let folder = data_folder().ok_or_else(|| DownloadError::Fetch(say!("LumaLink.Update.NoFolder")))?.join(format!("update-{}", release.version));
    std::fs::create_dir_all(&folder).map_err(|e| DownloadError::Fetch(format!("{}: {e}", folder.display())))?;
    let setup = folder.join(SETUP_NAME);
    let signature_url = release.signature_url.as_deref().ok_or_else(|| DownloadError::Refused(say!("LumaLink.Update.Unsigned", version = release.version)))?;
    // No Setup is larger than this; a transfer that grows past it is something else (a portal's page, a wrong file).
    let most = if release.package_size > 0 { release.package_size } else { 1 << 30 };
    for attempt in 0..2 {
        let _ = std::fs::remove_file(&setup);
        let fetch_error = |e: anyhow::Error| if cancel.load(Ordering::Relaxed) { DownloadError::Cancelled } else { DownloadError::Fetch(say!("LumaLink.Update.FetchFailed", why = format!("{e:#}"))) };
        let mut signature = Vec::new();
        http::get(signature_url, "application/octet-stream", Duration::from_secs(30), 64 * 1024, &mut signature, &mut |_, _| {}, cancel).map_err(fetch_error)?;
        let part = folder.join(format!("{SETUP_NAME}.part"));
        let written = (|| -> Result<()> {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&part).with_context(|| part.display().to_string())?);
            http::get(&release.package_url, "application/octet-stream", Duration::from_secs(60), most, &mut out, progress, cancel)?;
            use std::io::Write;
            out.flush()?;
            Ok(())
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&part);
            return Err(fetch_error(e));
        }
        std::fs::rename(&part, &setup).map_err(|e| DownloadError::Fetch(format!("{}: {e}", setup.display())))?;
        if verify_signature(&setup, &String::from_utf8_lossy(&signature), RELEASE_KEY) {
            // The version is inside the signed bytes: an older signed Setup published under a newer tag (a downgrade
            // or a replay) is refused like an unsigned one.
            let carried = std::fs::read(&setup).ok().and_then(|b| setup_version(&b));
            if carried.as_deref() == Some(release.version.as_str()) && compare_versions(&release.version, VERSION) == std::cmp::Ordering::Greater {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&setup, std::fs::Permissions::from_mode(0o755)).map_err(|e| DownloadError::Fetch(format!("{}: {e}", setup.display())))?;
                }
                return Ok(setup);
            }
            let _ = std::fs::remove_file(&setup);
            return Err(DownloadError::Refused(say!("LumaLink.Update.WrongVersion", version = release.version)));
        }
        let _ = std::fs::remove_file(&setup);
        if attempt == 1 {
            break;
        }
    }
    Err(DownloadError::Refused(say!("LumaLink.Update.Unsigned", version = release.version)))
}

/// Starts a downloaded Setup to install over this LumaLink: it waits for this process to end, installs with the
/// person's last choices showing only its progress, and opens the window again when `open_window`.
pub fn start_setup(setup: &Path, open_window: bool) -> Result<()> {
    let mut command = std::process::Command::new(setup);
    command.arg("--update").arg("--wait").arg(std::process::id().to_string());
    if open_window {
        command.arg("--open");
    }
    // A process group of its own: closing the terminal `lumalink update` ran in does not stop the install.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().with_context(|| setup.display().to_string())?;
    Ok(())
}

/// Removes downloaded Setups of versions no newer than this one (an update that has been installed).
pub fn remove_old_downloads() {
    if let Some(data) = data_folder() {
        remove_old_downloads_in(&data);
    }
}

/// [`remove_old_downloads`] in `data`: in each `update-<version>` folder no newer than this LumaLink, the files
/// [`download`] wrote, then the folder when that leaves it empty.
pub fn remove_old_downloads_in(data: &Path) {
    let Ok(entries) = std::fs::read_dir(data) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(version) = name.strip_prefix("update-") {
            if entry.file_type().is_ok_and(|t| t.is_dir()) && compare_versions(version, VERSION) != std::cmp::Ordering::Greater {
                for f in download_files(&entry.path()) {
                    let _ = std::fs::remove_file(f);
                }
                let _ = std::fs::remove_dir(entry.path());
            }
        }
    }
}

/// The files LumaLink keeps in its data folder ([`data_folder`]): the update choices, the last check's answer and
/// Setup's record. Each downloaded Setup is in an `update-<version>` folder beside them. Nothing else in the folder is
/// LumaLink's: `LUMALINK_DATA` may name a folder that holds other things.
pub const DATA_FILES: [&str; 3] = ["settings.json", "update-check.json", "setup.json"];

/// LumaLink's own files in `data`: [`DATA_FILES`], a write of one of them left half done (`.<name>.<process>.part`),
/// in each `update-<version>` folder the downloaded Setup and a download left half done, in `builds` the build records
/// (`<16 hex digits>.txt`, crate::jobs::record_build) and their half-done writes, and in `running` the window's job
/// files (crate::running). Links are not followed.
pub fn own_data_files(data: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(data).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_file() && (DATA_FILES.contains(&name.as_str()) || half_written(&name)) {
            out.push(entry.path());
        } else if kind.is_dir() && name.strip_prefix("update-").is_some_and(|v| !v.is_empty()) {
            out.extend(download_files(&entry.path()));
        } else if kind.is_dir() && name == crate::jobs::BUILDS_FOLDER {
            out.extend(build_records(&entry.path()));
        } else if kind.is_dir() && name == crate::running::RUNNING_FOLDER {
            out.extend(std::fs::read_dir(entry.path()).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).filter(|p| crate::running::is_job_file(p)));
        }
    }
    out.sort();
    out
}

/// Whether `name` is [`write_atomically`]'s half-done write of one of [`DATA_FILES`].
fn half_written(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.').and_then(|r| r.strip_suffix(".part")) else { return false };
    let Some((file, process)) = rest.rsplit_once('.') else { return false };
    DATA_FILES.contains(&file) && !process.is_empty() && process.bytes().all(|b| b.is_ascii_digit())
}

/// The build records in `builds` (as files, not links): `<16 hex digits>.txt`, and `.<that>.<process>.part`.
fn build_records(folder: &Path) -> Vec<PathBuf> {
    let record = |n: &str| n.strip_suffix(".txt").is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    let half = |n: &str| {
        n.strip_prefix('.').and_then(|r| r.strip_suffix(".part")).and_then(|r| r.rsplit_once('.')).is_some_and(|(f, p)| record(f) && !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut out = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|t| t.is_file()) && (record(&name) || half(&name)) {
            out.push(entry.path());
        }
    }
    out
}

/// The files [`download`] writes in an `update-<version>` folder, those that are there (as files, not links).
fn download_files(folder: &Path) -> Vec<PathBuf> {
    [SETUP_NAME.to_string(), format!("{SETUP_NAME}.part")]
        .iter()
        .map(|n| folder.join(n))
        .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_file()))
        .collect()
}

/// Removes LumaLink's own files from `data` ([`own_data_files`]), then each folder that leaves empty, `data` included.
/// Anything else in `data` stays, and so does every folder that holds it.
pub fn remove_own_data(data: &Path) {
    for f in own_data_files(data) {
        let _ = std::fs::remove_file(&f);
        if let Some(folder) = f.parent().filter(|p| *p != data) {
            let _ = std::fs::remove_dir(folder);
        }
    }
    let _ = std::fs::remove_dir(data);
}

pub mod http {
    //! One GET request through the system's HTTP stack.
    use super::*;

    /// The body of `url` as text (at most 4 MB).
    pub fn get_text(url: &str, accept: &str, timeout: Duration) -> Result<String> {
        let mut body = Vec::new();
        get(url, accept, timeout, 4 << 20, &mut body, &mut |_, _| {}, &AtomicBool::new(false))?;
        String::from_utf8(body).map_err(|_| anyhow::anyhow!(crate::say!("LumaLink.Update.BadList")))
    }

    /// Writes the body of `url` to `out`: at most `most` bytes, `progress` told the bytes so far and the whole (0 when
    /// the server gives none), stopped when `cancel` is set. Fails on any answer but 200.
    #[cfg(windows)]
    pub fn get(url: &str, accept: &str, timeout: Duration, most: u64, out: &mut dyn std::io::Write, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<()> {
        winhttp::get(url, accept, timeout, most, out, progress, cancel)
    }

    /// Writes the body of `url` to `out` through `curl` (Linux and macOS carry it).
    #[cfg(not(windows))]
    pub fn get(url: &str, accept: &str, timeout: Duration, most: u64, out: &mut dyn std::io::Write, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<()> {
        use std::io::Read;
        let mut child = std::process::Command::new("curl")
            .args(["--silent", "--show-error", "--fail", "--location", "--proto", "=https,http"])
            .arg("--max-time")
            .arg(timeout.as_secs().max(1).to_string())
            .arg("--max-filesize")
            .arg(most.to_string())
            .arg("--user-agent")
            .arg(format!("LumaLink/{VERSION}"))
            .arg("--header")
            .arg(format!("Accept: {accept}"))
            .arg(url)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("curl")?;
        let mut stdout = child.stdout.take().context("curl")?;
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                bail!(crate::say!("LumaLink.Update.Cancelled"));
            }
            let n = stdout.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > most {
                let _ = child.kill();
                let _ = child.wait();
                bail!(crate::say!("LumaLink.Update.Net.TooLarge", server = url));
            }
            out.write_all(&buffer[..n])?;
            progress(total, 0);
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(())
    }

    #[cfg(windows)]
    mod winhttp {
        use super::*;
        use core::ffi::c_void;
        use windows_sys::Win32::Networking::WinHttp::*;

        struct Handle(*mut c_void);

        impl Drop for Handle {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe { WinHttpCloseHandle(self.0) };
                }
            }
        }

        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().chain(Some(0)).collect()
        }

        /// The last WinHTTP failure in words (the system has no text for WinHTTP's codes).
        fn failure(server: &str) -> anyhow::Error {
            use crate::say;
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
            anyhow::anyhow!(match code {
                ERROR_WINHTTP_NAME_NOT_RESOLVED | ERROR_WINHTTP_CANNOT_CONNECT => say!("LumaLink.Update.Net.Offline", server = server),
                ERROR_WINHTTP_TIMEOUT => say!("LumaLink.Update.Net.Timeout", server = server),
                ERROR_WINHTTP_CONNECTION_ERROR => say!("LumaLink.Update.Net.Reset", server = server),
                ERROR_WINHTTP_SECURE_FAILURE | ERROR_WINHTTP_SECURE_FAILURE_PROXY => say!("LumaLink.Update.Net.Certificate", server = server),
                _ => say!("LumaLink.Update.Net.Code", server = server, code = code),
            })
        }

        /// `scheme://host[:port]/path`: whether secure, the host, the port and the path with its query.
        fn split(url: &str) -> Result<(bool, String, u16, String)> {
            let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
                (true, r)
            } else if let Some(r) = url.strip_prefix("http://") {
                (false, r)
            } else {
                bail!(crate::say!("LumaLink.Update.Net.BadAddress", address = url));
            };
            let (authority, path) = match rest.find('/') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, "/"),
            };
            let (host, port) = match authority.rsplit_once(':') {
                Some((h, p)) => (h, p.parse().map_err(|_| anyhow::anyhow!(crate::say!("LumaLink.Update.Net.BadAddress", address = url)))?),
                None => (authority, if secure { 443 } else { 80 }),
            };
            if host.is_empty() {
                bail!(crate::say!("LumaLink.Update.Net.BadAddress", address = url));
            }
            Ok((secure, host.to_string(), port, path.to_string()))
        }

        pub fn get(url: &str, accept: &str, timeout: Duration, most: u64, out: &mut dyn std::io::Write, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<()> {
            use crate::say;
            let (secure, host, port, path) = split(url)?;
            let failure = || failure(&host);
            let agent = wide(&format!("LumaLink/{VERSION}"));
            let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
            // SAFETY: every pointer handed to WinHTTP below lives until the call returns; each handle is closed by
            // its Handle.
            unsafe {
                // The system's proxy settings, as the browser and Nebula's .NET requests use them (Windows 8.1 on).
                let mut session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0));
                if session.0.is_null() {
                    session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0));
                }
                if session.0.is_null() {
                    return Err(failure());
                }
                WinHttpSetTimeouts(session.0, ms, ms, ms, ms);
                let host_w = wide(&host);
                let connection = Handle(WinHttpConnect(session.0, host_w.as_ptr(), port, 0));
                if connection.0.is_null() {
                    return Err(failure());
                }
                let verb = wide("GET");
                let path_w = wide(&path);
                let request = Handle(WinHttpOpenRequest(connection.0, verb.as_ptr(), path_w.as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), if secure { WINHTTP_FLAG_SECURE } else { 0 }));
                if request.0.is_null() {
                    return Err(failure());
                }
                let headers = wide(&format!("Accept: {accept}\r\n"));
                WinHttpAddRequestHeaders(request.0, headers.as_ptr(), u32::MAX, WINHTTP_ADDREQ_FLAG_ADD);
                if WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0 || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
                    return Err(failure());
                }
                let mut status: u32 = 0;
                let mut size = 4u32;
                if WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, std::ptr::null(), &mut status as *mut u32 as *mut c_void, &mut size, std::ptr::null_mut()) == 0 {
                    return Err(failure());
                }
                if status != 200 {
                    bail!(say!("LumaLink.Update.Net.Status", server = host, status = status));
                }
                let mut whole: u64 = 0;
                let mut size = 8u32;
                if WinHttpQueryHeaders(request.0, WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER64, std::ptr::null(), &mut whole as *mut u64 as *mut c_void, &mut size, std::ptr::null_mut()) == 0 {
                    whole = 0;
                }
                if whole > most {
                    bail!(say!("LumaLink.Update.Net.TooLarge", server = host));
                }
                let mut buffer = vec![0u8; 64 * 1024];
                let mut total = 0u64;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        bail!(say!("LumaLink.Update.Cancelled"));
                    }
                    let mut read = 0u32;
                    if WinHttpReadData(request.0, buffer.as_mut_ptr() as *mut c_void, buffer.len() as u32, &mut read) == 0 {
                        return Err(failure());
                    }
                    if read == 0 {
                        break;
                    }
                    total += u64::from(read);
                    if total > most {
                        bail!(say!("LumaLink.Update.Net.TooLarge", server = host));
                    }
                    out.write_all(&buffer[..read as usize])?;
                    progress(total, whole);
                }
                if whole > 0 && total != whole {
                    bail!(say!("LumaLink.Update.Net.Short", server = host));
                }
            }
            Ok(())
        }

        #[cfg(test)]
        #[test]
        fn addresses_split() {
            assert_eq!(split("https://api.github.com/repos/a/b/releases?per_page=30").unwrap(), (true, "api.github.com".into(), 443, "/repos/a/b/releases?per_page=30".into()));
            assert_eq!(split("http://127.0.0.1:8080/x").unwrap(), (false, "127.0.0.1".into(), 8080, "/x".into()));
            assert_eq!(split("https://example.com").unwrap(), (true, "example.com".into(), 443, "/".into()));
            assert!(split("ftp://x/y").is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M-153: removing LumaLink's data takes only LumaLink's own files from the folder (LUMALINK_DATA may name one that
    /// holds other things), and the folder only when that leaves it empty.
    #[test]
    fn removing_lumalinks_data_leaves_everything_else_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let put = |rel: &str| {
            let f = data.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "x").unwrap();
        };
        let ours = ["settings.json", "update-check.json", "setup.json", ".settings.json.4242.part"];
        for f in ours {
            put(f);
        }
        put(&format!("update-9.9.9/{SETUP_NAME}"));
        put(&format!("update-9.9.9/{SETUP_NAME}.part"));
        put("builds/0123456789abcdef.txt");
        put("builds/.0123456789abcdef.txt.77.part");
        put("running/4242-7.txt");
        let theirs = ["notes.txt", "photos/a.jpg", "update-photos/b.jpg", ".notes.txt.12.part", "settings.json.bak", "builds/mine.txt"];
        for f in theirs {
            put(f);
        }
        remove_own_data(&data);
        for f in ours {
            assert!(!data.join(f).exists(), "{f} stayed");
        }
        assert!(!data.join("update-9.9.9").exists(), "the emptied download folder stayed");
        assert!(!data.join("builds/0123456789abcdef.txt").exists() && !data.join("builds/.0123456789abcdef.txt.77.part").exists(), "a build record stayed");
        assert!(!data.join("running").exists(), "a job file stayed");
        for f in theirs {
            assert!(data.join(f).is_file(), "{f} was removed");
        }
        // A folder that held only LumaLink's files goes.
        let alone = dir.path().join("alone");
        std::fs::create_dir_all(alone.join("update-1.0.0")).unwrap();
        std::fs::write(alone.join("settings.json"), "x").unwrap();
        std::fs::write(alone.join("update-1.0.0").join(SETUP_NAME), "x").unwrap();
        remove_own_data(&alone);
        assert!(!alone.exists());
    }

    /// M-153: old downloads go file by file; a folder named like one that holds other files keeps them.
    #[test]
    fn old_downloads_take_only_the_setups_they_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        std::fs::create_dir_all(data.join("update-0.0.1")).unwrap();
        std::fs::write(data.join("update-0.0.1").join(SETUP_NAME), "x").unwrap();
        std::fs::create_dir_all(data.join("update-notes")).unwrap();
        std::fs::write(data.join("update-notes").join("plan.txt"), "x").unwrap();
        std::fs::write(data.join("update-notes").join(SETUP_NAME), "x").unwrap();
        remove_old_downloads_in(data);
        assert!(!data.join("update-0.0.1").exists());
        assert!(data.join("update-notes").join("plan.txt").is_file());
    }

    fn list(releases: &[(&str, bool, &[&str])]) -> String {
        let items: Vec<serde_json::Value> = releases
            .iter()
            .map(|(tag, draft, assets)| {
                serde_json::json!({
                    "tag_name": tag, "draft": draft, "body": "notes", "html_url": format!("https://github.com/{REPOSITORY}/releases/tag/{tag}"),
                    "assets": assets.iter().map(|a| serde_json::json!({"name": a, "size": 7, "browser_download_url": format!("https://example.com/{tag}/{a}")})).collect::<Vec<_>>()
                })
            })
            .collect();
        serde_json::to_string(&items).unwrap()
    }

    /// The Setups a release of `version` carries for every system, with their signatures.
    fn packages(version: &str) -> Vec<String> {
        let setup = if is_beta(version) { "LumaLink-Beta-Setup" } else { "LumaLink-Setup" };
        ["exe", "windows-arm64.exe", "linux-x64", "linux-arm64", "macos-arm64", "macos-x64"]
            .iter()
            .map(|p| if *p == "exe" { format!("{setup}.exe") } else { format!("{setup}-{p}") })
            .flat_map(|name| [format!("{name}.sig"), name])
            .collect()
    }

    #[test]
    fn versions_compare_with_betas_before_their_release() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("1.0.1", "1.0.0"), Greater);
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Greater);
        assert_eq!(compare_versions("1.1.0-beta.2", "1.1.0"), Less);
        assert_eq!(compare_versions("1.1.0-beta.10", "1.1.0-beta.9"), Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Equal);
    }

    #[test]
    fn the_newest_release_of_the_channel_with_a_package_is_chosen() {
        let p = |v: &str| packages(v);
        let owned: Vec<(String, bool, Vec<String>)> = vec![
            ("v1.0.1".into(), false, p("1.0.1")),
            ("v1.2.0".into(), true, p("1.2.0")),
            ("v1.1.0".into(), false, p("1.1.0")),
            ("v1.3.0-beta.1".into(), false, p("1.3.0-beta.1")),
            ("v1.1.5".into(), false, vec!["README.md".into()]),
        ];
        let borrowed: Vec<Vec<&str>> = owned.iter().map(|(_, _, a)| a.iter().map(String::as_str).collect()).collect();
        let items: Vec<(&str, bool, &[&str])> = owned.iter().zip(&borrowed).map(|((t, d, _), a)| (t.as_str(), *d, a.as_slice())).collect();
        let json = list(&items);
        let found = select_newer(&json, "1.0.0").unwrap().unwrap();
        assert_eq!(found.version, "1.1.0", "drafts, betas and releases without a package are passed over");
        assert_eq!(found.package_size, 7);
        let setup = package_name("1.1.0");
        assert_eq!(found.package_url, format!("https://example.com/v1.1.0/{setup}"));
        assert_eq!(found.signature_url, Some(format!("https://example.com/v1.1.0/{setup}.sig")));
        assert!(select_newer(&json, "1.1.0").unwrap().is_none());
        let beta = select_newer(&json, "1.2.0-beta.1").unwrap().unwrap();
        assert_eq!(beta.version, "1.3.0-beta.1", "a beta updates to betas");
        assert!(select_newer("{}", "1.0.0").is_err());
        assert!(select_newer("not json", "1.0.0").is_err());
    }

    /// N-4039: a tag that is no version is passed over: its version would name the download's folder, and a `/` or
    /// `..` in it would put the download outside LumaLink's folder (fails before: offered as 9.0.0/../../x).
    #[test]
    fn a_tag_that_is_no_version_is_passed_over() {
        for tag in ["v9.0.0/../../x", "v9.0.0/..", "v9.0.0-beta.1/../x", "v..\\9.0.0", "v9.0", "v9.0.0.1", "v9.0.0-rc.1", "v9.0.0-beta.", "v 9.0.0"] {
            let version = tag.strip_prefix('v').unwrap();
            let sig = format!("{}.sig", package_name(version));
            let json = list(&[(tag, false, &[&package_name(version), &sig])]);
            assert!(select_newer(&json, "1.0.0").unwrap().is_none(), "{tag}");
            assert!(select_newer(&json, "1.0.0-beta.1").unwrap().is_none(), "{tag}");
        }
        for good in ["1.0.0", "10.20.30", "2.0.0-beta.12"] {
            assert!(is_version(good), "{good}");
        }
    }

    #[test]
    fn an_unsigned_setup_is_never_offered() {
        let json = list(&[("v9.0.0", false, &[&package_name("9.0.0")])]);
        assert!(select_newer(&json, "1.0.0").unwrap().is_none());
    }

    #[test]
    fn plain_http_is_refused_but_for_this_computer() {
        assert!(secure("https://github.com/x"));
        assert!(secure("http://127.0.0.1:9000/x"));
        assert!(!secure("http://github.com/x"));
        assert!(!secure("file:///c:/x"));
    }

    #[test]
    fn a_skipped_version_is_offered_again_only_when_a_newer_one_comes() {
        let release = |v: &str| Release { version: v.into(), tag: format!("v{v}"), notes: String::new(), page: String::new(), package_url: String::new(), signature_url: None, package_size: 0 };
        assert!(offered(&release("99.0.0"), None));
        assert!(!offered(&release("99.0.0"), Some("99.0.0")));
        assert!(offered(&release("99.0.1"), Some("99.0.0")));
        assert!(!offered(&release(VERSION), None));
        assert!(!offered(&release("99.0.0-beta.1"), None), "a release does not update to a beta");
    }

    #[test]
    fn the_setup_version_is_read_from_its_bytes() {
        let mark = |v: &str| format!("LUMALINK-SETUP-{}={v};", "VERSION");
        let bytes = format!("MZ....{}....rest", mark("1.2.3")).into_bytes();
        assert_eq!(setup_version(&bytes).as_deref(), Some("1.2.3"));
        let two = format!("{} {}", mark("1.2.3"), mark("1.2.4")).into_bytes();
        assert_eq!(setup_version(&two), None, "two versions that differ are none");
        assert_eq!(setup_version(b"no mark here"), None);
        assert_eq!(setup_version(mark("1.0.0 bad").as_bytes()), None);
        assert_eq!(setup_version(format!("{}{}", mark("2.0.0-beta.1"), "LUMALINK-SETUP-VERSION=").as_bytes()).as_deref(), Some("2.0.0-beta.1"));
    }

    #[test]
    fn base64_reads_the_key_and_refuses_junk() {
        assert_eq!(base64("TWFu").unwrap(), b"Man");
        assert_eq!(base64("TWE=").unwrap(), b"Ma");
        assert_eq!(base64("TQ==\n").unwrap(), b"M");
        assert!(base64("T!==").is_none());
        let (modulus, exponent) = key_numbers(RELEASE_KEY).unwrap();
        assert_eq!(modulus.len(), 384, "the release key is RSA 3072");
        assert_eq!(exponent, [1, 0, 1]);
    }

    /// SHA-256 against FIPS 180-4's examples, the long ones fed in uneven pieces.
    #[test]
    fn sha256_gives_the_standard_digests() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let of = |pieces: &[&[u8]]| {
            let mut h = signature::Sha256::default();
            pieces.iter().for_each(|p| h.update(p));
            hex(h.finish())
        };
        assert_eq!(of(&[b""]), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(of(&[b"abc"]), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(of(&[&long[..5], &long[5..], &[]]), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        let million = vec![b'a'; 1_000_000];
        assert_eq!(of(&[&million[..1], &million[1..]]), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    /// The signature check against a test key (`tests/data/update`, made with .NET's RSACryptoServiceProvider as the
    /// release script signs): the signed file passes; a changed byte, another key or a damaged signature fails. The
    /// portable check is run on every system, beside the system's own.
    #[test]
    fn a_signature_verifies_only_for_its_file_and_key() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/update");
        let key = std::fs::read_to_string(data.join("test-public-key.xml")).unwrap();
        let signature = std::fs::read_to_string(data.join("signed.bin.sig")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("signed.bin");
        std::fs::copy(data.join("signed.bin"), &file).unwrap();
        assert!(verify_signature(&file, &signature, &key));
        assert!(!verify_signature(&file, &signature, RELEASE_KEY), "another key");
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[10] ^= 1;
        std::fs::write(&file, &bytes).unwrap();
        assert!(!verify_signature(&file, &signature, &key), "a changed byte");
        bytes[10] ^= 1;
        std::fs::write(&file, &bytes).unwrap();
        assert!(!verify_signature(&file, &signature.replacen('A', "B", 1), &key), "a damaged signature");
        assert!(!verify_signature(&file, "not base64!", &key));
        let (modulus, exponent) = key_numbers(&key).unwrap();
        let hash = signature::sha256_of(&data.join("signed.bin")).unwrap();
        let bytes = base64(&signature).unwrap();
        assert!(signature::verify(&modulus, &exponent, &hash, &bytes));
        let mut other = hash;
        other[31] ^= 1;
        assert!(!signature::verify(&modulus, &exponent, &other, &bytes), "another hash");
        assert!(!signature::verify(&modulus, &exponent, &hash, &bytes[1..]), "a signature of the wrong length");
    }

    /// A check against a server on this computer: the list is read, the answer kept, and a later offer within the hour
    /// comes from the kept answer with no request.
    #[test]
    fn a_check_reads_the_list_and_keeps_the_answer() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let names = packages("99.0.0");
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let body = list(&[("v99.0.0", false, &names)]).replace("https://example.com", &format!("http://127.0.0.1:{port}"));
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            request
        });
        let data = tempfile::tempdir().unwrap();
        // The variables are this test's alone: no other test reads them.
        std::env::set_var("LUMALINK_DATA", data.path());
        std::env::set_var("LUMALINK_RELEASES_URL", format!("http://127.0.0.1:{port}/releases"));
        let found = newer_to_offer(CHECK_INTERVAL).expect("the list's release");
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /releases "), "{request}");
        assert!(request.to_ascii_lowercase().contains("accept: application/vnd.github+json"), "{request}");
        assert_eq!(found.version, "99.0.0");
        // The server is gone: the kept answer serves the next offer.
        assert_eq!(newer_to_offer(CHECK_INTERVAL).map(|r| r.version).as_deref(), Some("99.0.0"));
        let mut settings = Settings::load();
        settings.skipped_version = Some("99.0.0".into());
        settings.save().unwrap();
        assert!(newer_to_offer(CHECK_INTERVAL).is_none(), "a skipped version is not offered");
        assert!(last_found().is_none());
        std::env::remove_var("LUMALINK_DATA");
        std::env::remove_var("LUMALINK_RELEASES_URL");
    }
}
