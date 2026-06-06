# Prism Suite — Research Findings (June 2026)

Suite-level research backing [SUITE.md](./SUITE.md) (vision + interop) and the four app plans. This doc
covers the **shared engine, the interop mechanisms, and the cross-cutting policies** (color, AI) that all
four apps inherit. App-specific findings live in each app's own `RESEARCH.md`:

- [pigment/RESEARCH.md](./pigment/RESEARCH.md) — raster engine, compositor, blend math, brush, color IO, AI tools
- [contour/RESEARCH.md](./contour/RESEARCH.md) — vector path engine, booleans, SVG/PDF, image trace, gradients
- [pulse/RESEARCH.md](./pulse/RESEARCH.md) — time-addressable compositing, keyframes/expressions, effects, media
- [reel/RESEARCH.md](./reel/RESEARCH.md) — NLE editing model, video/audio decode, transitions, color, export

> Verify every crate version against crates.io at build time — third-party version metadata is sometimes
> stale. `wgpu` confirmed at **29.0.3** (2026-05-02); the `"29"` pin holds suite-wide.

---

## 1. The architectural bet — one engine, four apps

Adobe's moat isn't any single app; it's that the apps **interoperate** because they share a layer/
compositor/color engine. Prism gets the same property *if and only if* that engine is one codebase. The
key realization that makes this tractable:

> **Raster, vector preview, video frames, and motion comps all reduce to the same operation —
> composite tiles through a DAG of blend/effect nodes, in linear light, cached by what's dirty.**

- **Pigment** runs that compositor over raster layers.
- **Contour** is resolution-independent paths *rasterized through* the same compositor for preview/export.
- **Pulse** adds a **time axis**: every node is sampled at frame `t` (keyframes/expressions); cache key
  gains a frame dimension.
- **Reel** adds **clips on tracks with source in/out ranges**: the program frame is that same composite
  sampled at the playhead, plus an audio mix.

So Pulse = Pigment's compositor + time; Reel = the compositor + a clip/edit model + media. Build the
compositor **time-agnostic and clip-agnostic**; time and clips are each a thin layer on top. This is why
the shared crates must never bend toward one app's UI.

Sources: [SUITE.md](./SUITE.md) · pigment/RESEARCH.md §2 (tile compositing) · pulse/RESEARCH.md §1 · reel/RESEARCH.md §1

## 2. Shared crate matrix (current + planned)

Shared crates live at `prism/crates/` as their own workspace; every app path-deps in via `../crates/`
and **does not copy or modify** them.

| Crate | Status | Owns | Consumers |
|---|---|---|---|
| `prism-core` | **exists** | doc model, `Size`/`Rect`, color boundary, blend modes (18), tile types, adjustments, curve/histogram | pigment, contour, pulse, reel |
| `prism-color` | **exists** | sRGB/linear, ICC (`lcms2`/`qcms`) — grows OCIO, CMYK, spot, soft-proof | pigment (+ all on color tasks) |
| `prism-io` | **exists** | image load/export, PSD/EXR, resize, `.pigment`, text raster | pigment, reel (`load_image`) |
| `prism-vector` | **planned** | paths/anchors/handles, booleans (`i_overlay`), stroking/offset (`kurbo`), tessellation (`lyon`) | contour (owner), pigment (shape layers), pulse (masks/shape layers) |
| `prism-fx` | **planned** | OpenFX-style GPU effect/transition host — author once, run everywhere | pigment (filters), contour (live effects), pulse (effects), reel (transitions/effects) |
| `prism-media` | **planned** | FFmpeg decode/encode + frame-accurate seek + audio (`symphonia`/`cpal`/`rubato`) | pulse + reel (co-owned) |
| `prism-ai` | **planned** | `ort` (ONNX) runtime + provider abstraction + on-demand model cache | all (segmentation, matting, upscale, inpaint, transcription) |
| `prism-doc` | **planned** | interchange container (layer tree + scene graph + media refs) + Dynamic-Link node | all (interop) |

**Promotion discipline:** code is promoted to a `prism-*` crate only when it is genuinely generic. The
README/PLAN of each app names a "coordinate before promoting" step so the owning agents agree on a shape
that serves *all* consumers (e.g. `prism-vector` must satisfy Contour authoring **and** Pigment shape
layers **and** Pulse masks). Never raster-couple, time-couple, or clip-couple a shared crate.

Sources: prism/Cargo.toml (workspace members) · each app's Cargo.toml (path deps) · contour/RESEARCH.md §1 · pulse/RESEARCH.md §6 · reel/RESEARCH.md §2–3

## 3. Interop mechanisms (the Adobe-parity features)

All four reduce to **one render-graph-node abstraction** plus a shared container; build it suite-aware
from the start.

1. **Dynamic Link** — a node that evaluates a *linked* document on demand (at the requested time/
   resolution/tile) and caches the result. A Pulse comp in a Reel timeline, a Contour artboard in a
   Pigment doc, a nested sequence — all the **same** node, differing only in what they evaluate. Producer
   = Pulse (comps → Reel); consumers = Reel/Pigment/Contour.
2. **Smart objects / live placement** — the same node, embedded rather than externally linked; stays
   editable at source resolution, re-rasterized on transform. Pigment's smart objects = this node.
3. **`prism-doc` interchange container** — one format every app reads (layer tree + scene graph + media
   refs); lossy-but-faithful bridges to PSD/AI/SVG/AEP/Premiere-XML/OTIO. Defined with the suite.
4. **Shared clipboard** — copy a path/layer/keyframe/color in one app, paste editable in another (shared
   in-memory model + serialized fallback).
5. **One color pipeline** — `prism-color` means a swatch/look is identical across all four and on export.
6. **Shared effects** — `prism-fx` effects authored once run in any compositing app.
7. **Shared asset library** — brushes, gradients, LUTs, fonts, templates, swatches in a common store.

Sources: [SUITE.md](./SUITE.md) §"Interop mechanisms" · pulse/RESEARCH.md §10 · reel/RESEARCH.md §1,7

## 4. One color pipeline

All apps composite in **linear-light premultiplied** and manage color through `prism-color`:
- **`lcms2`** (Little CMS 2.17) primary engine — ICC v2/v4, CMYK/Lab/XYZ, soft-proof, intents;
  **`qcms`** pure-Rust RGB/gray fast path for wasm.
- **OpenColorIO** (ASWF Rust binding in progress) for the video/VFX apps (Pulse/Reel) — config-driven
  input/working/display/output transforms + creative looks; **`exr`** (pure Rust) covers scene-linear
  EXR meanwhile.
- Float working buffers (`Rgba16Float`; `f32` on demand); sRGB/transfer encode only at the final
  display/export boundary; HDR (>1.0, PQ/HLG) preserved end-to-end.
The payoff: a color picked in Contour, graded in Reel, and composited in Pulse is the **same color**, and
exports match.

Sources: pigment/RESEARCH.md §2–3,9 · pulse/RESEARCH.md §5 · reel/RESEARCH.md §6 · aswf.io (OCIO/OpenEXR Rust)

## 5. Shared AI policy (`prism-ai` / `ort`)

One runtime, one policy across the suite:
- **Runtime:** **`ort`** (ONNX Runtime) with **CoreML** (macOS), **DirectML** (Windows), **CUDA/TensorRT**
  (NVIDIA) execution providers; **`candle`**/**`tract`** pure-Rust fallback (wasm / no-EP).
- **Models are never bundled.** Fetched to a shared cache on first use **behind a feature flag**, with
  each weight's **license surfaced** (segmentation/restoration weights mostly MIT/Apache; diffusion
  weights carry OpenRAIL terms). Every AI tool **degrades gracefully** when models/GPU are absent.
- **Shared models across apps:** segmentation/matting (**SAM2/SAM3**, **BiRefNet_dynamic**, **RMBG-2.0**)
  power select-subject (Pigment), trace-region (Contour), roto (Pulse), object-mask/auto-reframe (Reel);
  inpaint (**LaMa**) powers content-aware fill (Pigment) and video CAF (Reel); super-res (**Real-ESRGAN/
  SwinIR**) is shared; transcription (**Whisper-class**) powers captions/text-based editing (Reel).
- **Generative fill / expand / extend — explicit suite policy: OPTIONAL and PLUGGABLE.** It runs via a
  provider abstraction with **two interchangeable backends — a local diffusion model (`candle`/ONNX) and
  a user-configured cloud endpoint (bring-your-own API key)** — plus "none". It is **never required** for
  core editing; the apps are fully functional with no AI backend configured. This applies uniformly to
  Pigment (Generative Fill), Contour (Generative Recolor/vectorize), Pulse (Generative Extend), and Reel
  (Generative Extend).

Sources: pigment/RESEARCH.md §10 · contour/RESEARCH.md §8 · pulse/RESEARCH.md §7 · reel/RESEARCH.md §9 · github.com/pykeio/ort

## 6. Shared app shell & tooling

- **App shell:** `eframe`/`egui` 0.34 (lockstep) across all four. Pigment/Pulse use the **wgpu** backend
  (shared GPU device with the compositor canvas); Contour/Reel currently use **glow** and move to wgpu
  where perf demands (large vector docs / GPU program monitor). Each app keeps its own `theme.rs`/
  `icons.rs` (phosphor) so look can drift, but all derive from the shared Prism dark theme.
- **Common deps:** `serde` (doc IO), `glam` (math), `bytemuck` (GPU casts), `rayon` (parallel
  tile/frame/boolean work), `thiserror`/`anyhow`, `rfd` (dialogs), `kurbo` (Bézier — vector + easing).
- **Automation/extensibility:** `rhai` (sandboxed scripting/actions) suite-wide; OpenFX-style plugins via
  `prism-fx`. Undo is per-app (tile-COW pixels in Pigment; small command stacks elsewhere) but the
  command pattern is shared.

Sources: each app's Cargo.toml · pigment/RESEARCH.md §1 (eframe/egui/wgpu) · pigment/RESEARCH.md §11 (rhai/OpenFX)

## 7. Current state of the suite (June 2026)

All four apps are scaffolded and run; the shared crates are promoted. Rough parity vs the Adobe analog:

| App | Analog | Built today | Approx parity | Next big lever |
|---|---|---|---|---|
| **Pigment** | Photoshop | Phases 0–5: GPU compositor, layers/blend, brush/wet-layer, selection/transform, adjustments/masks/filters, text/vector basics, PSD/EXR IO | ~60% | Retouch/heal + layer styles/smart objects (Ph6–7) |
| **Contour** | Illustrator | Bézier pen, shapes, pathfinder (i_overlay), SVG/PNG export, save | ~25% | Undo, then selection/layers/appearance (Ph1–3) |
| **Pulse** | After Effects | Comp + linear keyframe tracks, timeline, CPU solid preview, save | ~10% | Typed properties + Bézier ease + GPU compositor (Ph1) |
| **Reel** | Premiere Pro | Multitrack timeline, clip move/trim, stills preview, bin, save | ~8% | A/V engine: video decode + audio + source in/out (Ph1) |

Each app's PLAN.md defines its road to ≥85% parity and the phase where that line lands. Sequencing
principle suite-wide: **build the foundation that gates breadth first** (Pulse's property/compositor
rebuild, Reel's A/V engine, Contour's undo, Pigment's retouch core), then fan out.

Sources: pigment/PLAN.md · contour/PLAN.md · pulse/PLAN.md · reel/PLAN.md
