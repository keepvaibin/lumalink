# Game tests of LumaKit and LumaScript

Each test runs in the game: `<suite>/<test>/test.json` (the harness format, docs/modding/how-tos/testing-a-mod.md in
the Nebula repository), its mods in `mods/`, each built from `source/` by `lumalink build`.

- `kit`: LumaKit's modules, used by mods as a modder uses them.
- `script`: the language and its compiled form.

The kit under test is this repository's `lumakit/`. Tests name it `../../mods/lumakit`; the runner puts a copy there,
beside the other mods, where `lumalink build` looks first for a mod that a mod needs.

The Nebula repository's gate runs these with its own tests: `tools/lumacore/mod_tests.py`, with `LUMALINK` (this
repository's program, built from the commit in Nebula's `tools/lumacore/lumalink.commit`) and `LUMALINK_TREE` (this
checkout).
