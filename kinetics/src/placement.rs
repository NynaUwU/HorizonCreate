//! Block-level layer: turns placed kinetic blocks into a [`Network`] using
//! Create's connection rules (modelled on `RotationPropagator`).
//!
//! Supported so far:
//! - axle-through: any kinetic block connects to a neighbour on its own axis
//!   if that neighbour has the same axis (ratio +1);
//! - small cog <-> small cog: same axis, orthogonally adjacent in the plane (ratio -1);
//! - large cog <-> small cog: same axis, diagonal in the plane (large -> small = -2);
//! - large cog <-> large cog: perpendicular axes, diagonal offset (ratio ±1).
//!
//! TODO (next): gearbox, encased shafts/belts, clutch/gearshift, speed controller.
//!
//! Sign convention: speed is measured around the block's positive axis, like Create.
//! Ratios for meshing gears should be double-checked in-game against Create 6.x.

use crate::network::{Network, NodeId, NodeRole, Solution, Status};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn pick(self, p: Pos) -> i32 {
        match self {
            Axis::X => p.x,
            Axis::Y => p.y,
            Axis::Z => p.z,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Pos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
    fn sub(self, o: Pos) -> Pos {
        Pos::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn manhattan(self) -> i32 {
        self.x.abs() + self.y.abs() + self.z.abs()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Shaft,
    Cogwheel,
    LargeCogwheel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KineticBlock {
    pub kind: BlockKind,
    pub axis: Axis,
    /// Motors/engines are `Source`, machines are `Consumer` (attach the role to the
    /// block whose axle drives / is driven by the machine).
    pub role: NodeRole,
}

impl KineticBlock {
    pub fn passive(kind: BlockKind, axis: Axis) -> Self {
        Self { kind, axis, role: NodeRole::Passive }
    }
}

/// Speed modifier from `a` (at `pa`) to `b` (at `pb`), or `None` if not connected.
pub fn connection_ratio(a: &KineticBlock, pa: Pos, b: &KineticBlock, pb: Pos) -> Option<f32> {
    use BlockKind::*;
    let d = pb.sub(pa);

    // Axle through: unit step along the shared axis.
    if a.axis == b.axis && d.manhattan() == 1 && a.axis.pick(d).abs() == 1 {
        return Some(1.0);
    }

    // Large <-> large at right angles (Create: isLargeToLargeGear).
    if a.kind == LargeCogwheel && b.kind == LargeCogwheel && a.axis != b.axis {
        let third = third_axis(a.axis, b.axis);
        let (da, db) = (a.axis.pick(d), b.axis.pick(d));
        if third.pick(d) == 0 && da.abs() == 1 && db.abs() == 1 {
            return Some(if (da > 0) ^ (db > 0) { -1.0 } else { 1.0 });
        }
        return None;
    }

    if a.axis != b.axis || a.axis.pick(d) != 0 {
        return None;
    }
    let in_plane_diagonal = d.manhattan() == 2 && d.x.abs() <= 1 && d.y.abs() <= 1 && d.z.abs() <= 1;

    match (a.kind, b.kind) {
        (Cogwheel, Cogwheel) if d.manhattan() == 1 => Some(-1.0),
        (LargeCogwheel, Cogwheel) if in_plane_diagonal => Some(-2.0),
        (Cogwheel, LargeCogwheel) if in_plane_diagonal => Some(-0.5),
        _ => None,
    }
}

fn third_axis(a: Axis, b: Axis) -> Axis {
    [Axis::X, Axis::Y, Axis::Z].into_iter().find(|&c| c != a && c != b).unwrap()
}

/// Neighbour offsets worth checking: everything within a 3x3x3 cube.
fn offsets() -> impl Iterator<Item = Pos> {
    (-1..=1).flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| Pos::new(x, y, z))))
        .filter(|p| *p != Pos::new(0, 0, 0))
}

/// Result of solving a [`KineticWorld`], indexed by block position.
#[derive(Clone, Debug)]
pub struct WorldSolution {
    pub solution: Solution,
    pub node_of: HashMap<Pos, NodeId>,
}

impl WorldSolution {
    pub fn speed_at(&self, p: Pos) -> Option<f32> {
        self.node_of.get(&p).map(|&n| self.solution.speed[n])
    }
    pub fn status_at(&self, p: Pos) -> Option<&Status> {
        self.node_of.get(&p).map(|&n| self.solution.status_of(n))
    }
}

/// Sparse set of kinetic blocks. In Horizon this would be kept in sync by the
/// `native_block_changed` hook and re-solved on `native_server_tick` when dirty.
#[derive(Clone, Debug, Default)]
pub struct KineticWorld {
    blocks: BTreeMap<Pos, KineticBlock>,
}

impl KineticWorld {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn place(&mut self, pos: Pos, block: KineticBlock) {
        self.blocks.insert(pos, block);
    }

    pub fn remove(&mut self, pos: Pos) -> Option<KineticBlock> {
        self.blocks.remove(&pos)
    }

    pub fn get(&self, pos: Pos) -> Option<&KineticBlock> {
        self.blocks.get(&pos)
    }

    pub fn build_network(&self) -> (Network, HashMap<Pos, NodeId>) {
        let mut net = Network::new();
        let mut node_of = HashMap::with_capacity(self.blocks.len());
        for (&p, b) in &self.blocks {
            node_of.insert(p, net.add_node(b.role));
        }
        for (&pa, a) in &self.blocks {
            for off in offsets() {
                let pb = Pos::new(pa.x + off.x, pa.y + off.y, pa.z + off.z);
                // each unordered pair once
                if pb <= pa {
                    continue;
                }
                if let Some(b) = self.blocks.get(&pb) {
                    if let Some(r) = connection_ratio(a, pa, b, pb) {
                        net.connect(node_of[&pa], node_of[&pb], r);
                    }
                }
            }
        }
        (net, node_of)
    }

    pub fn solve(&self) -> WorldSolution {
        let (net, node_of) = self.build_network();
        WorldSolution { solution: net.solve(), node_of }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlockKind::*;

    const P0: Pos = Pos::new(0, 0, 0);

    fn motor(axis: Axis, rpm: f32) -> KineticBlock {
        KineticBlock { kind: Shaft, axis, role: NodeRole::Source { rpm, capacity: 16.0 } }
    }

    #[test]
    fn shaft_line_on_axis() {
        let mut w = KineticWorld::new();
        w.place(P0, motor(Axis::X, 32.0));
        for x in 1..5 {
            w.place(Pos::new(x, 0, 0), KineticBlock::passive(Shaft, Axis::X));
        }
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(4, 0, 0)), Some(32.0));
    }

    #[test]
    fn shafts_with_wrong_axis_do_not_connect() {
        let mut w = KineticWorld::new();
        w.place(P0, motor(Axis::X, 32.0));
        w.place(Pos::new(1, 0, 0), KineticBlock::passive(Shaft, Axis::Y));
        // side-by-side shafts (not on the axis) don't connect either
        w.place(Pos::new(0, 1, 0), KineticBlock::passive(Shaft, Axis::X));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 0, 0)), Some(0.0));
        assert_eq!(s.speed_at(Pos::new(0, 1, 0)), Some(0.0));
    }

    #[test]
    fn small_cogs_reverse_direction() {
        let mut w = KineticWorld::new();
        w.place(P0, KineticBlock { kind: Cogwheel, ..motor(Axis::Z, 16.0) });
        w.place(Pos::new(1, 0, 0), KineticBlock::passive(Cogwheel, Axis::Z));
        w.place(Pos::new(1, 1, 0), KineticBlock::passive(Cogwheel, Axis::Z));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 0, 0)), Some(-16.0));
        assert_eq!(s.speed_at(Pos::new(1, 1, 0)), Some(16.0));
    }

    #[test]
    fn large_to_small_doubles_speed() {
        let mut w = KineticWorld::new();
        w.place(P0, KineticBlock { kind: LargeCogwheel, ..motor(Axis::Y, 32.0) });
        w.place(Pos::new(1, 0, 1), KineticBlock::passive(Cogwheel, Axis::Y));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 0, 1)), Some(-64.0));
    }

    #[test]
    fn small_to_large_halves_speed() {
        let mut w = KineticWorld::new();
        w.place(P0, KineticBlock { kind: Cogwheel, ..motor(Axis::Y, 32.0) });
        w.place(Pos::new(-1, 0, 1), KineticBlock::passive(LargeCogwheel, Axis::Y));
        assert_eq!(w.solve().speed_at(Pos::new(-1, 0, 1)), Some(-16.0));
    }

    #[test]
    fn large_cogs_at_right_angle() {
        let mut w = KineticWorld::new();
        w.place(P0, KineticBlock { kind: LargeCogwheel, ..motor(Axis::X, 32.0) });
        w.place(Pos::new(1, 1, 0), KineticBlock::passive(LargeCogwheel, Axis::Y));
        w.place(Pos::new(1, -1, 0), KineticBlock::passive(LargeCogwheel, Axis::Y));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 1, 0)), Some(32.0));
        assert_eq!(s.speed_at(Pos::new(1, -1, 0)), Some(-32.0));
    }

    #[test]
    fn removing_a_block_splits_the_network() {
        let mut w = KineticWorld::new();
        w.place(P0, motor(Axis::X, 32.0));
        w.place(Pos::new(1, 0, 0), KineticBlock::passive(Shaft, Axis::X));
        w.place(Pos::new(2, 0, 0), KineticBlock::passive(Shaft, Axis::X));
        w.remove(Pos::new(1, 0, 0));
        let s = w.solve();
        assert_eq!(s.status_at(Pos::new(2, 0, 0)), Some(&Status::Stopped));
    }

    #[test]
    fn machine_overstresses_through_gearing() {
        // motor 16 RPM * 16 SU/RPM = 256 SU; small cog spins at 32 RPM;
        // a consumer of 8 SU/RPM at 32 RPM = 256 SU -> exactly fits.
        let mut w = KineticWorld::new();
        w.place(P0, KineticBlock { kind: LargeCogwheel, ..motor(Axis::Y, 16.0) });
        let machine_axle = Pos::new(1, 0, 1);
        w.place(machine_axle, KineticBlock {
            kind: Cogwheel,
            axis: Axis::Y,
            role: NodeRole::Consumer { impact: 8.0 },
        });
        assert_eq!(w.solve().status_at(machine_axle), Some(&Status::Running));

        // add a second identical machine on the same network -> overstressed
        w.place(Pos::new(1, 1, 1), KineticBlock {
            kind: Shaft,
            axis: Axis::Y,
            role: NodeRole::Consumer { impact: 8.0 },
        });
        assert_eq!(w.solve().status_at(machine_axle), Some(&Status::Overstressed));
    }
}
