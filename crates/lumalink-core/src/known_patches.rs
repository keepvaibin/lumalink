//! Code patches with a native version. A Riivolution memory patch changes the Wii program, which Nebula does not have, so
//! it cannot be carried over byte for byte. Those listed here were studied and each has a tunable in the native game:
//! when a mod's bytes match, the package sets the tunable (`luma.json` "tunables", see `tunable`) to the value read from
//! them. The numbers in `id` only name the changes inside LumaLink.
//!
//! To add one: note the mod's region, address and bytes, find or add the game's tunable in Nebula, and add rows here and
//! in `TUNABLES`.

use serde::Serialize;

/// The changes LumaLink recognises, each one tunable's (`TUNABLES`). 0: nothing to do, Nebula covers it by itself.
pub mod id {
    pub const NATIVE_ALREADY: u32 = 0;
    pub const FLIP_PANEL_GROUP: u32 = 1;
    pub const FLIP_PANEL_OBSERVER_GROUP: u32 = 2;
    pub const WATER_BAZOOKA_SHOT_FLAG: u32 = 3;
    pub const KAMECK_BARRIER_OFFSET_Y: u32 = 4;
    pub const SKELETAL_FISH_BABY_NO_BUBBLE: u32 = 5;
    pub const DODORYU_SHELL_ONLY: u32 = 6;
    pub const MISS_STAR_PIECES: u32 = 7;
    pub const NO_HIDDEN_STAR_TABLE: u32 = 8;
    pub const NO_SUDDEN_DEATH_DODORYU: u32 = 9;
    pub const BEGOMAN_FIRE_PROOF: u32 = 10;
    pub const MESSENGER_ALWAYS_EX: u32 = 11;
    pub const MESSENGER_EX_ONE_UPS: u32 = 12;
    pub const PLAYER_HEALTH: u32 = 13;
    pub const PLAYER_HEALTH_FIXED: u32 = 14;
    pub const HIT_TAKES_ALL: u32 = 15;
    pub const LIFE_UP_HEALTH: u32 = 16;
    pub const NO_LOW_HEALTH_WARNING: u32 = 17;
    pub const NO_DARK_COMET_RULES: u32 = 18;
    pub const MAX_LIVES: u32 = 19;
    pub const MAX_STAR_BITS: u32 = 20;
    pub const MAX_CARRIED_STAR_BITS: u32 = 21;
    pub const MAX_COINS: u32 = 22;
    pub const COINS_PER_ONE_UP: u32 = 23;
    pub const STAR_BITS_PER_ONE_UP: u32 = 24;
    pub const STARTING_LIVES: u32 = 25;
    pub const PURPLE_COINS_FOR_STAR: u32 = 26;
    pub const WATER_AIR: u32 = 27;
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct KnownPatch {
    pub address: u32,
    pub value: &'static str,
    pub id: u32,
    pub parameter: u32,
    /// Its row in LumaLink's text (LumaLink.Patch.*): what the change does, in the player's language with say!.
    pub what: &'static str,
}

use id::*;

const fn row(address: u32, value: &'static str, id: u32, parameter: u32, what: &'static str) -> KnownPatch {
    KnownPatch { address, value, id, parameter, what }
}

pub const KNOWN: &[KnownPatch] = &[
    row(0x80153D84, "39000001", WATER_BAZOOKA_SHOT_FLAG, 1, "LumaLink.Patch.WaterCannonFlag"),
    row(0x80001800, "C4BB8000C03E00103C608000C0031800EC00082AD01E0010809E008C4E800020", KAMECK_BARRIER_OFFSET_Y, 0xC4BB8000, "LumaLink.Patch.KamekBarrierLower"),
    row(0x800382B8, "4BFC954D", KAMECK_BARRIER_OFFSET_Y, 0xC4BB8000, "LumaLink.Patch.KamekBarrierLower"),
    row(0x8007679C, "60000000", SKELETAL_FISH_BABY_NO_BUBBLE, 1, "LumaLink.Patch.NoFishBubble"),
    row(0x80054490, "4836F0A9", DODORYU_SHELL_ONLY, 1, "LumaLink.Patch.DodoryuShellsOnly"),
    row(0x800564C4, "4836D075", DODORYU_SHELL_ONLY, 1, "LumaLink.Patch.DodoryuShellsOnly"),
    row(0x803F82D8, "4BC0A058", MISS_STAR_PIECES, 0xFFFFFC19, "LumaLink.Patch.MissLosesStarBits"),
    row(0x80002330, "3860FC19483ADFB980010034483F5FA0", MISS_STAR_PIECES, 0xFFFFFC19, "LumaLink.Patch.MissLosesStarBits"),
    row(0x803F6298, "38600000", NO_HIDDEN_STAR_TABLE, 1, "LumaLink.Patch.HiddenStarsNoTable"),
    row(0x803F5D40, "386000004E800020", NO_SUDDEN_DEATH_DODORYU, 1, "LumaLink.Patch.NoSuddenDeathDodoryu"),
    row(0x800D02C0, "38600000", BEGOMAN_FIRE_PROOF, 1, "LumaLink.Patch.BegomanFireProof"),
    row(0x800D5DD8, "38600000", BEGOMAN_FIRE_PROOF, 1, "LumaLink.Patch.BegomanFireProof"),
    row(0x800D7D50, "38600000", BEGOMAN_FIRE_PROOF, 1, "LumaLink.Patch.BegomanFireProof"),
    row(0x80275440, "60000000", MESSENGER_ALWAYS_EX, 1, "LumaLink.Patch.MessengerSupply"),
    row(0x8039AA2C, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x8039AAF0, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x804A432C, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x8039AA48, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x8039AB0C, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x804A430C, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x8039BF80, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x8039C044, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
    row(0x804A656C, "60000000", NATIVE_ALREADY, 0, "LumaLink.Patch.CrashDebugger"),
];

/// How a setting's value is read from what a mod writes at its place in the program.
#[derive(Clone, Copy)]
enum Reads {
    /// `li rD, value`, the number a mod chose.
    Number,
    /// `cmpwi rA, value`.
    Compared,
    /// `li rD, 0`: the setting is on.
    Zeroed,
    /// `nop`: the given value.
    Nothing(u32),
    /// An unconditional branch over what was there: the setting is on.
    Skipped,
    /// The function returns zero at once: the setting is on.
    ReturnsZero,
}

struct Setting {
    address: u32,
    id: u32,
    reads: Reads,
    what: &'static str,
}

const fn set(address: u32, id: u32, reads: Reads, what: &'static str) -> Setting {
    Setting { address, id, reads, what }
}

/// The game's settings, by where the program keeps them.
const SETTINGS: &[Setting] = &[
    set(0x801DD690, FLIP_PANEL_OBSERVER_GROUP, Reads::Number, "LumaLink.Patch.FlipPanelObserverGroups"),
    set(0x801DCE48, FLIP_PANEL_GROUP, Reads::Number, "LumaLink.Patch.FlipPanelGroups"),
    set(0x80275458, MESSENGER_EX_ONE_UPS, Reads::Number, "LumaLink.Patch.MessengerOneUps"),
    set(0x802750F0, MESSENGER_EX_ONE_UPS, Reads::Compared, "LumaLink.Patch.MessengerOneUps"),
    set(0x802750FC, MESSENGER_EX_ONE_UPS, Reads::Number, "LumaLink.Patch.MessengerOneUps"),
    set(0x802AE8A4, PLAYER_HEALTH, Reads::Number, "LumaLink.Patch.PlayerHealth"),
    set(0x802AE8C4, PLAYER_HEALTH_FIXED, Reads::Skipped, "LumaLink.Patch.DaredevilHealth"),
    set(0x802B1C2C, HIT_TAKES_ALL, Reads::Zeroed, "LumaLink.Patch.HitTakesAll"),
    set(0x802B1CD8, HIT_TAKES_ALL, Reads::Zeroed, "LumaLink.Patch.HitTakesAll"),
    set(0x802B079C, LIFE_UP_HEALTH, Reads::Number, "LumaLink.Patch.LifeUpHealth"),
    set(0x802B07A0, LIFE_UP_HEALTH, Reads::Nothing(0), "LumaLink.Patch.LifeUpNoHealth"),
    set(0x802DDF50, NO_LOW_HEALTH_WARNING, Reads::Skipped, "LumaLink.Patch.NoLowHealthWarning"),
    set(0x803CDB60, NO_DARK_COMET_RULES, Reads::ReturnsZero, "LumaLink.Patch.NoDaredevilRules"),
    set(0x803B237C, MAX_LIVES, Reads::Number, "LumaLink.Patch.MaxLives"),
    set(0x803B236C, MAX_LIVES, Reads::Number, "LumaLink.Patch.MaxLives"),
    set(0x803B2348, STARTING_LIVES, Reads::Number, "LumaLink.Patch.StartingLives"),
    set(0x803B22E4, STARTING_LIVES, Reads::Number, "LumaLink.Patch.StartingLives"),
    set(0x803B2318, STARTING_LIVES, Reads::Number, "LumaLink.Patch.StartingLives"),
    set(0x803B23E0, MAX_STAR_BITS, Reads::Number, "LumaLink.Patch.MaxStarBits"),
    set(0x803B2550, MAX_CARRIED_STAR_BITS, Reads::Number, "LumaLink.Patch.MaxCarriedStarBits"),
    set(0x803B2598, MAX_CARRIED_STAR_BITS, Reads::Number, "LumaLink.Patch.MaxCarriedStarBits"),
    set(0x80345860, MAX_COINS, Reads::Number, "LumaLink.Patch.MaxCoins"),
    set(0x80345890, COINS_PER_ONE_UP, Reads::Number, "LumaLink.Patch.CoinsPerOneUp"),
    set(0x803458C8, PURPLE_COINS_FOR_STAR, Reads::Compared, "LumaLink.Patch.PurpleCoins"),
    set(0x8034592C, STAR_BITS_PER_ONE_UP, Reads::Number, "LumaLink.Patch.StarBitsPerOneUp"),
    set(0x802B1CA8, WATER_AIR, Reads::Number, "LumaLink.Patch.WaterAir"),
    set(0x802AE8A8, WATER_AIR, Reads::Number, "LumaLink.Patch.WaterAir"),
];

/// A place in the program a mod set a value of, read from the instruction it wrote.
pub struct SettingFound {
    pub id: u32,
    pub value: u32,
    pub what: &'static str,
}

fn instruction(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn signed_low(word: u32) -> i32 {
    i32::from((word & 0xFFFF) as u16 as i16)
}

/// `rD` of `li rD, n` and n.
fn load_immediate(word: u32) -> Option<(u32, i32)> {
    (word >> 26 == 14 && (word >> 16) & 31 == 0).then(|| ((word >> 21) & 31, signed_low(word)))
}

fn is_branch(word: u32) -> bool {
    word >> 26 == 18 && word & 3 == 0
}

/// The setting a patch's bytes change, whatever value it gives it.
pub fn setting(address: u32, value: &[u8]) -> Option<SettingFound> {
    let found = SETTINGS.iter().find(|s| s.address == (address | 0x8000_0000))?;
    let word = instruction(value, 0)?;
    let read = match found.reads {
        Reads::Number => {
            let (_, number) = load_immediate(word)?;
            u32::try_from(number).ok()?
        }
        Reads::Compared => (word >> 26 == 11).then(|| u32::try_from(signed_low(word)).ok()).flatten()?,
        Reads::Zeroed => (load_immediate(word)?.1 == 0).then_some(1)?,
        Reads::Nothing(given) => (word == 0x6000_0000).then_some(given)?,
        Reads::Skipped => is_branch(word).then_some(1)?,
        Reads::ReturnsZero => (load_immediate(word) == Some((3, 0)) && instruction(value, 4) == Some(0x4E80_0020)).then_some(1)?,
    };
    // The game can't start the player on no health.
    if found.id == PLAYER_HEALTH && read == 0 {
        return None;
    }
    Some(SettingFound { id: found.id, value: read, what: found.what })
}

/// One of the game's objects as another, or gone.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ObjectChangeRow {
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub becomes: Option<String>,
    #[serde(rename = "in", skip_serializing_if = "Option::is_none")]
    pub within: Option<&'static str>,
}

/// What a write over the game's table of objects does, row by row. None: it writes something the table has no row for.
pub fn object_table_write(address: u32, value: &[u8]) -> Option<Vec<ObjectChangeRow>> {
    use crate::object_table::{OBJECT_ROWS, OBJECT_TABLE};
    const ROW: u32 = 12;
    let address = address | 0x8000_0000;
    let end = OBJECT_TABLE + OBJECT_ROWS.len() as u32 * ROW;
    if address < OBJECT_TABLE || address >= end || value.is_empty() || value.len() % 4 != 0 || address % 4 != 0 {
        return None;
    }
    let mut changes = Vec::new();
    let mut at = address;
    let mut bytes = value;
    while !bytes.is_empty() {
        let index = (at - OBJECT_TABLE) / ROW;
        let (object, create, archive) = *OBJECT_ROWS.get(index as usize)?;
        let column = ((at - OBJECT_TABLE) % ROW) / 4;
        let words = ((3 - column) as usize).min(bytes.len() / 4);
        let read = |c: u32| (c >= column && ((c - column) as usize) < words).then(|| instruction(bytes, (c - column) as usize * 4)).flatten();
        let (new_name, new_create, new_archive) = (read(0), read(1), read(2));
        if new_name == Some(0) || new_create == Some(0) {
            changes.push(ObjectChangeRow { object: object.to_string(), becomes: None, within: None });
        } else if new_name.is_some() {
            return None;
        } else if let Some(create_now) = new_create {
            if create_now != create {
                let other = OBJECT_ROWS.iter().find(|(_, c, a)| *c == create_now && new_archive.is_none_or(|n| n == *a))?;
                changes.push(ObjectChangeRow { object: object.to_string(), becomes: Some(other.0.to_string()), within: None });
            }
        } else if new_archive.is_some_and(|n| n != archive) {
            let other = OBJECT_ROWS.iter().find(|(_, c, a)| *c == create && Some(*a) == new_archive)?;
            changes.push(ObjectChangeRow { object: object.to_string(), becomes: Some(other.0.to_string()), within: None });
        }
        bytes = &bytes[words * 4..];
        at += words as u32 * 4;
    }
    Some(changes)
}

/// What a treasure box holds: `li r0, kind` for each kind it reads from its name.
const TREASURE_KINDS: [&str; 8] = ["", "Coin", "YellowChip", "BlueChip", "KinokoOneUp", "KinokoLifeUp", "AirBubble", "PowerStar"];
const TREASURE_SITES: [(u32, &str); 5] = [
    (0x8024_D4EC, "YellowChip"),
    (0x8024_D50C, "BlueChip"),
    (0x8024_D52C, "KinokoOneUp"),
    (0x8024_D54C, "KinokoLifeUp"),
    (0x8024_D56C, "AirBubble"),
];

/// A treasure box told to hold something else.
pub fn treasure_box_write(address: u32, value: &[u8]) -> Option<ObjectChangeRow> {
    let site = TREASURE_SITES.iter().find(|(at, _)| *at == (address | 0x8000_0000))?;
    let (_, kind) = load_immediate(instruction(value, 0)?)?;
    // Up to the kinds a box can hold without a star of its own.
    let held = TREASURE_KINDS.get(usize::try_from(kind).ok()?).filter(|k| !k.is_empty() && **k != "PowerStar")?;
    Some(ObjectChangeRow { object: site.1.to_string(), becomes: Some((*held).to_string()), within: Some("treasure_boxes") })
}

/// The still models things show before they're collected, by their row in the game's table.
const DISPLAY_ROWS: [&str; 15] = [
    "Coin", "Kinopio", "SpinDriver", "SuperSpinDriver", "StarPieceDummy", "Tico", "KeySwitch", "PowerStar", "KinokoOneUp", "Kuribo", "BlueChip", "YellowChip", "StrayTico", "GrandStar", "KinokoLifeUp",
];
const DISPLAY_LOOKUP: u32 = 0x801D_0330;

/// The display model lookup, rewritten so one row reads another: `cmpwi r3, from; bne on; li r29, to; b on`.
pub fn display_model_write(address: u32, value: &[u8]) -> Option<ObjectChangeRow> {
    if address | 0x8000_0000 != DISPLAY_LOOKUP || value.len() != 16 {
        return None;
    }
    let compare = instruction(value, 0)?;
    let (to_register, to) = load_immediate(instruction(value, 8)?)?;
    if compare >> 16 != 0x2C03 || to_register != 29 || instruction(value, 4)? >> 16 != 0x4082 || !is_branch(instruction(value, 12)?) {
        return None;
    }
    let from = DISPLAY_ROWS.get(usize::try_from(signed_low(compare)).ok()?)?;
    let to = DISPLAY_ROWS.get(usize::try_from(to).ok()?)?;
    Some(ObjectChangeRow { object: (*from).to_string(), becomes: Some((*to).to_string()), within: Some("display_models") })
}

/// The time limits a comet patch adds, and the parts of the patches read to find them.
pub type CometLimits = (Vec<CometLimitRow>, Vec<(u32, usize)>);

/// A comet mission's time limit.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CometLimitRow {
    pub galaxy: String,
    pub mission: u32,
    pub seconds: u32,
}

/// Memory the program doesn't use, where mods put their own code and tables.
pub fn is_spare_memory(address: u32) -> bool {
    (0x8000_1000..0x8000_3000).contains(&(address | 0x8000_0000))
}

/// The call in the comets' setup to the lookup of a time limit in a table; a mod sends it through its own code to add tables.
const COMET_LIMIT_CALL: u32 = 0x8029_9B4C;
/// That lookup: r3 the table, r4 its rows.
const COMET_LIMIT_LOOKUP: u32 = 0x8029_9C7C;
const COMET_LIMIT_ROW: usize = 12;

/// `bl target` at `at`.
fn call_target(word: u32, at: u32) -> Option<u32> {
    if word >> 26 != 18 || word & 3 != 1 {
        return None;
    }
    let offset = ((word & 0x03FF_FFFC) as i32) << 6 >> 6;
    Some(at.wrapping_add_signed(offset))
}

/// The code a patch at the comets' call jumps to.
pub fn comet_limit_hook(address: u32, value: &[u8]) -> Option<u32> {
    if address | 0x8000_0000 != COMET_LIMIT_CALL {
        return None;
    }
    call_target(instruction(value, 0)?, COMET_LIMIT_CALL)
}

/// Spare memory as a mod's patches filled it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpareMemory {
    /// Address and bytes of each patch.
    pub parts: Vec<(u32, Vec<u8>)>,
}

impl SpareMemory {
    fn bytes_at(&self, address: u32) -> Option<&[u8]> {
        self.parts.iter().find_map(|(at, bytes)| {
            let offset = address.checked_sub(*at)? as usize;
            // Patches sit end to end: the end of one is the start of the next.
            bytes.get(offset..).filter(|rest| !rest.is_empty())
        })
    }

    fn text_at(&self, address: u32) -> Option<String> {
        let bytes = self.bytes_at(address)?;
        let end = bytes.iter().position(|b| *b == 0)?;
        std::str::from_utf8(&bytes[..end]).ok().map(str::to_string)
    }

    /// The time limits added by code at `target`: every call in it to the game's lookup with a table of the mod's own. What's read of the patches comes back so the caller knows what's left over.
    pub fn comet_limits(&self, target: u32) -> Option<CometLimits> {
        let code = self.bytes_at(target)?;
        let mut rows = Vec::new();
        let mut used = Vec::new();
        let (mut high, mut table, mut count) = (None, None, None);
        let mut length = 0usize;
        for at in (0..code.len().saturating_sub(3)).step_by(4) {
            let word = instruction(code, at)?;
            let here = target + at as u32;
            length = at + 4;
            if word >> 26 == 15 && (word >> 21) & 31 == 3 && (word >> 16) & 31 == 0 {
                high = Some(word << 16);
            } else if word >> 26 == 14 && (word >> 21) & 31 == 3 && (word >> 16) & 31 == 3 {
                table = high.map(|h: u32| h.wrapping_add_signed(signed_low(word)));
            } else if let Some((4, number)) = load_immediate(word) {
                count = usize::try_from(number).ok();
            } else if call_target(word, here) == Some(COMET_LIMIT_LOOKUP) {
                if let (Some(table), Some(count)) = (table, count) {
                    let bytes = self.bytes_at(table)?;
                    for row in 0..count {
                        let at = row * COMET_LIMIT_ROW;
                        let galaxy = self.text_at(instruction(bytes, at)?)?;
                        used.push((instruction(bytes, at)?, galaxy.len() + 1));
                        rows.push(CometLimitRow { galaxy, mission: instruction(bytes, at + 4)?, seconds: instruction(bytes, at + 8)? });
                    }
                    used.push((table, (count + 1) * COMET_LIMIT_ROW));
                }
            } else if is_branch(word) && at != 0 {
                break;
            }
        }
        used.push((target, length));
        (!rows.is_empty()).then_some((rows, used))
    }
}

/// The native version of a patch's bytes, if there is one. Nebula's switches don't depend on the version of the game; the
/// table has the addresses mods use.
pub fn find(address: u32, value: &[u8]) -> Option<&'static KnownPatch> {
    KNOWN.iter().find(|k| k.address == (address | 0x8000_0000) && crate::riivolution::hex_bytes(k.value) == value)
}

/// Whether a place in the program is a table of pointers, whose values also differ between versions.
pub fn is_table(address: u32) -> bool {
    let address = address | 0x8000_0000;
    (crate::object_table::OBJECT_TABLE..crate::object_table::OBJECT_TABLE + crate::object_table::OBJECT_ROWS.len() as u32 * 12).contains(&address)
        || COMET_TABLES.iter().any(|(at, _)| (*at..*at + 0x400).contains(&address))
        || (EVENT_FLAG_TABLE..EVENT_FLAG_TABLE + 0x8000).contains(&address)
}

/// How a recognised patch's number becomes its tunable's value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Carry {
    /// The number as it is.
    Number,
    /// The number as a signed one (4294966297 is -999).
    Signed,
    /// The number's bits as a float (0xC4BB8000 is -1500).
    FloatBits,
    /// Any number turns the tunable on.
    On,
    /// Any number turns the tunable off: the tunable's sense is the opposite of the old switch's.
    Off,
}

/// The game's tunable each switch became (Nebula's `luma.json` "tunables"): the switch, its old name, the tunable's key
/// ("<file stem>.<name>"), how its value carries over.
const TUNABLES: &[(u32, &str, &str, Carry)] = &[
    (FLIP_PANEL_GROUP, "flip_panel_group", "FlipPanel.cPanelGroupSize", Carry::Number),
    (FLIP_PANEL_OBSERVER_GROUP, "flip_panel_observer_group", "FlipPanel.cObserverGroupSize", Carry::Number),
    (WATER_BAZOOKA_SHOT_FLAG, "water_bazooka_shot_flag", "WaterBazooka.cShotInvalidSpinKill", Carry::On),
    (KAMECK_BARRIER_OFFSET_Y, "kamek_barrier_offset_y", "BossKameckBarrier.cOffsetY", Carry::FloatBits),
    (SKELETAL_FISH_BABY_NO_BUBBLE, "skeletal_fish_baby_no_bubble", "SkeletalFishBaby.cBreakAirBubble", Carry::Off),
    (DODORYU_SHELL_ONLY, "dodoryu_shell_only", "GameRules.cDodoryuShellOnly", Carry::On),
    (MISS_STAR_PIECES, "miss_star_pieces", "SequenceUtil.cMissStarPieceNum", Carry::Signed),
    (NO_HIDDEN_STAR_TABLE, "no_hidden_star_table", "SceneUtil.cHiddenStarTable", Carry::Off),
    (NO_SUDDEN_DEATH_DODORYU, "no_sudden_death_dodoryu", "SceneUtil.cSuddenDeathDodoryu", Carry::Off),
    (BEGOMAN_FIRE_PROOF, "begoman_fire_proof", "GameRules.cBegomanFireProof", Carry::On),
    (MESSENGER_ALWAYS_EX, "messenger_always_ex", "KinopioAstro.cMessangerAlwaysEx", Carry::On),
    (MESSENGER_EX_ONE_UPS, "messenger_ex_one_ups", "KinopioAstro.sMessangerOneUpExNum", Carry::Number),
    (PLAYER_HEALTH, "player_health", "GameRules.cPlayerHealth", Carry::Number),
    (PLAYER_HEALTH_FIXED, "player_health_fixed", "MarioActor.cDarkCometOneHealth", Carry::Off),
    (HIT_TAKES_ALL, "hit_takes_all", "MarioActor.cHitTakesAll", Carry::On),
    (LIFE_UP_HEALTH, "life_up_health", "GameRules.cLifeUpHealth", Carry::Number),
    (NO_LOW_HEALTH_WARNING, "no_low_health_warning", "MarioEffect.cLowHealthWarning", Carry::Off),
    (NO_DARK_COMET_RULES, "no_dark_comet_rules", "EventUtil.cDarkCometRules", Carry::Off),
    (MAX_LIVES, "max_lives", "GameDataPlayerStatus.cMaxPlayerLeft", Carry::Number),
    (MAX_STAR_BITS, "max_star_bits", "GameRules.cMaxStarBits", Carry::Number),
    (MAX_CARRIED_STAR_BITS, "max_carried_star_bits", "GameDataTemporaryInGalaxy.cMaxCarriedStarPieceNum", Carry::Number),
    (MAX_COINS, "max_coins", "GameRules.cMaxCoins", Carry::Number),
    (COINS_PER_ONE_UP, "coins_per_one_up", "ScenePlayingResult.cCoinStepNum", Carry::Number),
    (STAR_BITS_PER_ONE_UP, "star_bits_per_one_up", "ScenePlayingResult.cStarPieceStepNum", Carry::Number),
    (STARTING_LIVES, "starting_lives", "GameDataPlayerStatus.cStartingPlayerLeft", Carry::Number),
    (PURPLE_COINS_FOR_STAR, "purple_coins_for_star", "ScenePlayingResult.cPurpleCoinPowerStarAppearNum", Carry::Number),
    (WATER_AIR, "water_air", "MarioActor.cWaterAir", Carry::Number),
];

/// Each tunable's number in the game's code (luma_numbers.json: a file's constant, signature "" and place 0), which a
/// converted mod's code sets (LumaCore set_number).
const NUMBER_NAMES: &[(&str, &str)] = &[
    ("FlipPanel.cPanelGroupSize", "Game/MapObj/FlipPanel.cpp#cPanelGroupSize"),
    ("FlipPanel.cObserverGroupSize", "Game/MapObj/FlipPanel.cpp#cObserverGroupSize"),
    ("WaterBazooka.cShotInvalidSpinKill", "Game/Enemy/WaterBazooka.cpp#cShotInvalidSpinKill"),
    ("BossKameckBarrier.cOffsetY", "Game/Boss/BossKameckBarrier.cpp#cOffsetY"),
    ("SkeletalFishBaby.cBreakAirBubble", "Game/Boss/SkeletalFishBaby.cpp#cBreakAirBubble"),
    ("GameRules.cDodoryuShellOnly", "Game/System/GameRules.cpp#cDodoryuShellOnly"),
    ("SequenceUtil.cMissStarPieceNum", "Game/Util/SequenceUtil.cpp#cMissStarPieceNum"),
    ("SceneUtil.cHiddenStarTable", "Game/Util/SceneUtil.cpp#cHiddenStarTable"),
    ("SceneUtil.cSuddenDeathDodoryu", "Game/Util/SceneUtil.cpp#cSuddenDeathDodoryu"),
    ("GameRules.cBegomanFireProof", "Game/System/GameRules.cpp#cBegomanFireProof"),
    ("KinopioAstro.cMessangerAlwaysEx", "Game/NPC/KinopioAstro.cpp#cMessangerAlwaysEx"),
    ("KinopioAstro.sMessangerOneUpExNum", "Game/NPC/KinopioAstro.cpp#sMessangerOneUpExNum"),
    ("GameRules.cPlayerHealth", "Game/System/GameRules.cpp#cPlayerHealth"),
    ("MarioActor.cDarkCometOneHealth", "Game/Player/MarioActor.cpp#cDarkCometOneHealth"),
    ("MarioActor.cHitTakesAll", "Game/Player/MarioActor.cpp#cHitTakesAll"),
    ("GameRules.cLifeUpHealth", "Game/System/GameRules.cpp#cLifeUpHealth"),
    ("MarioEffect.cLowHealthWarning", "Game/Player/MarioEffect.cpp#cLowHealthWarning"),
    ("EventUtil.cDarkCometRules", "Game/Util/EventUtil.cpp#cDarkCometRules"),
    ("GameDataPlayerStatus.cMaxPlayerLeft", "Game/System/GameDataPlayerStatus.cpp#cMaxPlayerLeft"),
    ("GameRules.cMaxStarBits", "Game/System/GameRules.cpp#cMaxStarBits"),
    ("GameDataTemporaryInGalaxy.cMaxCarriedStarPieceNum", "Game/System/GameDataTemporaryInGalaxy.cpp#cMaxCarriedStarPieceNum"),
    ("GameRules.cMaxCoins", "Game/System/GameRules.cpp#cMaxCoins"),
    ("ScenePlayingResult.cCoinStepNum", "Game/Scene/ScenePlayingResult.cpp#cCoinStepNum"),
    ("ScenePlayingResult.cStarPieceStepNum", "Game/Scene/ScenePlayingResult.cpp#cStarPieceStepNum"),
    ("GameDataPlayerStatus.cStartingPlayerLeft", "Game/System/GameDataPlayerStatus.cpp#cStartingPlayerLeft"),
    ("ScenePlayingResult.cPurpleCoinPowerStarAppearNum", "Game/Scene/ScenePlayingResult.cpp#cPurpleCoinPowerStarAppearNum"),
    ("MarioActor.cWaterAir", "Game/Player/MarioActor.cpp#cWaterAir"),
];

/// A recognised patch as the game's number a mod's code sets: its name and value (a bool 1 or 0). None: Nebula covers
/// it by itself.
pub fn number(switch: u32, number: u32) -> Option<(&'static str, f64)> {
    let (key, value) = tunable(switch, number)?;
    let name = NUMBER_NAMES.iter().find(|n| n.0 == key)?.1;
    let value = match value {
        serde_json::Value::Bool(b) => f64::from(u8::from(b)),
        v => v.as_f64()?,
    };
    Some((name, value))
}

/// Every tunable key a recognised patch can become.
pub fn tunable_keys() -> impl Iterator<Item = &'static str> {
    TUNABLES.iter().map(|t| t.2)
}

/// A recognised patch as a `luma.json` "tunables" row: the key and its value. None: Nebula covers it by itself.
pub fn tunable(switch: u32, number: u32) -> Option<(&'static str, serde_json::Value)> {
    let &(_, _, key, carry) = TUNABLES.iter().find(|t| t.0 == switch)?;
    Some((
        key,
        match carry {
            Carry::Number => number.into(),
            Carry::Signed => (number as i32).into(),
            Carry::FloatBits => f64::from(f32::from_bits(number)).into(),
            Carry::On => (number != 0).into(),
            Carry::Off => (number == 0).into(),
        },
    ))
}

/// The tunable that took the place of a switch an older `luma.json` names, and whether its sense is the opposite.
pub fn tunable_for_switch(name: &str) -> Option<(&'static str, bool)> {
    TUNABLES.iter().find(|t| t.1 == name).map(|t| (t.2, t.3 == Carry::Off))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tunable_has_its_number() {
        for (switch, _, key, _) in TUNABLES {
            assert!(NUMBER_NAMES.iter().any(|n| n.0 == *key), "{key} has no number name");
            assert!(number(*switch, 1).is_some(), "{key}");
        }
        assert_eq!(number(PLAYER_HEALTH, 1), Some(("Game/System/GameRules.cpp#cPlayerHealth", 1.0)));
        assert_eq!(number(HIT_TAKES_ALL, 1), Some(("Game/Player/MarioActor.cpp#cHitTakesAll", 1.0)));
        assert_eq!(number(NO_DARK_COMET_RULES, 1), Some(("Game/Util/EventUtil.cpp#cDarkCometRules", 0.0)));
    }

    #[test]
    fn a_patch_is_known_by_region_address_and_bytes() {
        assert_eq!(find(0x8039AA2C, &[0x60, 0, 0, 0]).unwrap().id, 0);
        assert_eq!(find(0x0007679C, &[0x60, 0, 0, 0]).unwrap().id, SKELETAL_FISH_BABY_NO_BUBBLE);
        assert!(find(0x8007679C, &[0x60, 0, 0, 1]).is_none());
    }

    #[test]
    fn each_change_becomes_its_tunable_with_the_value_carried_over() {
        assert_eq!(tunable(PLAYER_HEALTH, 2), Some(("GameRules.cPlayerHealth", serde_json::json!(2))));
        assert_eq!(tunable(MISS_STAR_PIECES, 4294966297), Some(("SequenceUtil.cMissStarPieceNum", serde_json::json!(-999))));
        assert_eq!(tunable(KAMECK_BARRIER_OFFSET_Y, 0xC4BB_8000), Some(("BossKameckBarrier.cOffsetY", serde_json::json!(-1500.0))));
        assert_eq!(tunable(SKELETAL_FISH_BABY_NO_BUBBLE, 1), Some(("SkeletalFishBaby.cBreakAirBubble", serde_json::json!(false))));
        assert_eq!(tunable(DODORYU_SHELL_ONLY, 1), Some(("GameRules.cDodoryuShellOnly", serde_json::json!(true))));
        assert_eq!(tunable(NATIVE_ALREADY, 1), None);
        assert!((1..=WATER_AIR).all(|id| tunable(id, 1).is_some()), "every change has its tunable");
        assert_eq!(tunable_for_switch("no_low_health_warning"), Some(("MarioEffect.cLowHealthWarning", true)));
        assert_eq!(tunable_for_switch("max_coins"), Some(("GameRules.cMaxCoins", false)));
        assert_eq!(tunable_for_switch("nope"), None);
    }

    #[test]
    fn a_setting_takes_any_value_a_mod_chooses() {
        let health = setting(0x802AE8A4, &0x3BC0_0002u32.to_be_bytes()).unwrap();
        assert_eq!((health.id, health.value), (PLAYER_HEALTH, 2));
        assert!(setting(0x802AE8A4, &0x3BC0_0000u32.to_be_bytes()).is_none());
        assert!(setting(0x802AE8A4, &0x6000_0000u32.to_be_bytes()).is_none());
        assert_eq!(setting(0x80275458, &0x3880_002Au32.to_be_bytes()).unwrap().value, 42);
        assert_eq!(setting(0x802750F0, &0x2C1F_0007u32.to_be_bytes()).unwrap().value, 7);
        assert_eq!(setting(0x802B07A0, &0x6000_0000u32.to_be_bytes()).unwrap().value, 0);
        assert!(setting(0x802DDF50, &0x4800_0060u32.to_be_bytes()).is_some());
        assert!(setting(0x803CDB60, &[0x38, 0x60, 0, 0, 0x4E, 0x80, 0, 0x20, 0, 0, 0, 0]).is_some());
        assert!(setting(0x803CDB60, &[0x38, 0x60, 0, 1, 0x4E, 0x80, 0, 0x20]).is_none());
    }

    #[test]
    fn writes_over_the_object_table_say_what_each_row_becomes() {
        use crate::object_table::{OBJECT_ROWS, OBJECT_TABLE};
        let row = |name: &str| OBJECT_ROWS.iter().position(|r| r.0 == name).unwrap() as u32;
        let gone = object_table_write(OBJECT_TABLE + row("RestartCube") * 12, &[0; 12]).unwrap();
        assert_eq!(gone[0].object, "RestartCube");
        assert_eq!(gone[0].becomes, None);
        let up = OBJECT_ROWS[row("BenefitItemOneUp") as usize];
        let mut value = up.1.to_be_bytes().to_vec();
        value.extend_from_slice(&up.2.to_be_bytes());
        let swapped = object_table_write(OBJECT_TABLE + row("BenefitItemLifeUp") * 12 + 4, &value).unwrap();
        assert_eq!(swapped[0].becomes.as_deref(), Some("BenefitItemOneUp"));
        // A row's creator pointing nowhere in the table isn't something to guess at.
        assert!(object_table_write(OBJECT_TABLE + 4, &0x8000_1234u32.to_be_bytes()).is_none());
    }

    #[test]
    fn a_box_and_a_display_model_follow_the_numbers_written() {
        let held = treasure_box_write(0x8024_D54C, &0x3800_0004u32.to_be_bytes()).unwrap();
        assert_eq!((held.object.as_str(), held.becomes.as_deref()), ("KinokoLifeUp", Some("KinokoOneUp")));
        assert_eq!(treasure_box_write(0x8024_D54C, &0x3800_0002u32.to_be_bytes()).unwrap().becomes.as_deref(), Some("YellowChip"));
        let mut lookup = Vec::new();
        for word in [0x2C03_000Eu32, 0x4082_0018, 0x3BA0_0008, 0x4800_0010] {
            lookup.extend_from_slice(&word.to_be_bytes());
        }
        let shown = display_model_write(0x801D_0330, &lookup).unwrap();
        assert_eq!((shown.object.as_str(), shown.becomes.as_deref()), ("KinokoLifeUp", Some("KinokoOneUp")));
    }

    #[test]
    fn code_in_spare_memory_is_read_for_its_table() {
        let mut table = b"AGalaxy\0".to_vec();
        table.resize(16, 0);
        for word in [0x8000_1000u32, 4, 90, 0, 0, 0] {
            table.extend_from_slice(&word.to_be_bytes());
        }
        // Right after the table, as mods lay them out.
        let target = 0x8000_1000u32 + table.len() as u32;
        let lis = 0x3C60_8000u32;
        let addi = 0x3863_1010u32;
        let li = 0x3880_0001u32;
        let call = |at: u32| 0x4800_0001u32 | ((COMET_LIMIT_LOOKUP.wrapping_sub(at)) & 0x03FF_FFFC);
        let mut code = Vec::new();
        for (i, word) in [lis, addi, li].iter().enumerate() {
            let _ = i;
            code.extend_from_slice(&word.to_be_bytes());
        }
        code.extend_from_slice(&call(target + 12).to_be_bytes());
        code.extend_from_slice(&0x4800_0000u32.to_be_bytes());
        let spare = SpareMemory { parts: vec![(0x8000_1000, table), (target, code)] };
        let (rows, _) = spare.comet_limits(target).unwrap();
        assert_eq!(rows, vec![CometLimitRow { galaxy: "AGalaxy".to_string(), mission: 4, seconds: 90 }]);
    }

    #[test]
    fn every_row_has_bytes() {
        assert!(KNOWN.iter().all(|k| !crate::riivolution::hex_bytes(k.value).is_empty()));
    }
}

/// Tables a mod can patch in the Wii program, which Nebula keeps as data. Their places, and the names their rows point at.
pub const GALAXY_TABLE: u32 = 0x8053_C800;
pub const EVENT_FLAG_TABLE: u32 = 0x8053_BC94;
const COMET_TABLES: [(u32, u32); 4] = [(0x8053_AF78, 2), (0x8053_B000, 3), (0x8053_B088, 4), (0x8053_B150, 5)];
const NAMES: &[(u32, &str)] = &[
    (0x805C9844, "StarDustGalaxy"),
    (0x805C9853, "BattleShipGalaxy"),
    (0x805C9864, "PhantomGalaxy"),
    (0x805C9872, "HeavenlyBeachGalaxy"),
    (0x805C9886, "SandClockGalaxy"),
    (0x805C9896, "CosmosGardenGalaxy"),
    (0x805C98A9, "IceVolcanoGalaxy"),
    (0x805C98BA, "OceanRingGalaxy"),
    (0x805C98CA, "ReverseKingdomGalaxy"),
    (0x805C98DF, "FactoryGalaxy"),
    (0x805C9D97, "HeavenlyBeachGalaxy"),
    (0x805C9E87, "TearDropGalaxy"),
    (0x805CA01E, "HeavensDoorGalaxy"),
    (0x805CA03E, "BeltConveyerExGalaxy"),
    (0x805CA063, "StarDustGalaxy"),
    (0x805CA083, "CocoonExGalaxy"),
    (0x805CA0A7, "PhantomGalaxy"),
    (0x805CA0B5, "TeresaMario2DGalaxy"),
    (0x805CA102, "SandClockGalaxy"),
    (0x805CA112, "SnowCapsuleGalaxy"),
    (0x805CA14E, "OceanRingGalaxy"),
    (0x805CA15E, "TransformationExGalaxy"),
    (0x805CA1B3, "HellProminenceGalaxy"),
    (0x805CA1C8, "FishTunnelGalaxy"),
    (0x805CA60A, "SpecialStarGrand1"),
    (0x805CA61C, "SpecialStarGrand2"),
    (0x805CA62E, "SpecialStarGrand3"),
    (0x805CA640, "SpecialStarGrand4"),
    (0x805CA652, "SpecialStarGrand5"),
    (0x805CA664, "SpecialStarGrand6"),
    (0x805CA747, "ExOpen1CocoonExGalaxy"),
    (0x805CA75D, "ExOpen1TearDropGalaxy"),
    (0x805CA773, "ExOpen1FishTunnelGalaxy"),
    (0x805CA78B, "ExOpenCommonBase"),
    (0x805CA79C, "ExOpenSubTransformationExGalaxy"),
    (0x805CA7BC, "ExOpenSubTeresaMario2DGalaxy"),
    (0x805CA7D9, "ExOpen1TransformationExGalaxy"),
    (0x805CA7F7, "ExOpen1TeresaMario2DGalaxy"),
    (0x805CA812, "ExOpen1SnowCapsuleGalaxy"),
    (0x805CA82B, "ExOpen1BeltConveyerExGalaxy"),
    (0x805CA847, "AppearBeltConveyerExGalaxy"),
    (0x805CA862, "AppearCocoonExGalaxy"),
    (0x805CA877, "AppearTearDropGalaxy"),
    (0x805CA88C, "AppearFishTunnelGalaxy"),
    (0x805CA8A3, "AppearTransformationExGalaxy"),
    (0x805CA8C0, "AppearTeresaMario2DGalaxy"),
    (0x805CA8DA, "AppearSnowCapsuleGalaxy"),
    (0x805CA8F2, "StarPieceBeltConveyerExGalaxy"),
    (0x805CA910, "StarPieceCocoonExGalaxy"),
    (0x805CA928, "StarPieceTearDropGalaxy"),
    (0x805CA940, "StarPieceFishTunnelGalaxy"),
    (0x805CA95A, "StarPieceTransformationExGalaxy"),
    (0x805CA97A, "StarPieceTeresaMario2DGalaxy"),
    (0x805CA997, "StarPieceSnowCapsuleGalaxy"),
];

fn name_at(pointer: u32) -> Option<String> {
    NAMES.iter().find(|(at, _)| *at == pointer).map(|(_, name)| (*name).to_string())
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A row of the game's progression.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EventFlagRow {
    pub name: String,
    pub kind: u8,
    pub a: u8,
    pub b: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second: Option<String>,
}

/// One comet of a dome's schedule.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CometRow {
    pub galaxy: String,
    pub coins: u32,
}

/// The dome a patched comet table belongs to, and its rows: a name and a kind, until a row of nothing.
pub fn comet_table(address: u32, value: &[u8]) -> Option<(u32, Vec<CometRow>)> {
    let dome = COMET_TABLES.iter().find(|(at, _)| *at == (address | 0x8000_0000))?.1;
    let mut rows = Vec::new();
    for at in (0..value.len()).step_by(8) {
        let pointer = word(value, at)?;
        if pointer == 0 {
            return Some((dome, rows));
        }
        rows.push(CometRow { galaxy: name_at(pointer)?, coins: word(value, at + 4)? });
    }
    None
}

/// The game's table of what opens what, patched whole: twenty bytes a row.
pub fn event_flag_table(bytes: &[u8]) -> Option<Vec<EventFlagRow>> {
    if bytes.is_empty() || bytes.len() % 20 != 0 {
        return None;
    }
    let optional = |pointer: u32| if pointer == 0 { Some(None) } else { name_at(pointer).map(Some) };
    bytes
        .chunks_exact(20)
        .map(|row| {
            Some(EventFlagRow {
                name: name_at(word(row, 0)?)?,
                kind: row[4],
                a: row[6],
                b: row[7],
                first: optional(word(row, 12)?)?,
                second: optional(word(row, 16)?)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod table_tests {
    use super::*;

    #[test]
    fn a_comet_table_reads_by_name_and_a_strange_one_does_not() {
        let mut value = Vec::new();
        value.extend_from_slice(&0x805C_9853u32.to_be_bytes());
        value.extend_from_slice(&1u32.to_be_bytes());
        value.extend_from_slice(&[0; 8]);
        let (dome, rows) = comet_table(0x8053_AF78, &value).unwrap();
        assert_eq!(dome, 2);
        assert_eq!(rows, vec![CometRow { galaxy: "BattleShipGalaxy".to_string(), coins: 1 }]);
        assert!(comet_table(0x8053_0000, &value).is_none());
        value[0] = 0x12;
        assert!(comet_table(0x8053_AF78, &value).is_none());
    }

    #[test]
    fn a_flag_row_keeps_its_kind_and_numbers() {
        let mut row = Vec::new();
        row.extend_from_slice(&0x805C_A79Cu32.to_be_bytes());
        row.extend_from_slice(&[3, 1, 2, 40]);
        row.extend_from_slice(&[0; 4]);
        row.extend_from_slice(&0x805C_9853u32.to_be_bytes());
        row.extend_from_slice(&[0; 4]);
        let rows = event_flag_table(&row).unwrap();
        assert_eq!(rows[0].kind, 3);
        assert_eq!((rows[0].a, rows[0].b), (2, 40));
        assert_eq!(rows[0].first.as_deref(), Some("BattleShipGalaxy"));
        assert_eq!(rows[0].second, None);
        assert!(event_flag_table(&row[..19]).is_none());
    }
}
