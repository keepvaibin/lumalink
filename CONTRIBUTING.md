# Contributing to LumaLink

## Code

```console
cargo test --workspace
cargo clippy --workspace --all-targets
```

- The rules a `.luma` is held to live in Nebula's `crates/luma-validate`, the one library the game runs too. LumaLink
  never has its own copy of a rule.
- Everything LumaLink cannot use is named with the reason. Nothing is dropped silently.
- A path from a mod or an archive never reaches outside its folder.
- Tests make their own small files. No game files or mods are committed.
- Every line LumaLink says exists in the game's 14 languages: a row of `crates/lumalink-core/text/LumaLinkText.tsv`,
  said with `say!`. The build refuses a row that lacks a language or whose placeholders differ; a test refuses an id
  without a row and a row never said.

## Code patches of published mods

`crates/lumalink-core/src/known_patches.rs` says which bytes at which address mean which change. Conversion
turns each into a data patch by key or a small generated mod with a hook; a patch nobody has mapped is reported by
the game function it touches.

By contributing, you agree that your work is licensed under GPL-3.0-only.
