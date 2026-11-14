//! Where things are in other versions of the program, against the first release (RMGE01): each run of addresses and how far it lies from there. A patch written for another version reads as one for that release. Numbers only, from symbol tables.

const RUNS_P: &[(u32, i32)] = &[
    (0x80004000, 0),
    (0x80224440, -8),
    (0x8022474C, -28),
    (0x80495EA0, 0),
];

const RUNS_J: &[(u32, i32)] = &[
    (0x80004000, 0),
    (0x8039EFB8, 4),
    (0x80495E80, 32),
    (0x8052DF00, 0),
];

const RUNS_K: &[(u32, i32)] = &[
    (0x80004000, 0),
    (0x8000D410, 8),
    (0x8000E2EC, 24),
    (0x8000ED1C, 112),
    (0x8001520C, 124),
    (0x800156F8, -868),
    (0x80033B10, -884),
    (0x8007A208, -904),
    (0x8009CE20, -1136),
    (0x8009DA70, -1156),
    (0x800A24A4, -1136),
    (0x8017AE58, -1144),
    (0x8017B8E0, -1344),
    (0x8017D8C8, -1388),
    (0x801FA514, -1340),
    (0x801FB16C, -1324),
    (0x8022496C, -1332),
    (0x80224C78, -1352),
    (0x802420B4, -1368),
    (0x80275714, -1412),
    (0x802999DC, -1456),
    (0x802A0B10, -1468),
    (0x802A136C, -1476),
    (0x802A390C, -1508),
    (0x8030ADC8, -1524),
    (0x80340A74, -1568),
    (0x80340DB8, -2028),
    (0x80341248, -2332),
    (0x80341390, -2356),
    (0x80350A4C, -2428),
    (0x80351A74, -2440),
    (0x80351FA4, -2444),
    (0x80352614, -2532),
    (0x803529CC, -2572),
    (0x8035357C, -3048),
    (0x80359424, -3052),
    (0x80359F60, -3208),
    (0x8035A8F0, -3228),
    (0x8035AA20, -3236),
    (0x8035D434, -3264),
    (0x8036755C, -5328),
    (0x80367D3C, -5860),
    (0x80367FA4, -5428),
    (0x803719E0, -5452),
    (0x8037AD64, -5460),
    (0x8037BF4C, -5656),
    (0x8037C55C, -5704),
    (0x8038150C, -5776),
    (0x80382290, -5784),
    (0x80382654, -5808),
    (0x80386AB8, -5672),
    (0x803873A8, -5276),
    (0x80388504, -5280),
    (0x80389E44, -5324),
    (0x8038DFA8, -5340),
    (0x8038E7C0, -5468),
    (0x8038E8C0, -5492),
    (0x8038ED78, -5500),
    (0x80398EA0, -5460),
    (0x8039C1A0, -5488),
    (0x8039E584, -5484),
    (0x803A0678, -5540),
    (0x803A093C, -5600),
    (0x803B1994, -5700),
    (0x803CED14, -5768),
    (0x803CF2A0, -5784),
    (0x803CF7EC, -5892),
    (0x803CFADC, -6016),
    (0x803D6EF4, -5912),
    (0x803DF1F0, -6052),
    (0x803DF41C, -5772),
    (0x803E9350, -5788),
    (0x803FBCFC, -5792),
    (0x803FC414, -5800),
    (0x803FCBCC, -5808),
    (0x803FCDFC, -5820),
    (0x803FD940, -5832),
    (0x80408978, -5892),
    (0x80409844, -5912),
    (0x80409B0C, -6068),
    (0x80409E54, -6072),
    (0x8040A098, -6096),
    (0x8040A364, -6176),
    (0x8040A5BC, -6220),
    (0x8040AB10, -6496),
    (0x8044FD00, -6512),
    (0x804520D0, -6648),
    (0x80452E60, -6640),
    (0x80462554, -7832),
    (0x804625E0, -7532),
    (0x804659CC, -7940),
    (0x80468A20, -8168),
    (0x8046914C, -8172),
    (0x8046AAB0, -8320),
    (0x8046B510, -8408),
    (0x8046BC3C, -8704),
    (0x8046C034, -8772),
    (0x804980E0, -8768),
    (0x804A7450, -5756),
    (0x804A84B0, -5776),
    (0x804AB44C, -5780),
    (0x804AE15C, -5828),
    (0x804AFD00, -43760),
    (0x804B0940, -9348),
    (0x804BB0B4, -9344),
    (0x804C0330, -9352),
    (0x804C7B78, -9388),
    (0x804CA9BC, -9232),
    (0x804CC75C, -5328),
    (0x8051B834, -5520),
    (0x80524420, -5524),
    (0x8052CA00, -5520),
    (0x8052E800, -5504),
    (0x8053943C, -5512),
    (0x8053A780, -5544),
    (0x8053B360, -5576),
    (0x8053B890, -5624),
    (0x8053D8A8, -5632),
    (0x8055C0B8, -83456),
    (0x80564A18, -83480),
    (0x80574BB0, -83496),
    (0x80575710, -83480),
    (0x80589AF0, -83548),
    (0x80589ECC, -83552),
    (0x805ADFF4, -83560),
    (0x805B6CB0, -83576),
    (0x805D1958, -83688),
    (0x805D1A38, -83736),
    (0x805D3EA8, -83824),
    (0x805D4AA4, -83840),
    (0x805D54A0, -83836),
    (0x805D551C, -83832),
    (0x805D6524, -84704),
    (0x805D8A84, -84716),
    (0x805D8BA4, -84720),
    (0x805D9114, -84736),
    (0x805D91CC, -84720),
    (0x805DB5F4, -84728),
    (0x805DBA80, -84736),
    (0x805DFBA8, -84752),
    (0x805E1030, -84784),
    (0x805EA180, -84800),
    (0x805F0AA0, -84040),
    (0x805FD188, -84536),
    (0x805FDED0, -84544),
    (0x80600680, -84352),
    (0x80600E70, -84360),
    (0x8060A3D0, -84352),
    (0x806B1638, -84360),
    (0x806B2468, -84384),
    (0x806B24E8, -84392),
    (0x806B2AA8, -84400),
    (0x806B2B90, -84408),
    (0x806B2E64, -84416),
    (0x806B65DC, -84428),
    (0x806B65F8, -84432),
    (0x806B72E0, -84416),
    (0x806B75F0, -84408),
    (0x806B7620, -84416),
    (0x806B7760, -84424),
    (0x806B7900, -84416),
    (0x806B7A40, -84384),
    (0x806C0F60, -84408),
    (0x806C2300, -84384),
];

/// An address in a version of the game, as the first release has it. The first release and versions with no table come back as they are.
pub fn to_first_release(address: u32, version: char) -> u32 {
    let runs = match version {
        'P' => RUNS_P,
        'J' => RUNS_J,
        'K' => RUNS_K,
        _ => return address,
    };
    let address = address | 0x8000_0000;
    match runs.binary_search_by_key(&address, |r| r.0) {
        Ok(at) => address.wrapping_add_signed(runs[at].1),
        Err(0) => address,
        Err(at) => address.wrapping_add_signed(runs[at - 1].1),
    }
}

/// Each word of a table that points into the program, read for the first release.
pub fn pointers_to_first_release(value: &[u8], version: char) -> Vec<u8> {
    let mut out = value.to_vec();
    if matches!(version, 'P' | 'J' | 'K') {
        for word in out.chunks_exact_mut(4) {
            let number = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            if (0x8000_0000..0x8180_0000).contains(&number) {
                word.copy_from_slice(&to_first_release(number, version).to_be_bytes());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_release_stays_and_others_move() {
        assert_eq!(to_first_release(0x8000_4000, 'E'), 0x8000_4000);
        assert_ne!(RUNS_P.len(), 0);
        assert_ne!(RUNS_J.len(), 0);
    }
}
