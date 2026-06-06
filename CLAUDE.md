# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Prism** is an open-source creative suite: four desktop apps built in Rust on one shared engine, each targeting ≥85% parity with an Adobe app.

| App | Adobe analog | Domain | Status |
|-----|--------------|--------|--------|
| **Pigment** | Photoshop | raster editing | most built (~60%); only app with a custom GPU compositor |
| **Contour** | Illustrator | vector graphics | ~25% |
| **Pulse** | After Effects | motion / compositing | ~10% |
| **Reel** | Premiere Pro | video NLE (editing) | ~8% |

The architectural bet: raster, vector, video frames, and comp layers all reduce to *compositing tiles through a DAG in linear light, cached by what's dirty*. Build that once; each app adds its own axis (Pulse adds time, Reel adds clips/edits). See `SUITE.md` for the vision, `RESEARCH.md` for suite-level research.

## Repository layout — nested workspaces, nested git repos

This is **not** one Cargo workspace. It is five:

```
prism/                  # root: git repo + Cargo workspace of the SHARED crates only
  Cargo.toml            #   members = crates/prism-{color,core,io}
  crates/
    prism-color/        # color science: Rgba, sRGB<->linear boundary, (later) ICC/OCIO
    prism-core/         # GPU-agnostic state: document, layer tree, blend, tiles, adjust, curve, raster, shape, histogram
    prism-io/           # file <-> pixels: image/psd/exr decode, text, resize, export, .pigment doc file
  pigment/  contour/  pulse/  reel/   # each is its OWN git repo AND its own Cargo workspace
```

- Each app dir is an **independent git repo** (the root `.gitignore` excludes `pigment/`, `contour/`, `pulse/`, `reel/`). Each is its own Cargo workspace whose single member is `<app>/crates/<app>-app`, path-depending on `../crates/prism-*`.
- **Separate agents own Contour, Pulse, and Reel.** Do not edit or commit another app's source unless that is the task — surface in-progress files you didn't create instead of committing them.
- Commit messages: **never** add a `Co-Authored-By` trailer (per user global config).

## Build / run / test

App binaries are NOT in the root workspace — you must work from inside the app dir. Each app's bin is named after the app (`default-run`).

```bash
# Run an app (from its dir):
cd pigment && cargo run            # also: contour / pulse / reel

# Shared-crate tests (from root):
cargo test -p prism-core           # or prism-color / prism-io

# An app's tests (from its dir):
cd pigment && cargo test

# A single test by name:
cargo test flood_fill              # substring match across the workspace
```

Unit tests live inline (`#[cfg(test)]`) in the crate/module they cover. Pigment additionally has a **headless GPU test** (`canvas::gpu_tests`) that boots a real wgpu device via `pollster`; it **skips silently when no GPU adapter is present** (CI / headless), so a "passing" run there may have run nothing.

## GPU model — Pigment is the exception

- **Pigment** uses eframe's **wgpu** backend and hand-written WGSL passes in `pigment/crates/pigment-app/src/shaders/` (`composite`, `display`, `dab`, `filter`, `selection`), driven from `canvas.rs`. All compositor passes are recorded into egui's own `CommandEncoder` in `prepare()`; `paint()` only issues the display draw. GPU resources persist across frames in egui `callback_resources`; reach them outside the paint callback via `with_gpu(frame, ...)`.
- **Contour / Pulse / Reel** use eframe's default **glow** backend and draw through egui's painter (plus uploaded textures for Reel) — **no custom GPU pass**. Don't add a wgpu pipeline to these without reason.
- Compositing is **linear-light, premultiplied, float** (`Rgba16Float` working textures). sRGB↔linear conversion is the gamma boundary owned by `prism-color`; encode happens at blit to egui's non-sRGB target.

## Engine boundaries (respect these)

- `prism-core` knows **nothing about wgpu** — it owns state only. Rendering lives in the app. Keep it that way; do not pull GPU types into `prism-core`.
- Keep the shared crates **app-agnostic and time-agnostic**. Pulse's time axis and Reel's clip model are layers *on top* of the shared compositor, not changes *to* it. Code that only one app needs belongs in that app, not in `crates/`.
- Planned `prism-*` promotions (not yet done — coordinate across app owners before promoting): `prism-vector` (paths/booleans/stroke), `prism-fx` (OpenFX-style effects), `prism-media` (FFmpeg + audio), `prism-ai` (`ort` runtime), `prism-doc` (interchange + Dynamic Link). The GPU compositor currently lives inside `pigment-app` and has **not** been promoted.

## Conventions & gotchas

- Pinned: `eframe`/`egui` **0.34**, `wgpu` 29 (re-exported by eframe — never pin `wgpu`/`egui_wgpu` separately, they must move in lockstep). Verify other crate versions with `cargo add` at build; PLAN/RESEARCH version notes can be stale.
- `prism-core` declares `[lints] workspace = true`, resolved against **whichever workspace builds it**. Every app workspace therefore must mirror the lint block (`clippy::needless_range_loop/too_many_arguments/field_reassign_with_default = "allow"`, `rust::deprecated = "allow"`) or builds error on undefined `workspace.lints`. When adding a new app workspace, copy that block.
- `deprecated = "allow"` is deliberate (egui 0.34 deprecations are mid-cycle) — don't "fix" deprecation warnings by churn.
- Dev profile: app at `opt-level = 1`, all deps at `3` (keeps the app usable while iterating).
- **Stale naming in docs:** `pigment/ARCHITECTURE.md` still refers to `pigment-core` / `pigment-gpu` / `pigment-io`. The real crates are `prism-core` / `prism-color` / `prism-io`, and there is no `pigment-gpu` crate (GPU lives in the app). Trust the code over that doc's names.

## Doc map

- `SUITE.md` (root + per-app copies) — suite vision, the four apps, interop mechanisms.
- `RESEARCH.md` (root) — suite-level shared-engine + interop research; per-app `RESEARCH.md` — cited findings backing that app's plan.
- `<app>/PLAN.md` — that app's phased roadmap to ≥85% Adobe parity, grounded in current code.
- `pigment/ARCHITECTURE.md` — Pigment module/data-flow detail (most-built app; read for the GPU compositor walkthrough).
