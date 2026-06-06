# Prism — UI/UX guidelines (sampled from Affinity & Adobe)

Research-backed conventions every Prism app should converge on. Sourced from
Affinity v2 / unified-Affinity (the closest free, pro, single-window reference)
and the Adobe suite. Use this as the bar when building panels and workspaces.

## What Affinity/Adobe do that we should match

1. **Studio = dockable, tabbed, collapsible panels** on the right (Affinity calls
   it the *Studio*). Panels group into tabbed stacks; each section collapses; the
   whole column scrolls. Users **show/hide** any panel from a **Window menu**
   (checkmark = visible) and **save/switch/share workspaces** ("Studio presets").
2. **Compact tool palette** on the far left — a thin icon column that can be **1 or
   more columns**; related tools nest behind a **flyout** (long-press / corner
   triangle). Never a tall ungrouped list that overflows.
3. **Contextual toolbar (tool options) across the top**, directly under the main
   toolbar — its controls change with the active tool. This is *the* place for
   per-tool settings (brush size, shape corner, stroke), NOT a giant right panel.
4. **Personas / modes** (Affinity: Vector / Pixel / Layout): one document, multiple
   tool+panel sets. Our suite-level analog is the four apps + Dynamic Link.
5. **Every tall region scrolls.** No control is ever unreachable on a short window.
6. **Reset workspace** to defaults from the Window menu.

## Adopted Prism standard (apply to all four apps)

- **Mandatory now:** any `SidePanel`/panel whose content can exceed the window
  height **must** wrap its body in `egui::ScrollArea::vertical().auto_shrink([false,false])`.
  Long property stacks use `CollapsingHeader` sections so users hide what they
  don't need. *(Done this pass: pigment left tools + right panel; contour/pulse/reel
  panels — see their commits.)*
- **Next:** a **Window menu** to toggle panel visibility; **contextual tool-options
  bar** along the top (move per-tool controls out of the right panel); tool **groups
  with flyouts** in the palette; **dark/light theme** toggle.
- **Later:** **dockable/rearrangeable** panels + **saveable/shareable workspaces**
  (the Affinity Studio-preset model); tabbed panel groups; reset-workspace.

## Per-app UI gaps (tracked in each app's PLAN `## UI/UX & workspace`)

- **Pigment** — PLAN already has *Keyboard shortcuts* + *Workspace & panels*
  (dockable/floating/collapsible, save/load workspaces, tool-options bar). Elevate:
  tool **groups/flyouts** (23 tools is too many flat), contextual options bar.
- **Contour** — Studio-style dockable panels, saveable workspaces, Window menu,
  contextual tool bar, customizable palette.
- **Pulse** — same, plus timeline-panel ergonomics; the right Properties panel was
  the reported overflow case (now scrollable + collapsible).
- **Reel** — same, plus source/program dual viewers and a dedicated Effect Controls
  panel.

## Feature-parity gaps surfaced while researching (recorded per-app PLAN)

Each app's subagent appends genuinely-missing items vs its Adobe/Affinity analog
(e.g. Contour: image trace, recolor artwork, isolation mode, mesh gradient;
Pulse: expressions, 3D/camera, adjustment layers, render queue; Reel: audio
mixing, scopes, multicam, proxy media, render-queue presets). See each PLAN.

## Sources

- [Affinity v2 UI redesign — Affinity Spotlight](https://affinityspotlight.com/article/redesigned-with-you-in-mind-the-all-new-affinity-v2-ui/)
- [Affinity Designer 2 — Get To Know The Interface (Envato Tuts+)](https://design.tutsplus.com/tutorials/affinity-designer-2-get-to-know-the-interface--cms-108766)
- [Affinity (Canva) — unified Pixel/Vector/Layout Studios](https://www.affinity.studio/)
