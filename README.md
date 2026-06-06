<h1 align="center">Prism</h1>

<p align="center">
  <b>An open source creative suite</b> — four interoperating apps that work together
  the way Adobe's Creative Cloud does, built in Rust on a shared GPU engine.
</p>

---

See [SUITE.md](./SUITE.md) for the full vision (shared engine, Dynamic Link,
smart objects, common color pipeline).

## The apps

| # | App | Adobe analog | Domain | Status |
|---|-----|--------------|--------|--------|
| 1 | **[Pigment](./pigment/)** | Photoshop | Raster image editing | 🟢 Phases 0–5; can **Place `.contour`** docs |
| 2 | **[Contour](./contour/)** | Illustrator | Vector graphics | 🟢 v0 — rect/ellipse/line/**bezier pen**, **SVG/PNG export**, layers, **boolean ops** |
| 3 | **[Pulse](./pulse/)** | After Effects | Motion graphics / VFX | 🟢 v0 — **keyframe timeline** + animated preview |
| 4 | **Reel** | Premiere Pro | Video editing (NLE) | ⚪ planned |

All apps share the suite engine in [`crates/`](./crates/): **`prism-color`** (sRGB/linear,
Rgba), **`prism-core`** (document/scene model, layers, blend, tiles, shape/curve/histogram),
**`prism-io`** (image/psd/exr/text/export). First cross-app link landed: Pigment rasterizes a
Contour `.contour` artboard into a layer.

## Repository layout

```
prism/
├── README.md          this file
├── SUITE.md           the four-app vision + interop plan
├── pigment/           app #1 — raster editor (own git repo)
├── contour/           app #2 — vector editor (own git repo)
├── pulse/             app #3 — motion graphics (own git repo)
└── reel/              app #4 — video NLE (planned)
```

Each app is its own Cargo workspace + git repo for now. As the shared engine
stabilizes (`pigment-core`'s document model, tile/compositor, color, shape/text
rasterizers), the reusable parts get promoted to suite-level `prism-*` crates
that every app depends on — that shared core is the suite's whole architectural
bet (see SUITE.md §"Shared foundation").

## Why a suite, not four apps

The value compounds through interop: a vector artboard placed in Pigment stays
editable; a Pulse comp dropped into a Reel timeline updates live; one color
pipeline across all four. That only works if the layer/compositor/color engine
is one codebase — which is why the apps share crates rather than reimplement.

*Foundations are free. The product is the polish — and the glue between apps.*
