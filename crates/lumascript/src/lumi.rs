//! A library mod's interface (docs/modding/language/compiled-form.md): the `lumi1` signature of each exported function
//! (item 3) and the interface file `<mod id>.lumi` (item 7), written by the library's build and read by the compiler
//! of a mod that `use`s it.
//!
//! The file is UTF-8 LumaScript declarations only. A line `module <path>` starts each module of the library, and
//! what follows, up to the next such line, is that module's `use`s and `pub` items as the library's source writes
//! them: its structs and enums whole (a struct's functions without bodies), each function as its head, followed by a
//! comment with its signature. Nothing in it runs.

use crate::ast::{self, ItemKind, Pos};
use crate::check::Source;
use crate::diag::Diagnostic;
use std::fmt::Write as _;

/// SHA-256 of `data` (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// The layout of lumart's own values (text blocks, lists, closures) that cross between two mods' libraries. Two
/// libraries built with different layouts must never bind: every export's signature is taken over it, so the kernel
/// refuses the bind at load (L505 Name.FunctionSig), and a mod's build refuses a needed mod whose interface file
/// carries another (S0160). 1: every build before text rows; 2: a text block may be a row of the game's messages.
pub const RUNTIME_LAYOUT: u32 = 2;

/// An export's signature (item 3): `lumi1:` and the first 8 bytes of SHA-256 over `lumart<RUNTIME_LAYOUT>:` and its
/// canonical text `<path>(<param types>)-><result type>`, in lowercase hex.
pub fn signature(canonical: &str) -> String {
    let h = sha256(format!("lumart{RUNTIME_LAYOUT}:{canonical}").as_bytes());
    let mut s = String::from("lumi1:");
    for b in &h[..8] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// A position's place in `chars` (line and column from 1, columns counting characters as the lexer does).
fn offset(starts: &[usize], at: Pos, len: usize) -> usize {
    starts.get(at.line.saturating_sub(1) as usize).map_or(len, |s| (s + at.col.saturating_sub(1) as usize).min(len))
}

/// The interface file of library `mod_id`: per module (path, source text, parsed), its `use`s and `pub` structs,
/// enums and functions; `sig(module index, function name)` gives an exported function's signature, None for one that
/// is not exported (its types do not cross yet: a caller's call of it is refused, at its line, when it is built).
pub fn write(mod_id: &str, version: &str, modules: &[(&str, &str, &ast::Module)], sig: &dyn Fn(usize, &str) -> Option<String>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {mod_id} {version}: what it gives other mods. Declarations only;");
    let _ = writeln!(out, "// written by LumaLink's build of {mod_id}. A mod reads it through `use {mod_id}.<module>` with {mod_id} in its needs.");
    let _ = writeln!(out, "// runtime layout {RUNTIME_LAYOUT}");
    for (m, (path, text, module)) in modules.iter().enumerate() {
        let chars: Vec<char> = text.chars().collect();
        let mut starts = vec![0usize];
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                starts.push(i + 1);
            }
        }
        let lines: Vec<String> = text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect();
        // Each item's start, with the comment lines right above it.
        let begin = |item: &ast::Item| -> usize {
            let mut line = item.at.line as usize;
            while line >= 2 && lines.get(line - 2).is_some_and(|l| l.trim_start().starts_with("//")) {
                line -= 1;
            }
            starts.get(line - 1).copied().unwrap_or(chars.len())
        };
        let item_starts: Vec<usize> = module.items.iter().map(begin).collect();
        let _ = writeln!(out, "\nmodule {path}");
        for (i, item) in module.items.iter().enumerate() {
            let from = item_starts[i];
            let to = item_starts.get(i + 1).copied().unwrap_or(chars.len());
            let piece = |a: usize, b: usize| -> String { chars[a.min(b)..b].iter().collect::<String>() };
            match &item.kind {
                ItemKind::Use(ast::Use::C(_)) => {}
                ItemKind::Use(_) => {
                    let _ = writeln!(out, "{}", piece(from, to).trim_end());
                }
                ItemKind::Fn(f) if item.public => {
                    let head = piece(from, offset(&starts, f.body.at, chars.len()));
                    match sig(m, &f.name.name) {
                        Some(s) => {
                            let _ = writeln!(out, "{}  // {s}", head.trim_end());
                        }
                        None => {
                            let _ = writeln!(out, "{}  // not exported: a type of it does not cross yet", head.trim_end());
                        }
                    }
                }
                // A task: called from another mod, it gives a pending (11.1a), of `()` without a result.
                ItemKind::Task { name, body, .. } if item.public => {
                    let head = piece(from, offset(&starts, body.at, chars.len()));
                    match sig(m, &name.name) {
                        Some(s) => {
                            let _ = writeln!(out, "{}  // {s}", head.trim_end());
                        }
                        None => {
                            let _ = writeln!(out, "{}  // not exported: a type of it does not cross yet", head.trim_end());
                        }
                    }
                }
                ItemKind::Struct(s) if item.public => {
                    // The struct with its functions' bodies left out.
                    let mut text = String::new();
                    let mut at = from;
                    for f in &s.fns {
                        let a = offset(&starts, f.body.at, chars.len());
                        let b = (offset(&starts, f.body.end, chars.len()) + 1).min(chars.len());
                        if a < at || a > to {
                            continue;
                        }
                        text.push_str(piece(at, a).trim_end());
                        at = b;
                    }
                    text.push_str(&piece(at, to));
                    let _ = writeln!(out, "{}", text.trim_end());
                }
                ItemKind::Enum(_) if item.public => {
                    let _ = writeln!(out, "{}", piece(from, to).trim_end());
                }
                _ => {}
            }
        }
    }
    out
}

/// The runtime layout an interface file's library was built with: its header's `// runtime layout <n>`; 1 when it
/// has none (built before the layout was written).
pub fn layout_of(text: &str) -> u32 {
    for line in text.split('\n') {
        let l = line.trim_end_matches('\r');
        if l.starts_with("module ") {
            break;
        }
        if let Some(n) = l.strip_prefix("// runtime layout ").and_then(|n| n.trim().parse().ok()) {
            return n;
        }
    }
    1
}

/// One module's part of an interface file: its path, the file's line its text starts on (from 1: a place at line `n`
/// of the part is line `first_line + n - 1` of the file), and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub path: String,
    pub first_line: u32,
    pub text: String,
}

/// An interface file cut into its modules' parts, as [`read`] reads them.
pub fn parts(text: &str) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let l = line.trim_end_matches('\r');
        if let Some(path) = l.strip_prefix("module ").filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '/')) {
            parts.push(Part { path: path.to_string(), first_line: i as u32 + 2, text: String::new() });
            continue;
        }
        match parts.last_mut() {
            Some(part) => {
                part.text.push_str(l);
                part.text.push('\n');
            }
            // The header's comments, before the first module, keep the lines of each module as they are.
            None => {}
        }
    }
    parts
}

/// Reads an interface file into the library's modules, each parsed with [`crate::parse_interface`]. The errors are
/// by module path.
pub fn read(text: &str) -> Result<Vec<Source>, Vec<(String, Diagnostic)>> {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for Part { path, text: body, .. } in parts(text) {
        let (module, diags) = crate::parse_interface(&body);
        errors.extend(diags.into_iter().map(|d| (path.clone(), d)));
        out.push(Source { path, module });
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answers() {
        let hex = |b: [u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(signature("abc"), "lumi1:d6b34f3b6257033c");
    }

    #[test]
    fn an_interface_is_written_without_bodies_and_read_back() {
        let src = "use ./item\n\n// How hard it hit.\npub enum Attack { Spin, Trample }\n\nfn hidden() {}\n\n/// Moves it.\npub fn move_forward(a: LiveActor, speed: f32) {\n    let x = 1\n}\n\npub struct Box { w: f32, fn area(self) -> f32 { return self.w } }\n";
        let (module, d) = crate::parse_module(src);
        assert!(d.is_empty(), "{d:?}");
        let text = write("lumakit", "1.0.0", &[("actor", src, &module)], &|_, f| (f == "move_forward").then(|| "lumi1:0011223344556677".to_string()));
        assert!(text.contains("\nmodule actor\nuse ./item\n// How hard it hit.\npub enum Attack { Spin, Trample }\n"), "{text}");
        assert!(text.contains("/// Moves it.\npub fn move_forward(a: LiveActor, speed: f32)  // lumi1:0011223344556677\n"), "{text}");
        assert!(text.contains("pub struct Box { w: f32, fn area(self) -> f32 }\n"), "{text}");
        // The layout it was built with, read back; a file from before the layout was written is layout 1.
        assert_eq!(layout_of(&text), RUNTIME_LAYOUT);
        assert_eq!(layout_of("// lumakit 1.0.0: what it gives other mods.\nmodule actor\n// runtime layout 9\n"), 1);
        assert!(!text.contains("hidden") && !text.contains("let x"), "{text}");
        // A part's first line is the file's line after its `module` line.
        let cut = parts(&text);
        let module_line = text.split('\n').position(|l| l == "module actor").unwrap() as u32 + 1;
        assert_eq!((cut.len(), cut[0].path.as_str(), cut[0].first_line), (1, "actor", module_line + 1));
        assert_eq!(text.split('\n').nth(cut[0].first_line as usize - 1), cut[0].text.split('\n').next());
        let back = read(&text).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].path, "actor");
        let names: Vec<String> = back[0]
            .module
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Fn(f) => Some(f.name.name.clone()),
                ItemKind::Enum(e) => Some(e.name.name.clone()),
                ItemKind::Struct(s) => Some(s.name.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["Attack", "move_forward", "Box"]);
    }
}
