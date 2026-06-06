# Prism Suite — Testing Report

**Date:** 2026-06-06  
**Method:** Static analysis (`cargo test`, `cargo clippy`), deep source review of all 4 apps  
**Test run:** 153 tests across the suite — all pass  

---

## Summary

| App | Tests | Clippy warnings | Logic/Feature bugs |
|-----|-------|-----------------|--------------------|
| Pigment | 23 | 3 | 2 |
| Contour | 79 (incl. 24 doc) | 2 | 1 |
| Pulse | 0 app tests | 2 | 1 |
| Reel | 0 app tests | 1 | 0 |
| **shared crates** | 51 | 0 | 0 |

---

## Pigment (Photoshop analog — ~60%)

### Logic Bugs

**[PIG-1] `Select > None` leaves selection active**  
File: `pigment/crates/pigment-app/src/app.rs` lines 1632–1635  
```rust
// BEFORE (bug):
if ui.button("None").clicked() {
    self.sel_op_pending = Some(SelectionOp::None);
    self.selection_active = true;   // ← should be false
}
```
"Select > None" is supposed to deselect everything (marching ants disappear, painting is unrestricted). Two bugs compound:
- `app.rs`: `selection_active` is set `true` (marching ants keep animating; "Add mask from selection" stays enabled).
- `canvas.rs` line 1563: `SelectionOp::None` clears the texture to black but sets `has_selection = true`, meaning the empty mask blocks all painting.  

After this menu item the user can't paint anything.

**[PIG-2] `Select > All` hides marching ants**  
File: `pigment/crates/pigment-app/src/app.rs` line 1629  
```rust
// BEFORE (bug):
if ui.button("All").clicked() {
    self.sel_op_pending = Some(SelectionOp::All);
    self.selection_active = false;   // ← should be true
}
```
The GPU correctly removes the mask constraint (can paint everywhere) but `selection_active = false` means no marching-ants animation runs and "Add mask from selection" is grayed out. Users get no visual feedback that the whole canvas is selected.

### Clippy Warnings

**[PIG-3] `app.rs:979` — `needless_range_loop`**  
```rust
for c in 0..3 {
    bytes.extend_from_slice(&f16::from_f32(px[c] * a).to_le_bytes());
}
```
Should iterate the slice directly: `for &ch in px.iter().take(3)`.

**[PIG-4] `canvas.rs:967` — `too_many_arguments` (set_curve_lut, 8 args)**  
The function `set_curve_lut(&mut self, device, queue, id, rgb, r, g, b)` exceeds the clippy limit of 7. Suppressed with `#[allow(clippy::too_many_arguments)]` (refactoring to a param struct is disproportionate for 8 curve channels).

**[PIG-5] `theme.rs:41` — `field_reassign_with_default`**  
```rust
let mut style = Style::default();
style.visuals = visuals();   // assigned immediately after default init
```
Should use struct-update syntax: `Style { visuals: visuals(), ..Style::default() }`.

---

## Contour (Illustrator analog — ~25%)

### Feature Bug

**[CON-1] No "Open .contour…" in File menu**  
File: `contour/crates/contour-app/src/app.rs` line 380  
The File menu has "New" and "Save .contour…" but no Open item. Users can create and save documents but cannot load them back. The `save_dialog` helper already reads the file format; an `open_dialog` method can mirror it.

### Clippy Warnings

**[CON-2] `export.rs:315` — `field_reassign_with_default`**  
```rust
let mut s = Stroke::default();
s.width = stroke_w;
```
Fix: `let s = Stroke { width: stroke_w, ..Stroke::default() };`

**[CON-3] `theme.rs:41` — `field_reassign_with_default`** (same pattern as Pigment)

---

## Pulse (After Effects analog — ~10%)

### Feature Bug

**[PUL-1] Export is a permanently-disabled stub**  
File: `pulse/crates/pulse-app/src/app.rs`  
```rust
ui.add_enabled_ui(false, |ui| { let _ = ui.button("Export… (stub)"); });
```
No export path exists. Documented as a stub; tracked here for completeness.

### Clippy Warnings

**[PUL-2] `graph.rs:346` — `too_many_arguments` (draw_curve, 8 args)**  
`fn draw_curve(painter, comp, layer_idx, prop, color, dur, t_to_x, v_to_y)` — 8 arguments. Suppressed with `#[allow]` (closures can't go in a params struct cleanly).

**[PUL-3] `theme.rs:41` — `field_reassign_with_default`** (same pattern)

### Dead Code (non-critical)

`comp.rs`: `LINEAR`, `Handle`, `with_out`, `with_in`, `value_bounds`, `move_key`, `key_mut` — public API stubs for planned graph-editor features, all correct to keep.  
`icons.rs`: `TIMELINE`, `GRAPH` — icon constants not yet wired to UI.

---

## Reel (Premiere Pro analog — ~8%)

### Clippy Warnings

**[REL-1] `theme.rs:42` — `field_reassign_with_default`** (same pattern across all four apps)

### Dead Code (non-critical)

`project.rs`: `DEFAULT_TRANSITION_DUR`, `MIN_TRANSITION_DUR`, `TransitionKind::label`, `find_cut`, `add_transition` emit dead-code warnings but ARE used in `app.rs` via explicit `use` statements. These are false positives from the lint pass seeing them as unreachable from the library root; suppressing with `#[allow(dead_code)]` on each item is the correct fix.

---

## Shared Crates (`prism-core`, `prism-color`, `prism-io`)

- **51 tests, all pass** — including flood-fill, curve easing, bezier splitting, and histogram logic.  
- **0 clippy warnings** on `cargo clippy -p prism-core -p prism-color -p prism-io`.  
- No bugs found in shared code.

---

## Fixes Applied

See individual commits in each app's git repo. Summary:

| ID | File | Change |
|----|------|--------|
| PIG-1 | `pigment/.../app.rs:1634` | `selection_active = false` after "None" |
| PIG-1 | `pigment/.../canvas.rs:1563` | `SelectionOp::None` sets `has_selection = false` |
| PIG-2 | `pigment/.../app.rs:1629` | `selection_active = true` after "All" |
| PIG-3 | `pigment/.../app.rs:979` | `for &ch in px.iter().take(3)` |
| PIG-4 | `pigment/.../canvas.rs:967` | `#[allow(clippy::too_many_arguments)]` |
| PIG-5 | `pigment/.../theme.rs:39–41` | struct-update syntax |
| CON-1 | `contour/.../app.rs:384–387` | added `open_dialog()` + menu item |
| CON-2 | `contour/.../export.rs:314–315` | struct-update syntax for `Stroke` |
| CON-3 | `contour/.../theme.rs:39–41` | struct-update syntax |
| PUL-2 | `pulse/.../graph.rs:345` | `#[allow(clippy::too_many_arguments)]` |
| PUL-3 | `pulse/.../theme.rs:39–41` | struct-update syntax |
| REL-1 | `reel/.../theme.rs:39–42` | struct-update syntax |
