//! A game table a mod brings whole, written as the patch by key the one form takes (docs/modding/format/luma.md,
//! section 5.1; docs/modding/format/table-keys.md; the validator's L303): the rows the mod changes (`set`, only the cells that differ), the rows it adds
//! and the game's rows it leaves out (`remove`), against the game's own table. A table named by place keeps its rows
//! whole (`"key": ["#"]` and `rows`, the validator's by-place form). Other mods' patches of the same table then still apply, and nothing of the mod's table is lost
//! but what the game's table cannot hold, which is named.

use crate::inspect::ColumnNames;
use anyhow::{bail, Result};
use nebula_import::cook::table::{Cell, Column, Kind, TableIr};
use nebula_import::cook::table_keys::{rule_for, Key};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// What a whole table came to.
#[derive(Debug, Default, PartialEq)]
pub struct Patched {
    /// The patch (`layers/<l>/patches/<path>.json`); None when the mod's table is the game's.
    pub patch: Option<Value>,
    pub set: usize,
    pub added: usize,
    pub removed: usize,
    /// The mod's columns the game's table does not have (by name or hash): their values cannot reach the game.
    pub lost_columns: Vec<String>,
    /// Keys the mod's table gives more than once: the first row of each is the one used.
    pub repeated: Vec<String>,
}

/// A cell as the JSON a patch writes, read as `column` holds it (as `lumalink extract` writes tables).
fn cell_json(column: &Column, cell: &Cell) -> Value {
    match (column.kind, cell) {
        (_, Cell::Text(word)) => match std::str::from_utf8(word) {
            Ok(text) => Value::String(text.to_string()),
            Err(_) => json!({ "bytes": word.iter().map(|b| format!("{b:02x}")).collect::<String>() }),
        },
        (Kind::Float, Cell::Bits(bits)) => json!(f64::from(f32::from_bits(*bits))),
        (Kind::Int, Cell::Bits(bits)) => match column.width {
            8 => json!(i64::from(*bits as u8 as i8)),
            16 => json!(i64::from(*bits as u16 as i16)),
            _ => json!(i64::from(*bits as i32)),
        },
        (Kind::Field, Cell::Bits(bits)) => json!(*bits),
        (Kind::Text, Cell::Bits(bits)) => json!(*bits),
    }
}

/// `mods` (the mod's table at `path`, a game path) against `game` (the game's table there).
pub fn patch_of(path: &str, game: &TableIr, mods: &TableIr, names: &ColumnNames) -> Result<Patched> {
    let rule = rule_for(&nebula_import::cook::store::store_path(path.trim_start_matches('/')));
    let mut out = Patched::default();
    // The mod's columns by the game's: a column the game lacks cannot be patched in.
    let mut common: Vec<(usize, usize)> = Vec::new();
    for (mc, column) in mods.columns.iter().enumerate() {
        match game.columns.iter().position(|g| g.hash == column.hash) {
            Some(gc) => common.push((mc, gc)),
            None => out.lost_columns.push(names.name(column.hash)),
        }
    }
    let name_of = |gc: usize| -> String {
        let hash = game.columns[gc].hash;
        match &rule.key {
            Key::Columns(keys) => keys.iter().find(|k| nebula_import::cook::table::column_hash(k) == hash).cloned().unwrap_or_else(|| names.name(hash)),
            Key::Position => names.name(hash),
        }
    };
    // A mod's row as the patch writes it: each column both tables have, read as the game's column holds it.
    let row_json = |row: &[Cell]| -> Map<String, Value> { common.iter().map(|&(mc, gc)| (name_of(gc), cell_json(&game.columns[gc], &row[mc]))).collect() };
    let keys = match &rule.key {
        Key::Position => {
            let same = mods.rows.len() == game.rows.len() && mods.rows.iter().zip(&game.rows).all(|(m, g)| common.iter().all(|&(mc, gc)| cell_json(&game.columns[gc], &m[mc]) == cell_json(&game.columns[gc], &g[gc])));
            if !same {
                out.patch = Some(json!({ "patch": 1, "key": ["#"], "rows": mods.rows.iter().map(|r| Value::Object(row_json(r))).collect::<Vec<_>>() }));
            }
            return Ok(out);
        }
        Key::Columns(keys) => keys,
    };
    // The key's columns in each table (the one missing, when one is).
    let at = |t: &TableIr| -> std::result::Result<Vec<usize>, &String> {
        keys.iter().map(|k| t.columns.iter().position(|c| c.hash == nebula_import::cook::table::column_hash(k)).ok_or(k)).collect()
    };
    let gk = at(game).map_err(|k| anyhow::anyhow!(crate::say!("LumaLink.Table.GameNoKeyColumn", column = k)))?;
    let mk = at(mods).map_err(|k| anyhow::anyhow!(crate::say!("LumaLink.Table.NoKeyColumn", column = k)))?;
    let id = |t: &TableIr, k: &[usize], row: &[Cell]| -> String {
        let parts: Vec<String> = k.iter().map(|&c| nebula_import::cook::patch::cell_text(&t.columns[c], &row[c])).collect();
        let joined = parts.join("\u{1f}");
        if rule.case_insensitive {
            joined.to_lowercase()
        } else {
            joined
        }
    };
    // The game repeats a few keys (docs/modding/format/table-keys.md, section 4): a `where` names the first.
    let mut game_rows: HashMap<String, usize> = HashMap::new();
    for (r, row) in game.rows.iter().enumerate() {
        game_rows.entry(id(game, &gk, row)).or_insert(r);
    }
    let key_json = |row: &[Cell]| -> Map<String, Value> { keys.iter().zip(&mk).zip(&gk).map(|((k, &mc), &gc)| (k.clone(), cell_json(&game.columns[gc], &row[mc]))).collect() };
    let (mut set, mut add, mut remove) = (Vec::new(), Vec::new(), Vec::new());
    let mut seen: HashMap<String, ()> = HashMap::new();
    for row in &mods.rows {
        let row_id = id(mods, &mk, row);
        if seen.insert(row_id.clone(), ()).is_some() {
            if !out.repeated.contains(&row_id) {
                out.repeated.push(row_id.replace('\u{1f}', ", "));
            }
            continue;
        }
        match game_rows.get(&row_id) {
            Some(&g) => {
                let changed: Map<String, Value> = common
                    .iter()
                    .filter(|&&(mc, gc)| cell_json(&game.columns[gc], &row[mc]) != cell_json(&game.columns[gc], &game.rows[g][gc]))
                    .map(|&(mc, gc)| (name_of(gc), cell_json(&game.columns[gc], &row[mc])))
                    .collect();
                if !changed.is_empty() {
                    set.push(json!({ "where": Value::Object(key_json(row)), "values": Value::Object(changed) }));
                }
            }
            None => add.push(Value::Object(row_json(row))),
        }
    }
    let mut gone: Vec<(usize, Map<String, Value>)> = Vec::new();
    for (row_id, &g) in &game_rows {
        if !seen.contains_key(row_id) {
            let row = &game.rows[g];
            gone.push((g, keys.iter().zip(&gk).map(|(k, &gc)| (k.clone(), cell_json(&game.columns[gc], &row[gc]))).collect()));
        }
    }
    gone.sort_by_key(|(g, _)| *g);
    remove.extend(gone.into_iter().map(|(_, w)| Value::Object(w)));
    (out.set, out.added, out.removed) = (set.len(), add.len(), remove.len());
    if set.is_empty() && add.is_empty() && remove.is_empty() {
        return Ok(out);
    }
    let mut patch = Map::new();
    patch.insert("patch".into(), json!(1));
    patch.insert("key".into(), json!(keys));
    for (name, list) in [("remove", remove), ("set", set), ("add", add)] {
        if !list.is_empty() {
            patch.insert(name.into(), Value::Array(list));
        }
    }
    out.patch = Some(Value::Object(patch));
    Ok(out)
}

/// A table file of a mod, in any of its forms: the disc's (BCSV), the cooked one (NTBL) or the source form (JSON).
pub fn read_table(bytes: &[u8]) -> Result<TableIr> {
    use nebula_import::cook::table;
    if bytes.starts_with(table::MAGIC) {
        return TableIr::from_ntbl(bytes);
    }
    if table::is_bcsv(bytes) {
        if let Some(t) = TableIr::from_bcsv(bytes)? {
            return Ok(t);
        }
    }
    if bytes.first() == Some(&b'{') {
        return TableIr::from_json(bytes);
    }
    bail!(crate::say!("LumaLink.Table.NotATable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> TableIr {
        TableIr::from_json(text.as_bytes()).unwrap()
    }

    #[test]
    fn a_whole_table_becomes_its_changes_by_key() {
        // messageid.tbl's rule: by MessageId (docs/modding/format/table-keys.md).
        let path = "/LocalizeData/UsEnglish/MessageData/Message.arc/messageid.tbl";
        let key = match &rule_for(&nebula_import::cook::store::store_path(path.trim_start_matches('/'))).key {
            Key::Columns(k) => k.clone(),
            Key::Position => panic!("messageid.tbl is keyed"),
        };
        assert_eq!(key, ["MessageId"]);
        let game = table(r#"{"columns":{"Scale":"float"},"rows":[{"MessageId":"A","Kind":1,"Scale":1.0},{"MessageId":"B","Kind":2,"Scale":2.0},{"MessageId":"C","Kind":3,"Scale":3.0}]}"#);
        // B changed in one cell, C left out, D added, A as the game has it, a column the game lacks, and A again.
        let mods = table(r#"{"columns":{"Scale":"float"},"rows":[{"MessageId":"A","Kind":1,"Scale":1.0,"New":5},{"MessageId":"B","Kind":2,"Scale":2.5,"New":5},{"MessageId":"D","Kind":4,"Scale":1.0,"New":5},{"MessageId":"A","Kind":9,"Scale":1.0,"New":5}]}"#);
        let names = ColumnNames::none();
        let got = patch_of(path, &game, &mods, &names).unwrap();
        let hash = |n: &str| names.name(nebula_import::cook::table::column_hash(n));
        assert_eq!(
            got.patch.unwrap(),
            json!({
                "patch": 1,
                "key": ["MessageId"],
                "remove": [{"MessageId": "C"}],
                "set": [{"where": {"MessageId": "B"}, "values": {hash("Scale"): 2.5}}],
                "add": [{"MessageId": "D", hash("Kind"): 4, hash("Scale"): 1.0}],
            })
        );
        assert_eq!((got.set, got.added, got.removed), (1, 1, 1));
        assert_eq!(got.lost_columns, [hash("New")]);
        assert_eq!(got.repeated, ["A"]);
        // The game's own table: nothing to patch.
        assert_eq!(patch_of(path, &game, &game, &names).unwrap(), Patched::default());
        // No key column in the mod's table: said, not guessed.
        let keyless = table(r#"{"rows":[{"Kind":1}]}"#);
        assert!(patch_of(path, &game, &keyless, &names).unwrap_err().to_string().contains("MessageId"));
    }

    #[test]
    fn a_table_named_by_place_is_given_whole() {
        // GalaxyInfoTable has no rule of its own: by place (table-keys' last rule).
        let path = "/NebulaData/GalaxyInfoTable.bcsv";
        assert_eq!(rule_for(&nebula_import::cook::store::store_path(path.trim_start_matches('/'))).key, Key::Position);
        let game = table(r#"{"rows":[{"v":1},{"v":2}]}"#);
        let mods = table(r#"{"rows":[{"v":1},{"v":3}]}"#);
        let names = ColumnNames::none();
        let got = patch_of(path, &game, &mods, &names).unwrap();
        let v = names.name(nebula_import::cook::table::column_hash("v"));
        assert_eq!(got.patch.unwrap(), json!({"patch": 1, "key": ["#"], "rows": [{v.clone(): 1}, {v: 3}]}));
        assert_eq!(patch_of(path, &game, &game, &names).unwrap().patch, None);
    }

    #[test]
    fn a_table_file_reads_in_every_form() {
        let t = table(r#"{"rows":[{"name":"A"}]}"#);
        assert_eq!(read_table(&t.to_ntbl()).unwrap(), t);
        assert_eq!(read_table(br#"{"rows":[{"name":"A"}]}"#).unwrap(), t);
        assert!(read_table(b"neither").is_err());
    }
}
