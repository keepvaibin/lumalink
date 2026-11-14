//! The code patches LumaLink cannot turn into settings, written out for the modder: each with the game
//! function it changes, the game's and the mod's instructions, and a LumaScript hook to start from, as comments in
//! `source/code-patches.lum`. The game never reads `source/`.

use crate::plan::UnknownCode;
use crate::{game_functions, known_patches, ppc, say};
use std::collections::BTreeSet;
use std::fmt::Write;

/// Where in the package the list goes.
pub const FILE: &str = "source/code-patches.lum";

/// The list for the mod `name`, and how many changes it names; `None` when there are none.
pub fn text(name: &str, unknown: &[UnknownCode]) -> Option<(String, usize)> {
    if unknown.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut said = 0;
    let mut lists = 0;
    let mut seen = BTreeSet::new();
    let _ = writeln!(out, "// {}", say!("LumaLink.Stub.Header", name = name));
    for u in unknown {
        if u.value.is_empty() && (u.kind == "code list" || u.kind == "code file") {
            lists += 1;
            continue;
        }
        if !seen.insert((u.release, u.own, u.kind, u.value.clone(), u.original.clone())) {
            continue;
        }
        said += 1;
        out.push_str("//\n");
        one(&mut out, u);
    }
    if lists != 0 {
        said += lists;
        out.push_str("//\n");
        let _ = writeln!(out, "// {}", say!("LumaLink.Stub.CodeList", count = lists));
    }
    Some((out, said))
}

fn one(out: &mut String, u: &UnknownCode) {
    let own = u.own | 0x8000_0000;
    let count = if u.value.is_empty() { u.bytes } else { u.value.len() };
    let release_line = |out: &mut String| {
        if u.release != 'E' && u.release != '\0' {
            let _ = writeln!(out, "// {}", say!("LumaLink.Stub.Release", game = format!("RMG{}01", u.release)));
        }
    };
    if u.kind == "search" {
        let _ = writeln!(out, "// {}", say!("LumaLink.Stub.Search", address = format!("0x{own:08X}"), count = count));
        listing(out, "LumaLink.Stub.Was", &words(&u.original));
        listing(out, "LumaLink.Stub.Now", &words(&u.value));
        return;
    }
    let function = game_functions::at(u.address | 0x8000_0000);
    match function {
        Some(f) => {
            let offset = (u.address | 0x8000_0000) - f.start;
            let named = format!("{}{}", f.name, f.params);
            let _ = writeln!(out, "// {}", say!("LumaLink.Stub.In", function = named, offset = format!("0x{offset:X}"), count = count));
        }
        None if known_patches::is_spare_memory(u.address) => {
            let _ = writeln!(out, "// {}", say!("LumaLink.Stub.OwnCode", address = format!("0x{own:08X}"), count = count));
        }
        None => {
            let _ = writeln!(out, "// {}", say!("LumaLink.Stub.Outside", address = format!("0x{own:08X}"), count = count));
        }
    }
    release_line(out);
    if u.value.is_empty() {
        return;
    }
    // Code reads as instructions; anything else as the words it is.
    let code = function.is_some() || known_patches::is_spare_memory(u.address);
    let show = |bytes: &[u8]| if code { ppc::listing(bytes, own) } else { addressed_words(bytes, own) };
    if function.is_some() {
        if u.original.is_empty() {
            let _ = writeln!(out, "// {}", say!("LumaLink.Stub.WasUnknown"));
        } else {
            listing(out, "LumaLink.Stub.Was", &show(&u.original));
        }
    }
    listing(out, "LumaLink.Stub.Now", &show(&u.value));
    let Some(f) = function else {
        return;
    };
    // A hook needs a name the language can say: not a static initializer or a function of an unnamed namespace.
    if f.name.contains(' ') {
        return;
    }
    let args = match f.arity() {
        Some(0) => String::new(),
        Some(1) => "arg".to_string(),
        Some(n) => (1..=n).map(|i| format!("arg{i}")).collect::<Vec<_>>().join(", "),
        None => "...".to_string(),
    };
    let _ = writeln!(out, "// hook before {}({args}) {{", f.name);
    // A function made to return at once is a hook that skips it.
    if u.address | 0x8000_0000 == f.start && u.value.get(..4) == Some(&[0x4E, 0x80, 0x00, 0x20]) {
        out.push_str("//     skip\n");
    }
    out.push_str("// }\n");
}

fn listing(out: &mut String, title: &str, lines: &[String]) {
    let _ = writeln!(out, "// {}", crate::text::say(title, &[]));
    for line in lines {
        let _ = writeln!(out, "//     {line}");
    }
}

/// Bytes as words, for a search (no address: it is wherever the search finds them).
fn words(bytes: &[u8]) -> Vec<String> {
    bytes.chunks(4).map(|w| w.iter().map(|b| format!("{b:02X}")).collect::<String>()).collect()
}

/// Bytes outside code, as words at their addresses.
fn addressed_words(bytes: &[u8], at: u32) -> Vec<String> {
    bytes.chunks(4).enumerate().map(|(i, w)| format!("0x{:08X}  {}", at.wrapping_add(4 * i as u32), w.iter().map(|b| format!("{b:02X}")).collect::<String>())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_patch_names_its_function_its_instructions_and_a_hook() {
        // MarioActor::control starts at 0x802B0754 in the first release: made to return at once, then changed inside.
        let at_start = UnknownCode {
            address: 0x802B_0754,
            bytes: 4,
            kind: "memory",
            release: 'E',
            own: 0x802B_0754,
            value: vec![0x4E, 0x80, 0x00, 0x20],
            original: vec![0x94, 0x21, 0xFF, 0xF0],
        };
        let inside = UnknownCode { address: 0x802B_0778, own: 0x802B_0778, value: vec![0x38, 0x60, 0x00, 0x01], original: Vec::new(), ..at_start.clone() };
        let pal = UnknownCode { release: 'P', own: 0x802B_1000, ..inside.clone() };
        let own_code = UnknownCode { address: 0x8000_1800, own: 0x8000_1800, value: vec![0x60, 0, 0, 0, 0x4E, 0x80, 0, 0x20, 0xAB], original: Vec::new(), ..at_start.clone() };
        let data = UnknownCode { address: 0x8000_0010, own: 0x8000_0010, value: vec![0x3F, 0x80, 0, 0], ..own_code.clone() };
        let search = UnknownCode { kind: "search", value: vec![0x60, 0, 0, 0], original: vec![0x38, 0x60, 0, 1], ..data.clone() };
        let list = UnknownCode { kind: "code list", ..Default::default() };
        let all = [at_start.clone(), at_start.clone(), inside, pal, own_code, data, search, list.clone(), list];
        let (text, count) = text("Test", &all).unwrap();
        // The repeated change is said once; the two unreadable codes are counted together.
        assert_eq!(count, 8, "{text}");
        assert!(text.lines().all(|l| l.starts_with("//")), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.iter().position(|l| l.contains("MarioActor::control()") && l.contains("0x0")).unwrap();
        assert_eq!(lines[start + 2], "//     0x802B0754  stwu r1, -0x10(r1)", "{text}");
        assert_eq!(lines[start + 4], "//     0x802B0754  blr", "{text}");
        assert_eq!(&lines[start + 5..start + 8], ["// hook before MarioActor::control() {", "//     skip", "// }"], "{text}");
        let inside = lines.iter().position(|l| l.contains("MarioActor::control()") && l.contains("0x24")).unwrap();
        assert_eq!(lines[inside + 1], format!("// {}", say!("LumaLink.Stub.WasUnknown")));
        assert_eq!(lines[inside + 3], "//     0x802B0778  li r3, 0x1", "{text}");
        assert_eq!(&lines[inside + 4..inside + 6], ["// hook before MarioActor::control() {", "// }"], "{text}");
        // The PAL patch is listed at its own address, under the function it changes in the first release.
        assert!(text.contains(&say!("LumaLink.Stub.Release", game = "RMGP01")), "{text}");
        assert!(text.contains("//     0x802B1000  li r3, 0x1"), "{text}");
        // The mod's own code reads as instructions with no hook; data as words; a search with no address.
        assert!(text.contains("//     0x80001804  blr\n//     0x80001808  .byte 0xAB\n"), "{text}");
        assert!(text.contains("//     0x80000010  3F800000\n"), "{text}");
        assert!(text.contains("//     38600001\n"), "{text}");
        assert!(text.contains(&say!("LumaLink.Stub.CodeList", count = 2)), "{text}");
        assert_eq!(text.matches("hook before").count(), 3, "{text}");
        assert!(super::text("Test", &[]).is_none());
    }
}
