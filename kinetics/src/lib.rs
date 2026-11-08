//! Create-style kinetics, independent of any game engine.
//!
//! - [`network`]: abstract graph solver (RPM propagation, stress, conflicts).
//! - [`placement`]: block-level rules that build the graph from placed blocks.

pub mod network;
pub mod placement;

pub use network::{Network, NodeId, NodeRole, Solution, Status, MAX_RPM};
pub use placement::{Axis, BlockKind, KineticBlock, KineticWorld, Pos, WorldSolution};
