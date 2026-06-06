<h1 align="center">Prism</h1>

<p align="center">
  <b>An open source creative suite</b> — four interoperating apps that work together
  the way Adobe's Creative Cloud does, built in Rust on a shared GPU engine.
</p>

---

See [SUITE.md](./SUITE.md) for the full vision (shared engine, Dynamic Link,
smart objects, common color pipeline) and [RESEARCH.md](./RESEARCH.md) for the
suite-level research (shared crate matrix, interop mechanisms, color & AI policy).

## The apps

| # | App | Adobe analog | Domain | Status |
|---|-----|--------------|--------|--------|
| 1 | **[Pigment](https://github.com/KwaminaWhyte/prism-suite-pigment)** | Photoshop | Raster image editing | 🟢 Phases 0–5; **live-links** `.contour` docs |
| 2 | **[Contour](https://github.com/KwaminaWhyte/prism-suite-contour)** | Illustrator | Vector graphics | 🟢 v0 — rect/ellipse/line/**bezier pen**, **SVG/PNG export**, layers, **boolean ops** |
| 3 | **[Pulse](https://github.com/KwaminaWhyte/prism-suite-pulse)** | After Effects | Motion graphics / VFX | 🟢 v0 — **keyframe timeline** + animated preview |
| 4 | **[Reel](https://github.com/KwaminaWhyte/prism-suite-reel)** | Premiere Pro | Video editing (NLE) | 🟢 v0 — **multitrack timeline**, clips, preview (FFmpeg decode TBD) |

**All four apps exist and run**, sharing the suite engine in [`crates/`](./crates/):
**`prism-color`** (sRGB/linear, Rgba), **`prism-core`** (document/scene model, layers, blend,
tiles, shape/curve/histogram), **`prism-io`** (image/psd/exr/text/export).

**Cross-app interop:** Pigment **Dynamic-Links** a Contour `.contour` artboard — placed as a
rasterized layer that **re-renders automatically when the source file changes**. Edit in
Contour → save → Pigment updates.

## Planning & research

Every app has a parity roadmap to **≥85% of its Adobe analog** plus cited research:

| App | Plan | Research |
|---|---|---|
| Pigment | [PLAN.md](https://github.com/KwaminaWhyte/prism-suite-pigment/blob/main/PLAN.md) | [RESEARCH.md](https://github.com/KwaminaWhyte/prism-suite-pigment/blob/main/RESEARCH.md) · [ARCHITECTURE.md](https://github.com/KwaminaWhyte/prism-suite-pigment/blob/main/ARCHITECTURE.md) |
| Contour | [PLAN.md](https://github.com/KwaminaWhyte/prism-suite-contour/blob/main/PLAN.md) | [RESEARCH.md](https://github.com/KwaminaWhyte/prism-suite-contour/blob/main/RESEARCH.md) |
| Pulse | [PLAN.md](https://github.com/KwaminaWhyte/prism-suite-pulse/blob/main/PLAN.md) | [RESEARCH.md](https://github.com/KwaminaWhyte/prism-suite-pulse/blob/main/RESEARCH.md) |
| Reel | [PLAN.md](https://github.com/KwaminaWhyte/prism-suite-reel/blob/main/PLAN.md) | [RESEARCH.md](https://github.com/KwaminaWhyte/prism-suite-reel/blob/main/RESEARCH.md) |
| **Suite** | [SUITE.md](./SUITE.md) | [RESEARCH.md](./RESEARCH.md) |

Each plan grounds its phases in the app's *current* code, marks done vs planned, and tags effort
(S/M/L). Sequencing principle: build the foundation that gates breadth first, then fan out.

## Repository layout

```
prism/
├── README.md          this file
├── SUITE.md           the four-app vision + interop plan
├── pigment/           app #1 — raster editor (own git repo)
├── contour/           app #2 — vector editor (own git repo)
├── pulse/             app #3 — motion graphics (own git repo)
└── reel/              app #4 — video NLE (own git repo)
```

Each app is its own Cargo workspace + git repo. The shared engine has **already been
promoted** to suite-level `prism-*` crates in [`crates/`](./crates/) — `prism-core`'s
document model, tile/compositor, blend, color, and shape/text rasterizers — that every app
depends on by path. That shared core is the suite's whole architectural bet (see
SUITE.md §"Shared foundation" and [RESEARCH.md §2](./RESEARCH.md) for the crate matrix,
including the planned `prism-vector` / `prism-fx` / `prism-media` / `prism-ai` / `prism-doc`).

## Why a suite, not four apps

The value compounds through interop: a vector artboard placed in Pigment stays
editable; a Pulse comp dropped into a Reel timeline updates live; one color
pipeline across all four. That only works if the layer/compositor/color engine
is one codebase — which is why the apps share crates rather than reimplement.

*Foundations are free. The product is the polish — and the glue between apps.*
