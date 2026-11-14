//! LumaLink's engine. [`luma_file`] writes and reads `.luma` files (docs/modding/format/luma.md), each one checked by the
//! game's own validator (`luma-validate`, the same library the game runs). [`inspect`] and [`game_store`] read the
//! player's imported game. [`text`] holds LumaLink's own lines in the game's 14 languages. The readers of published mods ([`source`], [`layout`], [`riivolution`], [`texture_pack`],
//! [`gecko`]) and the conversion decisions ([`plan`], [`known_patches`], [`region_map`], [`object_table`],
//! [`player_constants`]) are kept for conversion, which writes the new format.

pub mod add;
pub mod build;
pub mod character;
pub mod code_stubs;
pub mod convert;
pub mod editor;
pub mod effects_check;
pub mod files_check;
pub mod gecko;
pub mod game_functions;
pub mod game_store;
pub mod inspect;
pub mod installed;
pub mod jobs;
pub mod known_patches;
pub mod object_table;
pub mod layout;
pub mod lsp;
pub mod luma_file;
pub mod messages;
pub mod numbers_code;
pub mod plan;
pub mod table_patch;
pub mod text;
pub mod player_constants;
pub mod ppc;
pub mod region_map;
pub mod riivolution;
pub mod script;
pub mod source;
pub mod texture_pack;

