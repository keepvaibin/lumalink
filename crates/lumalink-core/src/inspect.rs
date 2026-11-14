//! Looking inside the game and inside files: what a modder needs to know the names of.

use crate::say;
use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// A column's hash, as the game's tables have it.
fn column_hash(name: &str) -> u32 {
    nebula_import::cook::table::column_hash(name)
}

/// The names of columns: tables keep only their hashes, and the names come from the game's self-description
/// (self-description: `tables[].columns[].name`, made by the game's build from its source). A hash
/// not among them comes out as `0x` and its hash, which reads back the same.
#[derive(Clone, Debug, Default)]
pub struct ColumnNames(BTreeMap<u32, String>);

impl ColumnNames {
    /// No names: every column by its hash.
    pub fn none() -> Self {
        Self::default()
    }

    /// The column names a self-description file lists.
    pub fn from_self_description(bytes: &[u8], shown_as: &Path) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| anyhow::anyhow!(say!("LumaLink.SelfNotJson", path = shown_as.display(), line = e.line(), column = e.column())))?;
        let mut names = BTreeMap::new();
        for table in value.get("tables").and_then(Value::as_array).into_iter().flatten() {
            for column in table.get("columns").and_then(Value::as_array).into_iter().flatten() {
                if let Some(name) = column.get("name").and_then(Value::as_str) {
                    names.insert(column_hash(name), name.to_string());
                }
            }
        }
        Ok(Self(names))
    }

    pub(crate) fn name(&self, hash: u32) -> String {
        self.0.get(&hash).cloned().unwrap_or_else(|| format!("0x{hash:08x}"))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A table of the game, disc or cooked, as the JSON a mod writes.
pub fn table_to_json(bytes: &[u8], names: &ColumnNames) -> Result<Value> {
    if bytes.starts_with(nebula_import::cook::table::MAGIC) {
        return cooked_table_to_json(bytes, names);
    }
    if bytes.len() < 16 {
        bail!(say!("LumaLink.Table.TooShort"));
    }
    let little = {
        let fields = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
        let data = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
        data == 16 + fields * 12
    };
    let word = |at: usize| -> Result<u32> {
        let b: [u8; 4] = bytes.get(at..at + 4).with_context(|| say!("LumaLink.Table.PastEnd"))?.try_into()?;
        Ok(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let half = |at: usize| -> Result<u16> {
        let b: [u8; 2] = bytes.get(at..at + 2).with_context(|| say!("LumaLink.Table.PastEnd"))?.try_into()?;
        Ok(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let (rows, fields, data, size) = (word(0)? as usize, word(4)? as usize, word(8)? as usize, word(12)? as usize);
    if data != 16 + fields * 12 {
        bail!(say!("LumaLink.Table.NotTable"));
    }
    struct Column {
        name: String,
        mask: u32,
        offset: usize,
        shift: u32,
        kind: u8,
    }
    let mut columns = Vec::with_capacity(fields);
    for f in 0..fields {
        let at = 16 + 12 * f;
        let hash = word(at)?;
        columns.push(Column {
            name: names.name(hash),
            mask: word(at + 4)?,
            offset: half(at + 8)? as usize,
            shift: u32::from(*bytes.get(at + 10).with_context(|| say!("LumaLink.Table.PastEnd"))?),
            kind: *bytes.get(at + 11).with_context(|| say!("LumaLink.Table.PastEnd"))?,
        });
    }
    let pool = data + rows * size;
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let base = data + r * size;
        let mut row = Map::new();
        for column in &columns {
            let at = base + column.offset;
            let value = match column.kind {
                6 => {
                    let start = pool + word(at)? as usize;
                    let end = bytes[start..].iter().position(|b| *b == 0).map_or(bytes.len(), |e| start + e);
                    Value::String(text_of(&bytes[start..end]))
                }
                2 => {
                    let raw = word(at)?;
                    json!(f64::from(f32::from_bits(raw)))
                }
                0 | 3 => {
                    let raw = (word(at)? & column.mask) >> column.shift;
                    if column.mask == u32::MAX {
                        json!(raw as i32)
                    } else {
                        json!(raw)
                    }
                }
                4 => json!(i64::from(half(at)?) & i64::from(column.mask)),
                5 => json!(i64::from(*bytes.get(at).with_context(|| say!("LumaLink.Table.PastEnd"))?)),
                other => bail!(say!("LumaLink.Table.ColumnKind", kind = other)),
            };
            row.insert(column.name.clone(), value);
        }
        out.push(Value::Object(row));
    }
    let kinds: Map<String, Value> = columns
        .iter()
        .filter(|c| c.kind == 2)
        .map(|c| (c.name.clone(), Value::String("float".to_string())))
        .collect();
    Ok(if kinds.is_empty() { json!({ "rows": out }) } else { json!({ "columns": kinds, "rows": out }) })
}

/// A cooked table (the imported game's, or Nebula's own) as the JSON a mod writes: an int as the game reads it (signed,
/// at its width), a float marked in `columns`, words as UTF-8.
fn cooked_table_to_json(bytes: &[u8], names: &ColumnNames) -> Result<Value> {
    use nebula_import::cook::table::{Cell, Kind, TableIr};
    let table = TableIr::from_ntbl(bytes)?;
    let named: Vec<String> = table.columns.iter().map(|c| names.name(c.hash)).collect();
    let mut out = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        let mut fields = Map::new();
        for ((column, name), cell) in table.columns.iter().zip(&named).zip(row) {
            let value = match (column.kind, cell) {
                // Words are UTF-8 but for a few the disc spells with bytes no encoding reads (two cameras' ids end in a
                // counter byte); those keep every byte, so two rows never read as one (docs/modding/format/table-keys.md, section 5).
                (Kind::Text, Cell::Text(word)) => match std::str::from_utf8(word) {
                    Ok(text) => Value::String(text.to_string()),
                    Err(_) => json!({ "bytes": word.iter().map(|b| format!("{b:02x}")).collect::<String>() }),
                },
                (Kind::Float, Cell::Bits(bits)) => json!(f64::from(f32::from_bits(*bits))),
                (Kind::Int, Cell::Bits(bits)) => match column.width {
                    8 => json!(i64::from(*bits as u8 as i8)),
                    16 => json!(i64::from(*bits as u16 as i16)),
                    _ => json!(i64::from(*bits as i32)),
                },
                (_, Cell::Bits(bits)) => json!(*bits),
                (_, Cell::Text(_)) => bail!(say!("LumaLink.Table.WordForNumber", name = name)),
            };
            fields.insert(name.clone(), value);
        }
        out.push(Value::Object(fields));
    }
    let kinds: Map<String, Value> =
        table.columns.iter().zip(&named).filter(|(c, _)| c.kind == Kind::Float).map(|(_, n)| (n.clone(), Value::String("float".to_string()))).collect();
    Ok(if kinds.is_empty() { json!({ "rows": out }) } else { json!({ "columns": kinds, "rows": out }) })
}

/// One table of an imported game: its path, how many rows, and its columns (name, and int, float, field or string).
pub struct TableListing {
    pub path: String,
    pub rows: usize,
    pub columns: Vec<(String, &'static str)>,
}

/// Every table of an imported game, in the store's order: what a mod's `merge/` file can add rows to, remove rows from
/// or change. `filter` keeps those whose path or a column name holds it.
pub fn list_tables(store: &Path, filter: Option<&str>, names: &ColumnNames) -> Result<Vec<TableListing>> {
    use nebula_import::cook::table::{Kind, TableIr, MAGIC};
    use std::io::{Read, Seek, SeekFrom};
    let wanted = filter.map(str::to_ascii_lowercase);
    let mut pak = fs::File::open(store).with_context(|| say!("LumaLink.CannotRead", path = store.display()))?;
    let store_length = pak.metadata()?.len();
    let mut found = Vec::new();
    for (path, position, size) in crate::game_store::GameStore::files(store)? {
        if size < 32 {
            continue;
        }
        let mut magic = [0u8; 4];
        pak.seek(SeekFrom::Start(position))?;
        pak.read_exact(&mut magic)?;
        if &magic != MAGIC {
            continue;
        }
        if position.checked_add(size).is_none_or(|end| end > store_length) {
            bail!(say!("LumaLink.Store.PastEnd", path = path));
        }
        let mut bytes = vec![0u8; size as usize];
        pak.seek(SeekFrom::Start(position))?;
        pak.read_exact(&mut bytes)?;
        let table = TableIr::from_ntbl(&bytes).with_context(|| say!("LumaLink.Table.In", path = path))?;
        let columns: Vec<(String, &'static str)> = table
            .columns
            .iter()
            .map(|c| {
                let name = names.name(c.hash);
                let kind = match c.kind {
                    Kind::Int => "int",
                    Kind::Field => "field",
                    Kind::Float => "float",
                    Kind::Text => "string",
                };
                (name, kind)
            })
            .collect();
        if let Some(wanted) = &wanted {
            if !path.to_ascii_lowercase().contains(wanted) && !columns.iter().any(|(n, _)| n.to_ascii_lowercase().contains(wanted)) {
                continue;
            }
        }
        found.push(TableListing { path, rows: table.rows.len(), columns });
    }
    Ok(found)
}

/// One model of an imported game: its path, its joints, materials and textures, and the animations beside it in its
/// archive (a model made like it, or put in its place, plays those by name).
pub struct ModelListing {
    pub path: String,
    pub names: nebula_import::cook::model_write::ModelNames,
    pub animations: Vec<String>,
}

/// The endings of a model's animations in its archive: skeleton (bck, bca), texture moves (btk), colours (brk, bpk),
/// texture frames (btp), shown parts (bva), cluster and shape moves (blk, bla, bxk, bxa).
const ANIMATION_ENDINGS: [&str; 11] = ["bck", "bca", "btk", "brk", "bpk", "btp", "bva", "blk", "bla", "bxk", "bxa"];

/// Every model of an imported game whose path holds `filter`, read from the cooked store.
pub fn list_models(store: &Path, filter: Option<&str>) -> Result<Vec<ModelListing>> {
    use std::io::{Read, Seek, SeekFrom};
    let wanted = filter.map(str::to_ascii_lowercase);
    let files = crate::game_store::GameStore::disc_files(store)?;
    let ending = |path: &str| path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let folder = |path: &str| path.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_ascii_lowercase());
    let mut animations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, _, _) in &files {
        if ANIMATION_ENDINGS.contains(&ending(path).as_str()) {
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            animations.entry(folder(path)).or_default().push(name);
        }
    }
    let mut pak = fs::File::open(store).with_context(|| say!("LumaLink.CannotRead", path = store.display()))?;
    let mut found = Vec::new();
    for (path, position, size) in &files {
        if !matches!(ending(path).as_str(), "bdl" | "bmd") || wanted.as_ref().is_some_and(|w| !path.to_ascii_lowercase().contains(w)) {
            continue;
        }
        let mut bytes = vec![0u8; usize::try_from(*size)?];
        pak.seek(SeekFrom::Start(*position))?;
        pak.read_exact(&mut bytes)?;
        let names = nebula_import::cook::model_write::names(&bytes).with_context(|| say!("LumaLink.Model.In", path = path))?;
        let mut beside = animations.get(&folder(path)).cloned().unwrap_or_default();
        beside.sort_by_key(|a| a.to_ascii_lowercase());
        found.push(ModelListing { path: path.clone(), names, animations: beside });
    }
    Ok(found)
}

/// A language's messages of an imported game (`UsEnglish`, `EuFrench`...): (id, text with the game's tags in brackets,
/// whether that text reads back as the game's own units).
pub fn list_messages(store: &Path, language: &str) -> Result<Vec<(String, String, bool)>> {
    use nebula_import::cook::store as game_store;
    let folder = format!("{language}/MessageData/Message");
    let files = game_store::read_folder(store, &folder).with_context(|| say!("LumaLink.Store.NoFolder", folder = folder))?;
    let find = |name: &str| files.iter().find(|(path, _)| path.eq_ignore_ascii_case(name)).map(|(_, bytes)| bytes.as_slice());
    let (Some(message_file), Some(id_table)) = (find("message.bmg"), find("messageid.tbl")) else {
        bail!(say!("LumaLink.Store.NoMessages", folder = folder));
    };
    Ok(crate::messages::read(message_file, id_table)?
        .into_iter()
        .map(|(id, units)| {
            let text = crate::messages::decode(&units);
            let same = crate::messages::encode(&text) == units;
            (id, text, same)
        })
        .collect())
}

/// A table's words: as the game's own tables keep Japanese.
fn text_of(bytes: &[u8]) -> String {
    encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
}

/// Every file of an imported game under a folder, by its place.
pub fn list_files(store: &Path, folder: &str) -> Result<Vec<(String, u64)>> {
    use nebula_import::cook::store as game_store;
    let found = game_store::read_folder(store, folder.trim_matches('/'))?;
    Ok(found.into_iter().map(|(path, bytes)| (path, bytes.len() as u64)).collect())
}

/// One file or a whole folder of an imported game into `output`: tables as JSON too, so they can be read and changed.
pub fn extract(store: &Path, path: &str, output: &Path, names: &ColumnNames) -> Result<usize> {
    use nebula_import::cook::store as game_store;
    let path = path.trim_matches('/');
    let items = match game_store::read_folder(store, path) {
        Ok(items) if !items.is_empty() => items.into_iter().map(|(p, b)| (format!("{path}/{p}"), b)).collect(),
        _ => vec![(path.to_string(), game_store::read_file(store, path)?)],
    };
    let mut written = 0;
    for (place, bytes) in items {
        let target = output.join(&place);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| say!("LumaLink.CannotMake", path = parent.display()))?;
        }
        // A table or a particle file comes with its JSON beside it; nothing already there is written over.
        let readable = table_to_json(&bytes, names).ok().or_else(|| particles_to_json(store, &place, &bytes).ok());
        let mut json = target.as_os_str().to_owned();
        json.push(".json");
        let json = std::path::PathBuf::from(json);
        for out in std::iter::once(&target).chain(readable.as_ref().map(|_| &json)) {
            if out.exists() {
                bail!(say!("LumaLink.AlreadyThere", path = out.display()));
            }
        }
        if let Some(value) = readable {
            fs::write(&json, serde_json::to_vec_pretty(&value)?).with_context(|| say!("LumaLink.CannotWrite", path = json.display()))?;
            written += 1;
        }
        fs::write(&target, &bytes).with_context(|| say!("LumaLink.CannotWrite", path = target.display()))?;
        written += 1;
    }
    Ok(written)
}

/// The game's particles (ParticleData/Effect.arc/Particles.jpc) as a mod's `layers/<layer>/assets/effects.json` writes
/// them (docs/modding/content/effects.md, section 7): each under `replace` by its name from ParticleNames beside it, every value and its
/// textures by role. Copied into a mod and changed, the changed particles replace the game's; one copied under
/// `effects` (or named by `like`) is a new effect starting from it.
fn particles_to_json(store: &Path, place: &str, bytes: &[u8]) -> Result<serde_json::Value> {
    use nebula_import::cook::{effects, particle_json, particles, store as game_store};
    let ir = particles::read_nptc(bytes)?;
    let folder = place.rsplit_once('/').map_or("", |(folder, _)| folder);
    let names = particle_json::names_index(&game_store::read_file(store, &format!("{folder}/ParticleNames.bcsv"))?)?;
    Ok(effects::extract(&ir, &names, &|_| true))
}

/// The game's rules among its tunables (`luma.json` "tunables", by "<file stem>.<name>"), what each does and the game's
/// own value. Every tunable, thousands, is listed by the game itself: GALAXY_DUMP_TUNABLES=<file>.
pub const SETTINGS: &[(&str, &str, &str)] = &[
    ("GameRules.cPlayerHealth", "3", "The health a player starts with, and the most before a Life Up"),
    ("MarioActor.cDarkCometOneHealth", "true", "Daredevil comets leave the player one health"),
    ("MarioActor.cHitTakesAll", "false", "A hit takes all the player's health"),
    ("GameRules.cLifeUpHealth", "6", "The most health a Life Up gives. 0: none"),
    ("MarioEffect.cLowHealthWarning", "true", "The warning sound at low health or air"),
    ("EventUtil.cDarkCometRules", "true", "Daredevil comets keep their rules; false: a comet like any other"),
    ("GameDataPlayerStatus.cMaxPlayerLeft", "99", "The most lives"),
    ("GameDataPlayerStatus.cStartingPlayerLeft", "4", "The lives a file starts and loads with"),
    ("GameRules.cMaxStarBits", "9999", "The most star bits kept in the save"),
    ("GameDataTemporaryInGalaxy.cMaxCarriedStarPieceNum", "999", "The most star bits carried in a stage"),
    ("GameRules.cMaxCoins", "999", "The most coins in a stage"),
    ("ScenePlayingResult.cCoinStepNum", "50", "Coins for a 1-up. 0: none"),
    ("ScenePlayingResult.cStarPieceStepNum", "50", "Star bits for a 1-up. 0: none"),
    ("ScenePlayingResult.cPurpleCoinPowerStarAppearNum", "100", "Purple coins that bring the star"),
    ("MarioActor.cWaterAir", "8", "The air the player has under water"),
    ("SequenceUtil.cMissStarPieceNum", "0", "Star bits added after a miss, negative to take. -999: all lost"),
    ("FlipPanel.cPanelGroupSize", "32", "How many flip panels a group holds"),
    ("FlipPanel.cObserverGroupSize", "32", "The same for their observers"),
    ("WaterBazooka.cShotInvalidSpinKill", "false", "A spin doesn't break the water bazooka's shots"),
    ("BossKameckBarrier.cOffsetY", "0", "Added to Kamek's barrier's height"),
    ("SkeletalFishBaby.cBreakAirBubble", "true", "A hit skeletal fish leaves an air bubble"),
    ("GameRules.cDodoryuShellOnly", "false", "Dodoryu is hurt by shells alone"),
    ("SceneUtil.cHiddenStarTable", "true", "Hidden stars take their mission from the game's table"),
    ("SceneUtil.cSuddenDeathDodoryu", "true", "Dodoryu's one-hit run in CosmosGardenGalaxy's fourth mission"),
    ("GameRules.cBegomanFireProof", "false", "Begomans survive fire ground"),
    ("KinopioAstro.cMessangerAlwaysEx", "false", "The 1-up messenger always gives its larger supply"),
    ("KinopioAstro.sMessangerOneUpExNum", "20", "How many 1-ups that is"),
];

/// The stage names the game's rules go by, and what playing as each brings.
pub const STAGE_RULES: &[(&str, &str)] = &[
    ("AstroGalaxy", "The hub: counters stay up, no 1-up from star bits, Mario walks the Observatory's way, shadows as the hub's, star bits from Lumas count for the dome"),
    ("AstroDome", "A dome's inside: the map opens there, its pause menu"),
    ("LibraryRoom", "The library: a hub location"),
    ("PeachCastleGardenGalaxy", "The prologue's stage: its wipe, its pause menu, its shadows"),
    ("PeachCastleFinalGalaxy", "The ending's castle: its Toads"),
    ("EpilogueDemoStage", "The epilogue: no pause menu, no wipe"),
    ("KoopaBattleVs1Galaxy", "A Bowser arena: the grand star's fall, demo and sequence"),
    ("KoopaBattleVs2Galaxy", "A Bowser arena, and enemies that drop items as this one's do"),
    ("KoopaBattleVs3Galaxy", "The last arena: the last grand star's long sequence"),
    ("OceanRingGalaxy", "Swimming with a limit on how far the player can tilt; the ocean's water sound"),
    ("OceanPhantomCaveGalaxy", "Water bazookas as here; Syati's and the music's rules"),
    ("HeavensDoorGalaxy", "Planets from the start, its flip panels, its Butterflies, its pipes; fades in"),
    ("HeavenlyBeachGalaxy", "Water as here"),
    ("HellProminenceGalaxy", "Mission 5's sideways camera, the Tamakoro ball's rules"),
    ("CannonFleetGalaxy", "Mogu and boss music as here"),
    ("BattleShipGalaxy", "Boss music, and the player's fire as here"),
    ("ReverseKingdomGalaxy", "Boss music as here"),
    ("SandClockGalaxy", "The player's sand rules"),
    ("TriLegLv1Galaxy", "Bowser Jr.'s entrance, the player's walking on the walker"),
    ("CosmosGardenGalaxy", "Mission 4 is the sudden-death Dodoryu"),
    ("FactoryGalaxy", "Onimasu's rules, the opening's rules"),
    ("EggStarGalaxy", "The first opening, mission 2's star bit limit"),
    ("HoneyBeeKingdomGalaxy", "Its opening"),
    ("IceVolcanoGalaxy", "Music, wipe"),
    ("SkullSharkGalaxy", "Water, its steam"),
    ("TearDropGalaxy", "Water sphere as here"),
    ("TeresaMario2DGalaxy", "Fans as here"),
    ("FishTunnelGalaxy", "Toad as here"),
    ("SurfingLv1Galaxy", "The surfing race's restart"),
    ("SurfingLv2Galaxy", "The same, harder"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names_come_from_the_self_description() {
        let file = br#"{"self": 1, "tables": [{"file": "/ObjectData/x", "columns": [{"name": "PlanetAmbientA", "type": "int"}, {"name": "vpanaxis.Z", "type": "float"}]}]}"#;
        let names = ColumnNames::from_self_description(file, Path::new("self.json")).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names.name(column_hash("PlanetAmbientA")), "PlanetAmbientA");
        assert_eq!(names.name(column_hash("vpanaxis.Z")), "vpanaxis.Z");
        assert_eq!(ColumnNames::none().name(0x1234), "0x00001234");
        assert!(ColumnNames::from_self_description(b"not json", Path::new("self.json")).is_err());
    }

    #[test]
    fn a_cooked_table_reads_back_as_the_json_that_made_it() {
        let source = br#"{"columns":{"scale_x":"float"},"rows":[{"name":"Kuribo","scale_x":1.5,"Count":-3},{"name":"","scale_x":2,"Count":7}]}"#;
        let cooked = nebula_import::cook::table::TableIr::from_json(source).unwrap().to_ntbl();
        let names = ColumnNames::from_self_description(br#"{"tables": [{"columns": [{"name": "name"}, {"name": "scale_x"}, {"name": "Count"}]}]}"#, Path::new("self.json")).unwrap();
        let json = table_to_json(&cooked, &names).unwrap();
        // Without the names, each column is its hash, which the cooker reads as that column again (the columns may
        // come out in another order: a JSON object's keys are sorted).
        let hashed = table_to_json(&cooked, &ColumnNames::none()).unwrap();
        assert_eq!(hashed["columns"][format!("0x{:08x}", column_hash("scale_x"))], "float");
        let again = nebula_import::cook::table::TableIr::from_json(serde_json::to_string(&hashed).unwrap().as_bytes()).unwrap().to_ntbl();
        assert_eq!(table_to_json(&again, &ColumnNames::none()).unwrap(), hashed);
        assert_eq!(json["columns"]["scale_x"], "float");
        let rows = json["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Kuribo");
        assert_eq!(rows[0]["Count"], -3);
        assert_eq!(rows[0]["scale_x"], 1.5);
        assert_eq!(rows[1]["scale_x"], 2.0);
        // And it cooks again to the same table.
        let again = nebula_import::cook::table::TableIr::from_json(serde_json::to_string(&json).unwrap().as_bytes()).unwrap().to_ntbl();
        assert_eq!(again, cooked);
    }
}
