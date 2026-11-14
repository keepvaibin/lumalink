//! The game's functions by address: where a code patch lands, named. The first release's (RMGE01; a patch
//! for another release is read through `region_map` first), made by `data/game_functions.py` from a symbol table: each
//! function's start, size, name and parameter types. Numbers and names only.

use std::sync::OnceLock;

const TABLE: &[u8] = include_bytes!("../data/game_functions.z");

/// One function of the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function {
    pub start: u32,
    pub size: u32,
    /// As C++ names it: `MarioActor::control`, `MR::isDead`, `JGeometry::TVec3<float>::operator=`.
    pub name: &'static str,
    /// Its parameter types as C++ writes them, `(const LiveActor*)`, `(...) const`; `?` when the table does not know.
    pub params: &'static str,
}

impl Function {
    /// The number of parameters, when the table knows them.
    pub fn arity(&self) -> Option<usize> {
        let inner = self.params.strip_prefix('(')?.rsplit_once(')')?.0;
        if inner.is_empty() {
            return Some(0);
        }
        // Commas at the top level only: a parameter may itself be a function type.
        let mut depth = 0i32;
        let mut count = 1;
        for c in inner.chars() {
            match c {
                '(' | '<' => depth += 1,
                ')' | '>' => depth -= 1,
                ',' if depth == 0 => count += 1,
                _ => {}
            }
        }
        Some(count)
    }
}

fn all() -> &'static [Function] {
    static ALL: OnceLock<Vec<Function>> = OnceLock::new();
    ALL.get_or_init(|| {
        let bytes = miniz_oxide::inflate::decompress_to_vec_zlib(TABLE).expect("the function table is LumaLink's own");
        let text: &'static str = Box::leak(String::from_utf8(bytes).expect("the function table is UTF-8").into_boxed_str());
        text.lines()
            .filter_map(|line| {
                let mut parts = line.split('\t');
                let start = u32::from_str_radix(parts.next()?, 16).ok()?;
                let size = u32::from_str_radix(parts.next()?, 16).ok()?;
                Some(Function { start, size, name: parts.next()?, params: parts.next()? })
            })
            .collect()
    })
}

/// The function holding `address` (the first release's), if any.
pub fn at(address: u32) -> Option<Function> {
    let table = all();
    let after = table.partition_point(|f| f.start <= address);
    let f = *table.get(after.checked_sub(1)?)?;
    (address - f.start < f.size.max(1)).then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_names_its_function() {
        // MarioActor::control is 0x802B0754, 0x128 bytes, in the first release.
        let control = at(0x802B_0754).unwrap();
        assert_eq!((control.name, control.params, control.size, control.arity()), ("MarioActor::control", "()", 0x128, Some(0)));
        assert_eq!(at(0x802B_0754 + 0x127), Some(control));
        assert_ne!(at(0x802B_0754 + 0x128), Some(control));
        // Below the program, nothing.
        assert_eq!(at(0x8000_0000), None);
        // Overloads keep their own parameter types.
        let dead: Vec<&str> = all().iter().filter(|f| f.name == "MR::isDead").map(|f| f.params).collect();
        assert!(dead.contains(&"(const LiveActor*)") && dead.contains(&"(const LayoutActor*)"), "{dead:?}");
        let dead = all().iter().find(|f| f.name == "MR::isDead").unwrap();
        assert_eq!(dead.arity(), Some(1));
        assert_eq!(Function { params: "(void (int, int)*, long)", ..*dead }.arity(), Some(2));
        assert_eq!(Function { params: "?", ..*dead }.arity(), None);
        assert!(all().len() > 40_000);
        assert!(all().windows(2).all(|w| w[0].start <= w[1].start));
    }
}
