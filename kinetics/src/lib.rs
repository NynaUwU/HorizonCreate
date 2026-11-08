//! Create-style kinetics, independent of any game engine.
//!
//! - [`network`]: abstract graph solver (RPM propagation, stress, conflicts, speed controllers).
//! - [`placement`]: block-level rules that build the graph from placed blocks.
//! - [`presets`]: Create's default stress values.

pub mod network;
pub mod placement;
pub mod presets;

pub use network::{Network, NodeId, NodeRole, Solution, Status, Subnetwork, MAX_RPM};
pub use placement::{
    connection, Axis, BeltError, BlockKind, Dir, KineticBlock, KineticWorld, Link, Pos,
    WorldSolution,
};
