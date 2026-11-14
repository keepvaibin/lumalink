# LumaLink

Makes, checks, builds and packs mods for [Nebula](https://github.com/keepvaibin/nebula) (`.luma` files). It checks
with the game's own validator, so a mod LumaLink packs is one the game accepts.

## Commands

| Command | What it does |
|---|---|
| `lumalink new <folder> --id <id> --name <name> --summary <line> --author <who>` | Starts a mod: `luma.json` and an empty base layer |
| `lumalink add <what> ...` | Adds a need, option, permission, layer, file, asset, texture, text, character and more, checked before it is kept |
| `lumalink check <folder or .luma>` | Lists every problem with its code, file and line; `--json` for tools |
| `lumalink build <folder>` | Builds the mod's LumaScript and C for every platform |
| `lumalink pack <folder>` | Writes the `.luma` once every rule passes and none is left unchecked |
| `lumalink run <folder or .luma>` | Imports the mod into the installed game and starts it |
| `lumalink test <folder>` | Builds the mod and runs each `tests/**/test.json` in the game, hidden and silent |
| `lumalink convert <mod> <folder>` | Converts a Riivolution pack, disc-layout folder, loose files, texture pack or Gecko list to a mod folder |
| `lumalink format <paths>` | Formats `.lum` files; `--check` writes nothing |
| `lumalink list <.luma>` | What is inside |
| `lumalink extract <.luma> [entries] -o <folder>` | Writes entries out |
| `lumalink verify <.luma> --source <folder>` | Whether the `.luma` is exactly what the folder packs to |
| `lumalink explain L203` | What a code means |
| `lumalink list tables --game game.pak` | Tables, models, messages or files of your imported game; `--self` names table columns |
| `lumalink extract <path> --game game.pak -o <folder>` | Files out of your imported game |
| `lumalink lsp` | The editor server (Language Server Protocol over stdin and stdout) |
| `lumalink services` | Every service function of the game's SDK and whether scripts reach it |

Every command that reads the game uses the installed one unless `--game <game.pak>` and `--names <folder>` name
another; `--no-installed` leaves it out. Messages and help come in any of the game's 14 languages
(`--language EuGerman`, `lumalink help check --language JpJapanese`).

## Editors

`editors/vscode` is the LumaScript extension for Visual Studio Code: colours, brackets, snippets, and errors as you
type, completion, hover, go to definition, rename and formatting from `lumalink lsp`. Copy the folder into VS Code's
extensions folder (`~/.vscode/extensions/lumascript`) and restart it. Other editors run `lumalink lsp` with their own
Language Server Protocol client; Nebula's `docs/modding/lumalink/editor.md` has Neovim and Helix settings.

## Building

LumaLink uses Nebula's validator and cooker crates. `nebula/` must be a Nebula tree: a checkout of the commit in
`nebula.commit`, or a link to a Nebula worktree.

```console
git clone https://github.com/keepvaibin/nebula.git nebula
git -C nebula checkout $(cat nebula.commit)
cargo test --workspace
```

GPL-3.0-only.
