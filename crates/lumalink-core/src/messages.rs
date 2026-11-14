//! The game's messages as a mod writes them in `text` rows and messages merges: words with the game's message tags in
//! brackets, the same words Nebula reads (native/include/nebula_text_tags.h):
//!
//! `[wait 30]` `[page]` `[center]` `[sound SE_NAME]` `[icon 7]` `[small]` `[normal]` `[large]` `[player]` `[color 1]`, and
//! `[tag G T u...]` for any other tag (group, tag number, its 16-bit parameter units). `[[` is one `[`.
//!
//! One implementation, the cook's (nebula-import cook/message_merge.rs): [`decode`] gives a message's text in those
//! words; [`encode`] is Nebula's reading of them, so a message read out and put back is the game's own, unit for unit.

use crate::say;
use anyhow::{Context, Result};
use nebula_import::cook::message::Nmsg;
use nebula_import::cook::message_merge;
use nebula_import::cook::table::{column_hash, Cell, TableIr};

/// One message's units (to its end, without its 0) as mod text.
pub fn decode(units: &[u16]) -> String {
    message_merge::decode_text(units)
}

/// Mod text as the game's units, without the 0.
pub fn encode(text: &str) -> Vec<u16> {
    message_merge::encode_text(text)
}

/// A language's messages, cooked (`<Language>/MessageData/Message`: message.bmg and messageid.tbl): (id, text) in the id
/// table's order. A message ends at its first 0 outside a tag (tags hold 0s).
pub fn read(message_file: &[u8], id_table: &[u8]) -> Result<Vec<(String, Vec<u16>)>> {
    let messages = Nmsg::from_nmsg(message_file).with_context(|| say!("LumaLink.Messages.File"))?;
    let ids = TableIr::from_ntbl(id_table).with_context(|| say!("LumaLink.Messages.IdTable"))?;
    let column = |name: &str| ids.columns.iter().position(|c| c.hash == column_hash(name)).with_context(|| say!("LumaLink.Messages.NoColumn", name = name));
    let (id_at, index_at) = (column("MessageId")?, column("Index")?);
    let mut out = Vec::with_capacity(ids.rows.len());
    for row in &ids.rows {
        let (Cell::Text(id), Cell::Bits(index)) = (&row[id_at], &row[index_at]) else {
            continue;
        };
        let Some(message) = messages.messages.get(*index as usize) else {
            continue;
        };
        out.push((String::from_utf8_lossy(id).into_owned(), message_merge::units_at(&messages.text, message.text as usize).to_vec()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_read_out_goes_back_the_same() {
        let units = encode("Hi [player]! [[sic] [color 3]Star[color 0][wait 20][page][icon 12][tag 6 5 0 1 0 0][sound SE_SV_TICO]");
        let text = decode(&units);
        assert_eq!(text, "Hi [player]! [[sic] [color 3]Star[color 0][wait 20][page][icon 12][tag 6 5 0 1 0 0][sound SE_SV_TICO]");
        assert_eq!(encode(&text), units);
    }

    #[test]
    fn a_message_ends_at_its_first_0_outside_a_tag() {
        let mut text = encode("[color 1]A[color 0]");
        text.push(0);
        text.extend(encode("B"));
        assert_eq!(decode(message_merge::units_at(&text, 0)), "[color 1]A[color 0]");
        assert_eq!(decode(message_merge::units_at(&text, text.len() - 1)), "B");
    }
}
