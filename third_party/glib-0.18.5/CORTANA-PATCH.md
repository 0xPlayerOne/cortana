# Cortana glib 0.18.5 security backport

This directory is the `glib 0.18.5` crate published by gtk-rs, vendored only for the Linux Tauri
dependency graph.

- Crates.io archive SHA-256:
  `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`
- Advisory: `RUSTSEC-2024-0429` / `GHSA-wrw7-89jp-8q8g`
- Upstream fix: gtk-rs/gtk-rs-core commit
  `05dff0ee696f9bcd8617cd48c4b812d046d440cb`
- Local delta: in `src/variant_iter.rs`, pass the C out-argument as `&mut p` instead of the
  unsound immutable `&p`.
- Repository normalization: remove one surplus blank line at the end of `LICENSE`.

Tauri's current GTK3 stack pins the `glib 0.18` API family, while RustSec marks only `glib 0.20+`
as patched. Do not remove this patch until Tauri/wry no longer resolves the affected 0.18 crate.
Run `bun scripts/verify-vendored-glib.mjs` after any dependency update.
