//! prism-core — GPU-agnostic document/scene model shared across the Prism suite.
//!
//! This crate owns the *state*: documents, the layer tree, blend modes, the
//! sparse tile model, and (later) the command/undo stack. It deliberately knows
//! nothing about wgpu — rendering lives in `pigment-gpu`, and the app wires the
//! two together. See PLAN.md §2.

pub mod adjust;
pub mod blend;
pub mod curve;
pub mod detail;
pub mod document;
pub mod fill;
pub mod geometry;
pub mod gradient;
pub mod heal;
pub mod histogram;
pub mod inpaint;
pub mod layer;
pub mod raster;
pub mod shape;
pub mod tile;
pub mod tone;
pub mod warp;

pub use adjust::Adjustment;
pub use blend::BlendMode;
pub use document::Document;
pub use geometry::{Rect, Size};
pub use gradient::{ColorStop, Gradient, GradientType, OpacityStop};
pub use layer::{Layer, LayerId, LayerKind, LayerTree};
pub use prism_color::{self as color, Rgba};
pub use tile::{Tile, TileCoord, TILE_SIZE};
