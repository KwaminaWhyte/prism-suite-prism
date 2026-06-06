# Changelog — Prism suite

Suite-level changes (shared engine crates + cross-app interop). Each app keeps
its own changelog: [pigment](./pigment/CHANGELOG.md) ·
[contour](./contour/CHANGELOG.md) · [pulse](./pulse/CHANGELOG.md) ·
[reel](./reel/CHANGELOG.md).

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); pre-1.0.

## [Unreleased]

### Added
- Per-app `CHANGELOG.md` files (this file + one per app) to track work over time.
- `prism-core` — retouch primitives behind Pigment's Phase-6 tools:
  `heal::seamless_clone` (gradient-domain Poisson cloning), `heal::spot_heal`
  (auto-source blemish repair), `inpaint::content_aware_fill` (PatchMatch
  synthesis), `tone::dodge_burn` (local lighten/darken), and `warp` (displacement-
  field mesh warp + brush stamps, for Liquify).
- `prism-core` — `adjust::Curves` / `CurvePoints` (tone-curve adjustment data).

### Per-app progress (see each app's changelog)
- **Pigment** — Curves adjustment (GPU LUT); **Phase-6 retouch core**: Clone Stamp,
  Healing Brush, Spot Healing, Content-Aware Fill, Dodge & Burn, **Liquify**.
- **Contour** — undo/redo; direct-select path editing; stroke options
  (caps/joins/dashes); **multi-select + Align & Distribute**.
- **Pulse** — keyframe interpolation; graph editor; PNG image-sequence export +
  software compositor; **anchor point + layer parenting**.
- **Reel** — source in/out + ripple/roll/slip/slide editing; transitions; per-clip
  transform/opacity/crop + inspector; **sequence markers + work-area**.

## [0.0.1] - 2026-06-06

The suite established: one shared GPU-agnostic engine, four interoperating apps.

### Added
- **Shared engine crates** (root Cargo workspace):
  - `prism-color` — color science: `Rgba`, the sRGB↔linear boundary.
  - `prism-core` — GPU-agnostic document/scene model: layer tree, blend, tiles,
    adjust, curve, raster, shape, histogram.
  - `prism-io` — file↔pixels: image/PSD/EXR decode, text, resize, export,
    `.pigment` doc file.
- **Four apps**, each its own git repo + Cargo workspace, path-depending on the
  shared crates:
  - **Pigment** (Photoshop / raster) — most built; the only app with a custom
    wgpu compositor.
  - **Contour** (Illustrator / vector).
  - **Pulse** (After Effects / motion).
  - **Reel** (Premiere / video NLE).
- **Cross-app interop** — Pigment Dynamic-Links a Contour `.contour` artboard as a
  rasterized layer that re-renders when the source file changes (the suite's
  signature glue; `.contour` JSON is the cross-app contract).
- Suite docs: `README.md`, `SUITE.md` (vision), `RESEARCH.md` (shared-engine +
  interop research), per-app `PLAN.md`/`RESEARCH.md`.
