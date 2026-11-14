//! The Wii's instructions as text: what a code patch wrote and what was there, for its report. The
//! integer, branch, load and store, floating point and paired-single forms a mod writes in practice, in the assembler's
//! simplified spellings (`li`, `lis`, `mr`, `nop`, `blr`, `slwi`, `beq`); any other word is `.long 0x...`.

/// A signed number as the assembler writes an offset: `0x10`, `-0x8`.
fn signed(v: i32) -> String {
    if v < 0 {
        format!("-0x{:X}", -(v as i64))
    } else {
        format!("0x{v:X}")
    }
}

fn simm(w: u32) -> i32 {
    i32::from(w as u16 as i16)
}

/// A branch's target: relative to `at` unless absolute.
fn target(at: u32, offset: i32, absolute: bool) -> String {
    let to = if absolute { offset as u32 } else { at.wrapping_add(offset as u32) };
    format!("0x{to:08X}")
}

/// The condition of a conditional branch (BO, BI) in its simplified mnemonic's words, or None for a plain `bc`.
fn condition(bo: u32, bi: u32) -> Option<(String, String)> {
    let cr = bi / 4;
    let field = if cr == 0 { String::new() } else { format!("cr{cr}, ") };
    let bit = bi % 4;
    let name = match (bo & 0x1E, bit) {
        (12, 0) => "lt",
        (12, 1) => "gt",
        (12, 2) => "eq",
        (12, 3) => "so",
        (4, 0) => "ge",
        (4, 1) => "le",
        (4, 2) => "ne",
        (4, 3) => "ns",
        (20, _) => return Some((String::new(), String::new())),
        (16, _) => return Some(("dnz".into(), String::new())),
        (18, _) => return Some(("dz".into(), String::new())),
        _ => return None,
    };
    Some((name.into(), field))
}

/// D-form loads and stores, by primary opcode: (mnemonic, floating point register).
fn load_store(op: u32) -> Option<(&'static str, bool)> {
    Some(match op {
        32 => ("lwz", false),
        33 => ("lwzu", false),
        34 => ("lbz", false),
        35 => ("lbzu", false),
        36 => ("stw", false),
        37 => ("stwu", false),
        38 => ("stb", false),
        39 => ("stbu", false),
        40 => ("lhz", false),
        41 => ("lhzu", false),
        42 => ("lha", false),
        43 => ("lhau", false),
        44 => ("sth", false),
        45 => ("sthu", false),
        46 => ("lmw", false),
        47 => ("stmw", false),
        48 => ("lfs", true),
        49 => ("lfsu", true),
        50 => ("lfd", true),
        51 => ("lfdu", true),
        52 => ("stfs", true),
        53 => ("stfsu", true),
        54 => ("stfd", true),
        55 => ("stfdu", true),
        _ => return None,
    })
}

/// Opcode 31's X-form register ops and indexed loads and stores, by extended opcode.
fn x_form(xo: u32) -> Option<&'static str> {
    Some(match xo {
        28 => "and",
        60 => "andc",
        124 => "nor",
        284 => "eqv",
        316 => "xor",
        412 => "orc",
        444 => "or",
        476 => "nand",
        24 => "slw",
        536 => "srw",
        792 => "sraw",
        _ => return None,
    })
}

fn x_indexed(xo: u32) -> Option<(&'static str, bool)> {
    Some(match xo {
        23 => ("lwzx", false),
        55 => ("lwzux", false),
        87 => ("lbzx", false),
        119 => ("lbzux", false),
        151 => ("stwx", false),
        183 => ("stwux", false),
        215 => ("stbx", false),
        247 => ("stbux", false),
        279 => ("lhzx", false),
        311 => ("lhzux", false),
        343 => ("lhax", false),
        407 => ("sthx", false),
        439 => ("sthux", false),
        535 => ("lfsx", true),
        567 => ("lfsux", true),
        599 => ("lfdx", true),
        631 => ("lfdux", true),
        663 => ("stfsx", true),
        695 => ("stfsux", true),
        727 => ("stfdx", true),
        759 => ("stfdux", true),
        _ => return None,
    })
}

/// XO-form arithmetic (opcode 31, extended opcode without OE): (mnemonic, takes rB).
fn arithmetic(xo: u32) -> Option<(&'static str, bool)> {
    Some(match xo {
        266 => ("add", true),
        10 => ("addc", true),
        138 => ("adde", true),
        40 => ("subf", true),
        8 => ("subfc", true),
        136 => ("subfe", true),
        235 => ("mullw", true),
        75 => ("mulhw", true),
        11 => ("mulhwu", true),
        491 => ("divw", true),
        459 => ("divwu", true),
        104 => ("neg", false),
        202 => ("addze", false),
        234 => ("addme", false),
        200 => ("subfze", false),
        232 => ("subfme", false),
        _ => return None,
    })
}

/// One instruction word at address `at` as text.
pub fn instruction(w: u32, at: u32) -> String {
    known(w, at).unwrap_or_else(|| format!(".long 0x{w:08X}"))
}

fn known(w: u32, at: u32) -> Option<String> {
    let op = w >> 26;
    let (d, a, b) = ((w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31);
    let rc = if w & 1 != 0 { "." } else { "" };
    Some(match op {
        7 => format!("mulli r{d}, r{a}, {}", signed(simm(w))),
        8 => format!("subfic r{d}, r{a}, {}", signed(simm(w))),
        10 | 11 => {
            let cr = d >> 2;
            let field = if cr == 0 { String::new() } else { format!("cr{cr}, ") };
            if op == 10 {
                format!("cmplwi {field}r{a}, 0x{:X}", w & 0xFFFF)
            } else {
                format!("cmpwi {field}r{a}, {}", signed(simm(w)))
            }
        }
        12 => format!("addic r{d}, r{a}, {}", signed(simm(w))),
        13 => format!("addic. r{d}, r{a}, {}", signed(simm(w))),
        14 if a == 0 => format!("li r{d}, {}", signed(simm(w))),
        14 => format!("addi r{d}, r{a}, {}", signed(simm(w))),
        15 if a == 0 => format!("lis r{d}, 0x{:X}", w & 0xFFFF),
        15 => format!("addis r{d}, r{a}, 0x{:X}", w & 0xFFFF),
        16 => {
            let offset = i32::from((w & 0xFFFC) as u16 as i16);
            let to = target(at, offset, w & 2 != 0);
            let link = if w & 1 != 0 { "l" } else { "" };
            match condition(d, a) {
                Some((name, field)) => format!("b{name}{link} {field}{to}"),
                None => format!("bc{link} {d}, {a}, {to}"),
            }
        }
        17 if w & 2 != 0 => "sc".into(),
        18 => {
            let offset = ((w & 0x03FF_FFFC) << 6) as i32 >> 6;
            let link = if w & 1 != 0 { "l" } else { "" };
            let absolute = if w & 2 != 0 { "a" } else { "" };
            format!("b{link}{absolute} {}", target(at, offset, w & 2 != 0))
        }
        19 => {
            let xo = (w >> 1) & 0x3FF;
            let link = if w & 1 != 0 { "l" } else { "" };
            match xo {
                16 | 528 => {
                    let to = if xo == 16 { "lr" } else { "ctr" };
                    match condition(d, a) {
                        Some((name, field)) => {
                            let field = field.trim_end_matches(", ");
                            format!("b{name}{to}{link}{}{field}", if field.is_empty() { "" } else { " " })
                        }
                        None => format!("bc{to}{link} {d}, {a}"),
                    }
                }
                150 => "isync".into(),
                193 if d == a && a == b => format!("crclr {d}"),
                289 if d == a && a == b => format!("crset {d}"),
                193 => format!("crxor {d}, {a}, {b}"),
                289 => format!("creqv {d}, {a}, {b}"),
                449 => format!("cror {d}, {a}, {b}"),
                257 => format!("crand {d}, {a}, {b}"),
                _ => return None,
            }
        }
        20 | 21 | 23 => {
            let (sh, mb, me) = (b, (w >> 6) & 31, (w >> 1) & 31);
            match op {
                21 if mb == 0 && me == 31 - sh => format!("slwi{rc} r{a}, r{d}, {sh}"),
                21 if sh != 0 && me == 31 && mb == 32 - sh => format!("srwi{rc} r{a}, r{d}, {mb}"),
                21 if sh == 0 && me == 31 => format!("clrlwi{rc} r{a}, r{d}, {mb}"),
                21 => format!("rlwinm{rc} r{a}, r{d}, {sh}, {mb}, {me}"),
                20 => format!("rlwimi{rc} r{a}, r{d}, {sh}, {mb}, {me}"),
                _ => format!("rlwnm{rc} r{a}, r{d}, r{b}, {mb}, {me}"),
            }
        }
        24 if w == 0x6000_0000 => "nop".into(),
        24 => format!("ori r{a}, r{d}, 0x{:X}", w & 0xFFFF),
        25 => format!("oris r{a}, r{d}, 0x{:X}", w & 0xFFFF),
        26 => format!("xori r{a}, r{d}, 0x{:X}", w & 0xFFFF),
        27 => format!("xoris r{a}, r{d}, 0x{:X}", w & 0xFFFF),
        28 => format!("andi. r{a}, r{d}, 0x{:X}", w & 0xFFFF),
        29 => format!("andis. r{a}, r{d}, 0x{:X}", w & 0xFFFF),
        31 => {
            let xo = (w >> 1) & 0x3FF;
            if let Some((name, takes_b)) = arithmetic(xo & 0x1FF) {
                let oe = if xo & 0x200 != 0 { "o" } else { "" };
                return Some(if takes_b { format!("{name}{oe}{rc} r{d}, r{a}, r{b}") } else { format!("{name}{oe}{rc} r{d}, r{a}") });
            }
            if xo == 444 && d == b {
                return Some(format!("mr{rc} r{a}, r{d}"));
            }
            if xo == 124 && d == b {
                return Some(format!("not{rc} r{a}, r{d}"));
            }
            if let Some(name) = x_form(xo) {
                return Some(format!("{name}{rc} r{a}, r{d}, r{b}"));
            }
            if let Some((name, float)) = x_indexed(xo) {
                return Some(format!("{name} {}{d}, r{a}, r{b}", if float { "f" } else { "r" }));
            }
            let spr = a | (b << 5);
            match xo {
                0 | 32 => {
                    let cr = d >> 2;
                    let field = if cr == 0 { String::new() } else { format!("cr{cr}, ") };
                    format!("{} {field}r{a}, r{b}", if xo == 0 { "cmpw" } else { "cmplw" })
                }
                824 => format!("srawi{rc} r{a}, r{d}, {b}"),
                26 => format!("cntlzw{rc} r{a}, r{d}"),
                954 => format!("extsb{rc} r{a}, r{d}"),
                922 => format!("extsh{rc} r{a}, r{d}"),
                339 if spr == 8 => format!("mflr r{d}"),
                339 if spr == 9 => format!("mfctr r{d}"),
                339 if spr == 1 => format!("mfxer r{d}"),
                339 => format!("mfspr r{d}, {spr}"),
                467 if spr == 8 => format!("mtlr r{d}"),
                467 if spr == 9 => format!("mtctr r{d}"),
                467 if spr == 1 => format!("mtxer r{d}"),
                467 => format!("mtspr {spr}, r{d}"),
                19 => format!("mfcr r{d}"),
                144 => format!("mtcrf 0x{:X}, r{d}", (w >> 12) & 0xFF),
                83 => format!("mfmsr r{d}"),
                146 => format!("mtmsr r{d}"),
                598 => "sync".into(),
                86 => format!("dcbf r{a}, r{b}"),
                54 => format!("dcbst r{a}, r{b}"),
                278 => format!("dcbt r{a}, r{b}"),
                470 => format!("dcbi r{a}, r{b}"),
                982 => format!("icbi r{a}, r{b}"),
                1014 => format!("dcbz r{a}, r{b}"),
                4 => format!("tw {d}, r{a}, r{b}"),
                _ => return None,
            }
        }
        3 => format!("twi {d}, r{a}, {}", signed(simm(w))),
        32..=55 => {
            let (name, float) = load_store(op)?;
            format!("{name} {}{d}, {}(r{a})", if float { "f" } else { "r" }, signed(simm(w)))
        }
        56 | 57 | 60 | 61 => {
            let offset = ((w & 0xFFF) << 20) as i32 >> 20;
            let name = ["psq_l", "psq_lu", "", "", "psq_st", "psq_stu"][(op - 56) as usize];
            format!("{name} f{d}, {}(r{a}), {}, {}", signed(offset), (w >> 15) & 1, (w >> 12) & 7)
        }
        59 | 63 => {
            let c = (w >> 6) & 31;
            let s = if op == 59 { "s" } else { "" };
            let xo5 = (w >> 1) & 31;
            let three = match xo5 {
                18 => Some("fdiv"),
                20 => Some("fsub"),
                21 => Some("fadd"),
                _ => None,
            };
            if let Some(name) = three {
                return Some(format!("{name}{s}{rc} f{d}, f{a}, f{b}"));
            }
            let fused = match xo5 {
                28 => Some("fmsub"),
                29 => Some("fmadd"),
                30 => Some("fnmsub"),
                31 => Some("fnmadd"),
                _ => None,
            };
            if let Some(name) = fused {
                return Some(format!("{name}{s}{rc} f{d}, f{a}, f{c}, f{b}"));
            }
            match (op, xo5) {
                (_, 25) => return Some(format!("fmul{s}{rc} f{d}, f{a}, f{c}")),
                (59, 24) => return Some(format!("fres{rc} f{d}, f{b}")),
                (63, 23) => return Some(format!("fsel{rc} f{d}, f{a}, f{c}, f{b}")),
                (63, 26) => return Some(format!("frsqrte{rc} f{d}, f{b}")),
                _ => {}
            }
            if op == 59 {
                return None;
            }
            match (w >> 1) & 0x3FF {
                0 => format!("fcmpu cr{}, f{a}, f{b}", d >> 2),
                32 => format!("fcmpo cr{}, f{a}, f{b}", d >> 2),
                72 => format!("fmr{rc} f{d}, f{b}"),
                40 => format!("fneg{rc} f{d}, f{b}"),
                264 => format!("fabs{rc} f{d}, f{b}"),
                136 => format!("fnabs{rc} f{d}, f{b}"),
                12 => format!("frsp{rc} f{d}, f{b}"),
                14 => format!("fctiw{rc} f{d}, f{b}"),
                15 => format!("fctiwz{rc} f{d}, f{b}"),
                583 => format!("mffs{rc} f{d}"),
                711 => format!("mtfsf{rc} 0x{:X}, f{b}", (w >> 17) & 0xFF),
                38 => format!("mtfsb1{rc} {d}"),
                70 => format!("mtfsb0{rc} {d}"),
                _ => return None,
            }
        }
        4 => {
            let c = (w >> 6) & 31;
            let xo5 = (w >> 1) & 31;
            let three = match xo5 {
                18 => Some("ps_div"),
                20 => Some("ps_sub"),
                21 => Some("ps_add"),
                _ => None,
            };
            if let Some(name) = three {
                return Some(format!("{name}{rc} f{d}, f{a}, f{b}"));
            }
            let four = match xo5 {
                10 => Some("ps_sum0"),
                11 => Some("ps_sum1"),
                14 => Some("ps_madds0"),
                15 => Some("ps_madds1"),
                23 => Some("ps_sel"),
                28 => Some("ps_msub"),
                29 => Some("ps_madd"),
                30 => Some("ps_nmsub"),
                31 => Some("ps_nmadd"),
                _ => None,
            };
            if let Some(name) = four {
                return Some(format!("{name}{rc} f{d}, f{a}, f{c}, f{b}"));
            }
            match xo5 {
                12 => return Some(format!("ps_muls0{rc} f{d}, f{a}, f{c}")),
                13 => return Some(format!("ps_muls1{rc} f{d}, f{a}, f{c}")),
                25 => return Some(format!("ps_mul{rc} f{d}, f{a}, f{c}")),
                _ => {}
            }
            match (w >> 1) & 0x3FF {
                72 => format!("ps_mr{rc} f{d}, f{b}"),
                40 => format!("ps_neg{rc} f{d}, f{b}"),
                264 => format!("ps_abs{rc} f{d}, f{b}"),
                136 => format!("ps_nabs{rc} f{d}, f{b}"),
                528 => format!("ps_merge00{rc} f{d}, f{a}, f{b}"),
                560 => format!("ps_merge01{rc} f{d}, f{a}, f{b}"),
                592 => format!("ps_merge10{rc} f{d}, f{a}, f{b}"),
                624 => format!("ps_merge11{rc} f{d}, f{a}, f{b}"),
                0 => format!("ps_cmpu0 cr{}, f{a}, f{b}", d >> 2),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// A run of bytes at `at` as instructions, one per line ("0x802B0784  li r3, 0x1"); a tail shorter than a word is
/// its bytes.
pub fn listing(bytes: &[u8], at: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut chunks = bytes.chunks_exact(4);
    for (k, word) in chunks.by_ref().enumerate() {
        let here = at.wrapping_add(4 * k as u32);
        let w = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        out.push(format!("0x{here:08X}  {}", instruction(w, here)));
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let here = at.wrapping_add((bytes.len() - rest.len()) as u32);
        let hex: Vec<String> = rest.iter().map(|b| format!("0x{b:02X}")).collect();
        out.push(format!("0x{here:08X}  .byte {}", hex.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_mods_write_read_as_the_assembler_spells_them() {
        let cases: &[(u32, u32, &str)] = &[
            (0x3860_0001, 0, "li r3, 0x1"),
            (0x3860_FC19, 0, "li r3, -0x3E7"),
            (0x6000_0000, 0, "nop"),
            (0x4E80_0020, 0, "blr"),
            (0x4E80_0421, 0, "bctrl"),
            (0x4D82_0020, 0, "beqlr"),
            // known_patches.rs: the miss patch's branch into its code at 0x80002330.
            (0x4BC0_A058, 0x803F_82D8, "b 0x80002330"),
            (0x483A_DFB9, 0x8000_2334, "bl 0x803B02EC"),
            (0x7C08_02A6, 0, "mflr r0"),
            (0x7C08_03A6, 0, "mtlr r0"),
            (0x9421_FFF0, 0, "stwu r1, -0x10(r1)"),
            (0x8001_0014, 0, "lwz r0, 0x14(r1)"),
            (0x2C03_0000, 0, "cmpwi r3, 0x0"),
            (0x2807_0005, 0, "cmplwi r7, 0x5"),
            (0x4182_0010, 0x8000_1000, "beq 0x80001010"),
            (0x4086_FFF0, 0x8000_1000, "bne cr1, 0x80000FF0"),
            (0x7FE3_FB78, 0, "mr r3, r31"),
            (0x7C63_2378, 0, "or r3, r3, r4"),
            (0x5463_103A, 0, "slwi r3, r3, 2"),
            (0x5463_F0BE, 0, "srwi r3, r3, 2"),
            (0x5463_063E, 0, "clrlwi r3, r3, 24"),
            (0x3C60_8040, 0, "lis r3, 0x8040"),
            (0xC023_0000, 0, "lfs f1, 0x0(r3)"),
            (0xEC21_102A, 0, "fadds f1, f1, f2"),
            (0xFC20_0890, 0, "fmr f1, f1"),
            (0x7C64_1A14, 0, "add r3, r4, r3"),
            (0x7C03_2000, 0, "cmpw r3, r4"),
            (0xE021_8008, 0, "psq_l f1, 0x8(r1), 1, 0"),
            (0x1000_0000, 0, "ps_cmpu0 cr0, f0, f0"),
            (0x0000_0000, 0, ".long 0x00000000"),
            (0x7C00_07FE, 0, ".long 0x7C0007FE"),
        ];
        for &(word, at, text) in cases {
            assert_eq!(instruction(word, at), text, "{word:08X}");
        }
        assert_eq!(listing(&[0x60, 0, 0, 0, 0x4E, 0x80], 0x8000_0100), ["0x80000100  nop", "0x80000104  .byte 0x4E, 0x80"]);
    }
}
