# Changelog — Prism suite

Suite-level changes (shared engine crates + cross-app interop). Each app keeps
its own changelog: [pigment](https://github.com/KwaminaWhyte/prism-suite-pigment/blob/main/CHANGELOG.md) ·
[contour](https://github.com/KwaminaWhyte/prism-suite-contour/blob/main/CHANGELOG.md) · [pulse](https://github.com/KwaminaWhyte/prism-suite-pulse/blob/main/CHANGELOG.md) ·
[reel](https://github.com/KwaminaWhyte/prism-suite-reel/blob/main/CHANGELOG.md).

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); pre-1.0.

## [Unreleased]

### Added
- **`prism-io::text` — optional font family + family enumeration.**
  `render_text` gains a trailing `family: Option<&str>` argument: when
  `Some(name)` (and non-empty) it requests that face via
  `Attrs::family(Family::Name(name))`, otherwise it keeps cosmic-text's default
  sans-serif. Unknown/empty names degrade gracefully through cosmic-text's font
  matching, so rendering never fails. New `available_families() -> Vec<String>`
  enumerates the system font database (sorted, de-duplicated) so apps can
  populate a font chooser; any name it returns is valid input to `render_text`.
  `prism-core::layer::TextDef` gains a `family: Option<String>` field
  (`#[serde(default)]` → `None`) so existing serialized text defs round-trip
  unchanged (absent key deserializes to `None`). Additive and app-agnostic:
  contour/pulse/reel still build (they don't construct `TextDef` literals). New
  tests cover family enumeration, set-vs-default/empty-family rendering, and the
  `TextDef` serde round-trip incl. the legacy (no-`family`) case.

## [0.1.0] - 2026-06-09

### Added
- **`prism-core::gradient`** — new shared, app-agnostic multi-stop gradient
  primitive (used first by Pigment's gradient editor/fill; reusable by Contour's
  gradient meshes and Pulse's ramp generators). Additive and back-compatible —
  no existing API changed, so contour/pulse/reel still build. A `Gradient` has an
  independent **color rail** (`ColorStop` = position + straight RGB in the
  caller's working/linear space) and **opacity rail** (`OpacityStop` = position +
  alpha), interpolated independently and combined (Photoshop's two-rail model).
  Five geometries (`GradientType`: Linear, Radial, Angle, Reflected, Diamond),
  each mapping a pixel to the gradient parameter `t` from a single `start→end`
  drag via `GradientType::param`. `Gradient::sample(t)` returns straight RGBA;
  `Gradient::render(start, end, w, h)` rasterizes to interleaved **premultiplied**
  linear RGBA f32 (matching `shape.rs`), with optional **ordered (Bayer 8×8)
  dithering** to suppress 8-bit banding — fully deterministic (no RNG),
  toggleable, and mean-preserving. Stops need not be pre-sorted (lookups sort
  internally) and clamp to the end stops outside the range. Helper constructors
  (`two_color`, `foreground_to_transparent`, `Default` = black→white) and stable
  `GradientType` id round-trip. All types derive serde, so gradients can be
  embedded in the `.pigment` container later. 18 unit tests cover stop
  interpolation in the working space (incl. multi-stop + unsorted), the
  independent opacity rail, each geometry's parameterization, dither
  determinism/presence/average-preservation, premultiplied render, and
  ids/edge-cases; `prism-io` adds a serde round-trip test.
- **`prism-media`** — new shared A/V decode crate (the planned `prism-media`
  promotion; co-owned with Pulse/Reel). App-agnostic (no wgpu / egui / app
  types): probes media metadata and decodes video frames + whole audio tracks.
  - **Backend: the ffmpeg / ffprobe CLI** (not a `-sys`/`-next` binding). Shells
    out via `std::process::Command`, so it is version-tolerant (works with the
    installed FFmpeg 8.x), needs no `pkg-config` / linking, and stays behind a
    small surface so an in-process libav backend can replace it later. Binary
    paths default to `ffmpeg`/`ffprobe` and are overridable via `PRISM_FFMPEG` /
    `PRISM_FFPROBE`.
  - **API**: `probe(path) -> MediaInfo` (duration / first video stream's
    `width`×`height` + `avg_frame_rate`→fps / audio presence + `AudioInfo`) via
    `ffprobe -print_format json -show_format -show_streams`; `decode_frame_at(path,
    t, scale) -> VideoFrame` (8-bit straight-alpha sRGB RGBA, exactly
    `w*h*4` bytes) via `ffmpeg -ss <t> -frames:v 1 -f rawvideo -pix_fmt rgba
    [-vf scale]`; `decode_audio(path, sample_rate, channels) -> AudioBuffer`
    (interleaved `f32le`, whole-file). `MediaError` (thiserror) with a dedicated
    `BinaryNotFound` so callers degrade gracefully (never panic) when FFmpeg is
    absent.
  - Tests gate on FFmpeg presence (skip silently when absent, mirroring the
    suite's GPU-test-skip convention) — they generate a `testsrc` lavfi clip in
    a temp dir, assert probe geometry/duration/fps and native + scaled frame
    byte counts, decode audio of a sine clip, and assert the missing-binary path
    surfaces `BinaryNotFound`.
- Per-app `CHANGELOG.md` files (this file + one per app) to track work over time.
- `prism-core` — retouch primitives behind Pigment's Phase-6 tools:
  `heal::seamless_clone` (gradient-domain Poisson cloning), `heal::spot_heal`
  (auto-source blemish repair), `inpaint::content_aware_fill` (PatchMatch
  synthesis), `tone::dodge_burn` (local lighten/darken), `tone::sponge`
  (saturation), `detail::blur_sharpen` (local blur/sharpen), and `warp`
  (displacement-field mesh warp + brush stamps, for Liquify).
- `prism-core` — `adjust::Curves` / `CurvePoints` (tone-curve adjustment data);
  `adjust::{Vibrance, PhotoFilter, Posterize}` adjustment variants.
- `prism-io` — `.pigment` doc model gains an optional per-layer `styles` payload
  (`LayerMeta.styles: Option<LayerStyles>`) holding the 8 non-destructive layer
  styles (stroke, drop/inner shadow, color/gradient overlay, outer/inner glow,
  bevel & emboss) as plain serde data (colors `[f32;4]`/`[f32;3]`, sizes/offsets
  in px, angles in deg — no GPU/app types). Serialized with `#[serde(default)]` +
  `skip_serializing_if` so old docs (no `styles` key) still load and style-less
  layers stay byte-compact; full-payload round-trip + old-doc back-compat tested.
- `prism-io` — `.pigment` doc model gains an optional per-layer `adjustment`
  payload (`LayerMeta.adjustment: Option<prism_core::Adjustment>`) that stores an
  adjustment layer's full descriptor (kind + every param, all 14 kinds) by
  reusing the shared `Adjustment` enum's own serde derive verbatim — so the
  variable-length Curves control points and the Channel Mixer matrix serialize
  unchanged, and adding adjustment kinds needs no `prism-io` change. App-agnostic
  (just the already-shared `prism-core` type). Serialized with `#[serde(default)]`
  + `skip_serializing_if` so old docs (no `adjustment` key) still load and
  non-adjustment layers stay byte-compact; full-payload round-trip + old-doc
  back-compat tested. Closes the data-loss gap where reopening a saved Pigment
  document dropped every adjustment layer's parameters.

### Per-app progress (see each app's changelog)
- **Pigment** — Curves adjustment (GPU LUT); **Phase-6 retouch core**: Clone Stamp,
  Healing Brush, Spot Healing, Content-Aware Fill, Dodge & Burn, Liquify,
  Detail brush (sponge/blur/sharpen); **Phase-7 adjustments**: Vibrance, Photo
  Filter, Posterize.
- **Contour** — undo/redo; direct-select path editing; stroke options
  (caps/joins/dashes); **multi-select + Align & Distribute**.
- **Pulse** — keyframe interpolation; graph editor; PNG image-sequence export +
  software compositor; **anchor point + layer parenting**.
- **Reel** — source in/out + ripple/roll/slip/slide editing; transitions; per-clip
  transform/opacity/crop + inspector; sequence markers + work-area; nested
  sequences; **real video decode** (`ClipSource::Video` via the new `prism-media`
  ffmpeg-CLI bridge — frames decoded + scrubbed on the timeline).

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
