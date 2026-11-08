//! Block-level layer: turns placed kinetic blocks into a [`Network`] using
//! Create's connection rules (ported from `RotationPropagator`).
//!
//! Connections, in the same priority order as Create:
//! 1. **axle**: a unit step where both blocks have a shaft towards each other
//!    (`hasShaftTowards`). The ratio comes from per-face "port signs", which
//!    reproduce `getAxisModifier` (gearbox reversal, gearshift, clutch);
//! 2. **encased chain drives** next to each other along their connection axis
//!    (`ChainDriveBlock.areBlocksConnected`), ratio from adjustable chain gearshifts;
//! 3. **large cog <-> large cog** at right angles (ratio ±1);
//! 4. **large cog <-> small cog**, same axis, diagonal in the plane (large -> small = -2);
//! 5. **small cog <-> small cog**, same axis, adjacent in the plane (-1).
//!
//! Plus world-level links: belts ([`KineticWorld::link_belt`], ratio 1) and the
//! rotation speed controller driving the large cogwheel on top of it.
//!
//! Sign convention: speed is measured around the block's positive axis, like Create.
//! Split shafts (clutch, gearshift) are two nodes: one per half.

use super::network::{Network, NodeId, NodeRole, Solution, Status};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn pick(self, p: Pos) -> i32 {
        match self {
            Axis::X => p.x,
            Axis::Y => p.y,
            Axis::Z => p.z,
        }
    }

    pub fn is_horizontal(self) -> bool {
        self != Axis::Y
    }
}

/// One of the six block faces (Minecraft's `Direction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dir {
    pub axis: Axis,
    pub positive: bool,
}

impl Dir {
    pub const fn new(axis: Axis, positive: bool) -> Self {
        Self { axis, positive }
    }

    pub fn all() -> impl Iterator<Item = Dir> {
        Axis::ALL
            .into_iter()
            .flat_map(|a| [Dir::new(a, false), Dir::new(a, true)])
    }

    pub fn opposite(self) -> Self {
        Self {
            positive: !self.positive,
            ..self
        }
    }

    pub fn sign(self) -> f32 {
        if self.positive {
            1.0
        } else {
            -1.0
        }
    }

    pub fn offset(self) -> Pos {
        let v = if self.positive { 1 } else { -1 };
        match self.axis {
            Axis::X => Pos::new(v, 0, 0),
            Axis::Y => Pos::new(0, v, 0),
            Axis::Z => Pos::new(0, 0, v),
        }
    }

    /// The direction of a unit offset, if `d` is one.
    pub fn from_offset(d: Pos) -> Option<Dir> {
        if d.manhattan() != 1 {
            return None;
        }
        let axis = Axis::ALL.into_iter().find(|a| a.pick(d) != 0)?;
        Some(Dir::new(axis, axis.pick(d) > 0))
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
    pub fn manhattan(self) -> i32 {
        self.x.abs() + self.y.abs() + self.z.abs()
    }
    pub fn relative(self, d: Dir) -> Pos {
        self + d.offset()
    }
}

impl std::ops::Add for Pos {
    type Output = Pos;
    fn add(self, o: Pos) -> Pos {
        Pos::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl std::ops::Sub for Pos {
    type Output = Pos;
    fn sub(self, o: Pos) -> Pos {
        Pos::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Shaft (also: encased shaft, and anything with a plain through-axle).
    Shaft,
    Cogwheel,
    LargeCogwheel,
    /// Shafts on the four faces *perpendicular* to `axis`; straight-through
    /// rotation is reversed, like Create's gearbox.
    Gearbox,
    /// Split shaft. When `powered` the two halves are disconnected.
    Clutch {
        powered: bool,
    },
    /// Split shaft. When `powered` the second half spins in reverse.
    Gearshift {
        powered: bool,
    },
    /// Encased chain drive (formerly encased belt). Links to neighbouring chain
    /// drives along `connection`, which must be perpendicular to the block's axis.
    ChainDrive {
        connection: Axis,
    },
    /// Adjustable chain gearshift: like [`BlockKind::ChainDrive`], but speeds up
    /// what it drives through the chain by `1 + (signal + 1) / 16` when powered.
    ChainGearshift {
        connection: Axis,
        signal: u8,
    },
    /// Rotation speed controller. Input shaft along its (horizontal) `axis`; drives a
    /// large cogwheel directly above it (cog axis horizontal and != controller axis)
    /// at `target` RPM. Create clamps `target` to ±maxRotationSpeed.
    SpeedController {
        target: i32,
    },
    /// A machine facing along its axis (drill, saw, deployer: Create's
    /// `DirectionalKineticBlock` machines). Its only shaft is on its back, the face
    /// opposite the one it works on.
    Machine {
        facing_positive: bool,
    },
}

impl BlockKind {
    /// Number of network nodes the block needs.
    fn node_count(self) -> usize {
        match self {
            BlockKind::Clutch { .. } | BlockKind::Gearshift { .. } => 2,
            _ => 1,
        }
    }

    fn is_chain(self) -> bool {
        matches!(
            self,
            BlockKind::ChainDrive { .. } | BlockKind::ChainGearshift { .. }
        )
    }

    fn chain_connection(self) -> Option<Axis> {
        match self {
            BlockKind::ChainDrive { connection } | BlockKind::ChainGearshift { connection, .. } => {
                Some(connection)
            }
            _ => None,
        }
    }

    /// `ChainGearshiftBlockEntity.getModifierForSignal`.
    fn chain_modifier(self) -> f32 {
        match self {
            BlockKind::ChainGearshift { signal, .. } if signal > 0 => {
                1.0 + (signal as f32 + 1.0) / 16.0
            }
            _ => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KineticBlock {
    pub kind: BlockKind,
    pub axis: Axis,
    /// Motors/engines are `Source`, machines are `Consumer` (attach the role to the
    /// block whose axle drives / is driven by the machine). On split shafts the
    /// role goes to the negative half.
    pub role: NodeRole,
}

impl KineticBlock {
    pub fn passive(kind: BlockKind, axis: Axis) -> Self {
        Self {
            kind,
            axis,
            role: NodeRole::Passive,
        }
    }

    pub fn with_role(mut self, role: NodeRole) -> Self {
        self.role = role;
        self
    }

    /// Shaft port on `face`: (local node index, sign). `speed of the shaft on that
    /// face = sign * speed of the node`. `None` = no shaft there (`hasShaftTowards`).
    pub fn port(&self, face: Dir) -> Option<(usize, f32)> {
        use BlockKind::*;
        match self.kind {
            Gearbox => {
                if face.axis == self.axis {
                    return None;
                }
                // Reproduces GearboxBlockEntity's axis modifier: opposite faces reverse,
                // perpendicular faces reverse when both point the same way along their axes.
                let first = Axis::ALL.into_iter().find(|&a| a != self.axis).unwrap();
                let c = if face.axis == first { 1.0 } else { -1.0 };
                Some((0, face.sign() * c))
            }
            Clutch { .. } | Gearshift { .. } => {
                (face.axis == self.axis).then_some((usize::from(face.positive), 1.0))
            }
            ChainDrive { connection } | ChainGearshift { connection, .. } => {
                (face.axis == self.axis && connection != self.axis).then_some((0, 1.0))
            }
            SpeedController { .. } => {
                (face.axis == self.axis && self.axis.is_horizontal()).then_some((0, 1.0))
            }
            Machine { facing_positive } => {
                (face.axis == self.axis && face.positive != facing_positive).then_some((0, 1.0))
            }
            Shaft | Cogwheel | LargeCogwheel => (face.axis == self.axis).then_some((0, 1.0)),
        }
    }
}

/// How two blocks connect: `speed(b.node[lb]) = speed(a.node[la]) * ratio`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub la: usize,
    pub lb: usize,
    pub ratio: f32,
}

/// Connection between `a` (at `pa`) and `b` (at `pb`), or `None`.
pub fn connection(a: &KineticBlock, pa: Pos, b: &KineticBlock, pb: Pos) -> Option<Link> {
    let d = pb - pa;

    if let Some(dir) = Dir::from_offset(d) {
        // Axle <-> axle.
        if let (Some((la, sa)), Some((lb, sb))) = (a.port(dir), b.port(dir.opposite())) {
            return Some(Link {
                la,
                lb,
                ratio: sa / sb,
            });
        }
        // Encased chain drives.
        if a.kind.is_chain() && b.kind.is_chain() {
            let conn = a.kind.chain_connection()?;
            let ok = b.kind.chain_connection() == Some(conn)
                && dir.axis == conn
                && a.axis != conn
                && b.axis != conn;
            return ok.then(|| Link {
                la: 0,
                lb: 0,
                ratio: a.kind.chain_modifier() / b.kind.chain_modifier(),
            });
        }
    }

    let ratio = gear_ratio(a, b, d)?;
    Some(Link {
        la: 0,
        lb: 0,
        ratio,
    })
}

fn gear_ratio(a: &KineticBlock, b: &KineticBlock, d: Pos) -> Option<f32> {
    use BlockKind::*;
    if !matches!(a.kind, Cogwheel | LargeCogwheel) || !matches!(b.kind, Cogwheel | LargeCogwheel) {
        return None;
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
    let in_plane_diagonal =
        d.manhattan() == 2 && d.x.abs() <= 1 && d.y.abs() <= 1 && d.z.abs() <= 1;

    match (a.kind, b.kind) {
        (Cogwheel, Cogwheel) if d.manhattan() == 1 => Some(-1.0),
        (LargeCogwheel, Cogwheel) if in_plane_diagonal => Some(-2.0),
        (Cogwheel, LargeCogwheel) if in_plane_diagonal => Some(-0.5),
        _ => None,
    }
}

/// Backwards-compatible helper: ratio between the primary nodes of two blocks.
pub fn connection_ratio(a: &KineticBlock, pa: Pos, b: &KineticBlock, pb: Pos) -> Option<f32> {
    connection(a, pa, b, pb).map(|l| l.ratio)
}

fn third_axis(a: Axis, b: Axis) -> Axis {
    Axis::ALL.into_iter().find(|&c| c != a && c != b).unwrap()
}

/// Neighbour offsets worth checking: everything within a 3x3x3 cube.
fn offsets() -> impl Iterator<Item = Pos> {
    (-1..=1)
        .flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| Pos::new(x, y, z))))
        .filter(|p| *p != Pos::new(0, 0, 0))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeltError {
    MissingBlock(Pos),
    /// Belts need two shafts with the same axis, offset perpendicular to it.
    BadGeometry,
}

/// Result of solving a [`KineticWorld`], indexed by block position.
#[derive(Clone, Debug)]
pub struct WorldSolution {
    pub solution: Solution,
    /// First network node of every block (split shafts use `first` and `first + 1`).
    pub node_of: HashMap<Pos, NodeId>,
    blocks: HashMap<Pos, KineticBlock>,
}

impl WorldSolution {
    /// Speed of the block's primary node (negative half for split shafts; for a
    /// gearbox this is an internal reference, use [`Self::face_speed`]).
    pub fn speed_at(&self, p: Pos) -> Option<f32> {
        self.node_of.get(&p).map(|&n| self.solution.speed[n])
    }

    /// Speed of the shaft on `face` of the block at `p` (what a stress/speed gauge
    /// mounted there would read). `None` if there's no shaft on that face.
    pub fn face_speed(&self, p: Pos, face: Dir) -> Option<f32> {
        let (local, sign) = self.blocks.get(&p)?.port(face)?;
        Some(self.solution.speed[self.node_of[&p] + local] * sign)
    }

    pub fn status_at(&self, p: Pos) -> Option<&Status> {
        self.node_of.get(&p).map(|&n| self.solution.status_of(n))
    }

    /// (impact, capacity) in SU of the network the block belongs to.
    pub fn stress_at(&self, p: Pos) -> Option<(f32, f32)> {
        let n = *self.node_of.get(&p)?;
        let sub = &self.solution.subnetworks[self.solution.subnetwork_of[n]];
        Some((sub.impact, sub.capacity))
    }
}

/// Sparse set of kinetic blocks. In Horizon this would be kept in sync by the
/// `native_block_changed` hook and re-solved on `native_server_tick` when dirty.
#[derive(Clone, Debug)]
pub struct KineticWorld {
    blocks: BTreeMap<Pos, KineticBlock>,
    belts: Vec<(Pos, Pos)>,
    max_rpm: f32,
}

impl Default for KineticWorld {
    fn default() -> Self {
        Self {
            blocks: BTreeMap::new(),
            belts: Vec::new(),
            max_rpm: super::network::MAX_RPM,
        }
    }
}

impl KineticWorld {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create's `maxRotationSpeed` server config.
    pub fn with_max_rpm(mut self, max_rpm: f32) -> Self {
        self.max_rpm = max_rpm;
        self
    }

    pub fn place(&mut self, pos: Pos, block: KineticBlock) {
        self.blocks.insert(pos, block);
    }

    /// Removes the block and any belt attached to it (Create drops the belt too).
    pub fn remove(&mut self, pos: Pos) -> Option<KineticBlock> {
        self.belts.retain(|&(a, b)| a != pos && b != pos);
        self.blocks.remove(&pos)
    }

    pub fn get(&self, pos: Pos) -> Option<&KineticBlock> {
        self.blocks.get(&pos)
    }

    /// Mutable access, e.g. to toggle `powered` on a clutch from a redstone hook.
    pub fn get_mut(&mut self, pos: Pos) -> Option<&mut KineticBlock> {
        self.blocks.get_mut(&pos)
    }

    pub fn blocks(&self) -> impl Iterator<Item = (&Pos, &KineticBlock)> {
        self.blocks.iter()
    }

    /// Connect two shafts with a mechanical belt (ratio 1, same direction).
    pub fn link_belt(&mut self, a: Pos, b: Pos) -> Result<(), BeltError> {
        let ba = self.blocks.get(&a).ok_or(BeltError::MissingBlock(a))?;
        let bb = self.blocks.get(&b).ok_or(BeltError::MissingBlock(b))?;
        let d = b - a;
        let shaftish = |k: BlockKind| matches!(k, BlockKind::Shaft);
        if !shaftish(ba.kind)
            || !shaftish(bb.kind)
            || ba.axis != bb.axis
            || ba.axis.pick(d) != 0
            || d == Pos::new(0, 0, 0)
        {
            return Err(BeltError::BadGeometry);
        }
        self.belts.push((a, b));
        Ok(())
    }

    pub fn build_network(&self) -> (Network, HashMap<Pos, NodeId>) {
        let mut net = Network::new().with_max_rpm(self.max_rpm);
        let mut node_of = HashMap::with_capacity(self.blocks.len());
        for (&p, b) in &self.blocks {
            let first = net.add_node(b.role);
            for _ in 1..b.kind.node_count() {
                net.add_node(NodeRole::Passive);
            }
            node_of.insert(p, first);
            // internal link between split-shaft halves
            match b.kind {
                BlockKind::Clutch { powered: false } | BlockKind::Gearshift { powered: false } => {
                    net.connect(first, first + 1, 1.0)
                }
                BlockKind::Gearshift { powered: true } => net.connect(first, first + 1, -1.0),
                _ => {}
            }
        }
        for (&pa, a) in &self.blocks {
            for off in offsets() {
                let pb = pa + off;
                // each unordered pair once
                if pb <= pa {
                    continue;
                }
                if let Some(b) = self.blocks.get(&pb) {
                    if let Some(l) = connection(a, pa, b, pb) {
                        net.connect(node_of[&pa] + l.la, node_of[&pb] + l.lb, l.ratio);
                    }
                }
            }
            // Rotation speed controller -> large cogwheel above.
            if let BlockKind::SpeedController { target } = a.kind {
                let up = pa + Pos::new(0, 1, 0);
                if let Some(cog) = self.blocks.get(&up) {
                    if cog.kind == BlockKind::LargeCogwheel
                        && cog.axis.is_horizontal()
                        && a.axis.is_horizontal()
                        && cog.axis != a.axis
                    {
                        let t = (target as f32).clamp(-self.max_rpm, self.max_rpm);
                        net.connect_controlled(node_of[&pa], node_of[&up], t);
                    }
                }
            }
        }
        for &(a, b) in &self.belts {
            net.connect(node_of[&a], node_of[&b], 1.0);
        }
        (net, node_of)
    }

    pub fn solve(&self) -> WorldSolution {
        let (net, node_of) = self.build_network();
        WorldSolution {
            solution: net.solve(),
            node_of,
            blocks: self.blocks.iter().map(|(&p, &b)| (p, b)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlockKind::*;

    const P0: Pos = Pos::new(0, 0, 0);

    fn motor(axis: Axis, rpm: f32) -> KineticBlock {
        KineticBlock {
            kind: Shaft,
            axis,
            role: NodeRole::Source {
                rpm,
                capacity: 16.0,
            },
        }
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
        w.place(
            P0,
            KineticBlock {
                kind: Cogwheel,
                ..motor(Axis::Z, 16.0)
            },
        );
        w.place(Pos::new(1, 0, 0), KineticBlock::passive(Cogwheel, Axis::Z));
        w.place(Pos::new(1, 1, 0), KineticBlock::passive(Cogwheel, Axis::Z));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 0, 0)), Some(-16.0));
        assert_eq!(s.speed_at(Pos::new(1, 1, 0)), Some(16.0));
    }

    #[test]
    fn large_to_small_doubles_speed() {
        let mut w = KineticWorld::new();
        w.place(
            P0,
            KineticBlock {
                kind: LargeCogwheel,
                ..motor(Axis::Y, 32.0)
            },
        );
        w.place(Pos::new(1, 0, 1), KineticBlock::passive(Cogwheel, Axis::Y));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 0, 1)), Some(-64.0));
    }

    #[test]
    fn small_to_large_halves_speed() {
        let mut w = KineticWorld::new();
        w.place(
            P0,
            KineticBlock {
                kind: Cogwheel,
                ..motor(Axis::Y, 32.0)
            },
        );
        w.place(
            Pos::new(-1, 0, 1),
            KineticBlock::passive(LargeCogwheel, Axis::Y),
        );
        assert_eq!(w.solve().speed_at(Pos::new(-1, 0, 1)), Some(-16.0));
    }

    #[test]
    fn large_cogs_at_right_angle() {
        let mut w = KineticWorld::new();
        w.place(
            P0,
            KineticBlock {
                kind: LargeCogwheel,
                ..motor(Axis::X, 32.0)
            },
        );
        w.place(
            Pos::new(1, 1, 0),
            KineticBlock::passive(LargeCogwheel, Axis::Y),
        );
        w.place(
            Pos::new(1, -1, 0),
            KineticBlock::passive(LargeCogwheel, Axis::Y),
        );
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
        w.place(
            P0,
            KineticBlock {
                kind: LargeCogwheel,
                ..motor(Axis::Y, 16.0)
            },
        );
        let machine_axle = Pos::new(1, 0, 1);
        w.place(
            machine_axle,
            KineticBlock {
                kind: Cogwheel,
                axis: Axis::Y,
                role: NodeRole::Consumer { impact: 8.0 },
            },
        );
        assert_eq!(w.solve().status_at(machine_axle), Some(&Status::Running));

        // add a second identical machine on the same network -> overstressed
        w.place(
            Pos::new(1, 1, 1),
            KineticBlock {
                kind: Shaft,
                axis: Axis::Y,
                role: NodeRole::Consumer { impact: 8.0 },
            },
        );
        assert_eq!(
            w.solve().status_at(machine_axle),
            Some(&Status::Overstressed)
        );
    }

    fn shaft(axis: Axis) -> KineticBlock {
        KineticBlock::passive(Shaft, axis)
    }

    #[test]
    fn gearbox_reverses_straight_through_and_turns_corners() {
        // motor shaft on X feeding a Y-axis gearbox (shafts on X and Z faces)
        let mut w = KineticWorld::new();
        w.place(Pos::new(-1, 0, 0), motor(Axis::X, 32.0));
        w.place(P0, KineticBlock::passive(Gearbox, Axis::Y));
        w.place(Pos::new(1, 0, 0), shaft(Axis::X));
        w.place(Pos::new(0, 0, 1), shaft(Axis::Z));
        w.place(Pos::new(0, 0, -1), shaft(Axis::Z));
        w.place(Pos::new(0, 1, 0), shaft(Axis::Y)); // no shaft on the gearbox's own axis
        let s = w.solve();
        // straight through: reversed
        assert_eq!(s.speed_at(Pos::new(1, 0, 0)), Some(-32.0));
        // input from -X (negative), +Z is positive -> different directions -> +1
        assert_eq!(s.speed_at(Pos::new(0, 0, 1)), Some(32.0));
        // -Z: same axis direction as the -X input -> -1
        assert_eq!(s.speed_at(Pos::new(0, 0, -1)), Some(-32.0));
        assert_eq!(s.speed_at(Pos::new(0, 1, 0)), Some(0.0));
        // gauge on the input face reads the motor speed
        assert_eq!(s.face_speed(P0, Dir::new(Axis::X, false)), Some(32.0));
        assert_eq!(s.face_speed(P0, Dir::new(Axis::Y, true)), None);
    }

    #[test]
    fn two_gearboxes_back_to_back_restore_direction() {
        let mut w = KineticWorld::new();
        w.place(P0, motor(Axis::X, 16.0));
        w.place(Pos::new(1, 0, 0), KineticBlock::passive(Gearbox, Axis::Y));
        w.place(Pos::new(2, 0, 0), KineticBlock::passive(Gearbox, Axis::Z));
        w.place(Pos::new(3, 0, 0), shaft(Axis::X));
        assert_eq!(w.solve().speed_at(Pos::new(3, 0, 0)), Some(16.0));
    }

    #[test]
    fn clutch_disconnects_when_powered() {
        let mut w = KineticWorld::new();
        let out = Pos::new(2, 0, 0);
        w.place(P0, motor(Axis::X, 32.0));
        w.place(
            Pos::new(1, 0, 0),
            KineticBlock::passive(Clutch { powered: false }, Axis::X),
        );
        w.place(out, shaft(Axis::X));
        assert_eq!(w.solve().speed_at(out), Some(32.0));

        w.get_mut(Pos::new(1, 0, 0)).unwrap().kind = Clutch { powered: true };
        let s = w.solve();
        assert_eq!(s.speed_at(out), Some(0.0));
        assert_eq!(s.status_at(out), Some(&Status::Stopped));
        // the driven half still turns
        assert_eq!(
            s.face_speed(Pos::new(1, 0, 0), Dir::new(Axis::X, false)),
            Some(32.0)
        );
        assert_eq!(
            s.face_speed(Pos::new(1, 0, 0), Dir::new(Axis::X, true)),
            Some(0.0)
        );
    }

    #[test]
    fn powered_clutch_works_from_either_side() {
        let mut w = KineticWorld::new();
        w.place(P0, shaft(Axis::X));
        w.place(
            Pos::new(1, 0, 0),
            KineticBlock::passive(Clutch { powered: true }, Axis::X),
        );
        w.place(Pos::new(2, 0, 0), motor(Axis::X, 8.0));
        let s = w.solve();
        assert_eq!(s.speed_at(P0), Some(0.0));
        assert_eq!(s.speed_at(Pos::new(2, 0, 0)), Some(8.0));
    }

    #[test]
    fn gearshift_reverses_when_powered() {
        let mut w = KineticWorld::new();
        let out = Pos::new(2, 0, 0);
        w.place(P0, motor(Axis::X, 32.0));
        w.place(
            Pos::new(1, 0, 0),
            KineticBlock::passive(Gearshift { powered: false }, Axis::X),
        );
        w.place(out, shaft(Axis::X));
        assert_eq!(w.solve().speed_at(out), Some(32.0));
        w.get_mut(Pos::new(1, 0, 0)).unwrap().kind = Gearshift { powered: true };
        assert_eq!(w.solve().speed_at(out), Some(-32.0));
    }

    #[test]
    fn chain_drives_relay_sideways() {
        // three X-axis chain drives stacked along Y; the last drives a shaft on X
        let mut w = KineticWorld::new();
        let chain = |signal| {
            KineticBlock::passive(
                ChainGearshift {
                    connection: Axis::Y,
                    signal,
                },
                Axis::X,
            )
        };
        w.place(Pos::new(-1, 0, 0), motor(Axis::X, 16.0));
        w.place(
            P0,
            KineticBlock::passive(
                ChainDrive {
                    connection: Axis::Y,
                },
                Axis::X,
            ),
        );
        w.place(Pos::new(0, 1, 0), chain(0));
        w.place(
            Pos::new(0, 2, 0),
            KineticBlock::passive(
                ChainDrive {
                    connection: Axis::Y,
                },
                Axis::X,
            ),
        );
        w.place(Pos::new(1, 2, 0), shaft(Axis::X));
        // wrong connection axis: not linked
        w.place(
            Pos::new(0, -1, 0),
            KineticBlock::passive(
                ChainDrive {
                    connection: Axis::Z,
                },
                Axis::X,
            ),
        );
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(1, 2, 0)), Some(16.0));
        assert_eq!(s.speed_at(Pos::new(0, -1, 0)), Some(0.0));

        // powered adjustable chain gearshift (signal 15): what it drives runs x2
        w.place(Pos::new(0, 1, 0), chain(15));
        let s = w.solve();
        assert_eq!(s.speed_at(Pos::new(0, 1, 0)), Some(8.0)); // driven: 16 / 2
        assert_eq!(s.speed_at(Pos::new(1, 2, 0)), Some(16.0)); // 8 * 2
    }

    #[test]
    fn chain_gearshift_modifier_matches_create() {
        assert_eq!(
            BlockKind::ChainGearshift {
                connection: Axis::Y,
                signal: 0
            }
            .chain_modifier(),
            1.0
        );
        assert_eq!(
            BlockKind::ChainGearshift {
                connection: Axis::Y,
                signal: 7
            }
            .chain_modifier(),
            1.5
        );
        assert_eq!(
            BlockKind::ChainGearshift {
                connection: Axis::Y,
                signal: 15
            }
            .chain_modifier(),
            2.0
        );
    }

    #[test]
    fn belt_links_parallel_shafts() {
        let mut w = KineticWorld::new();
        w.place(P0, motor(Axis::Z, 24.0));
        w.place(Pos::new(5, 1, 0), shaft(Axis::Z));
        w.link_belt(P0, Pos::new(5, 1, 0)).unwrap();
        assert_eq!(w.solve().speed_at(Pos::new(5, 1, 0)), Some(24.0));

        w.place(Pos::new(0, 3, 0), shaft(Axis::X));
        assert_eq!(
            w.link_belt(P0, Pos::new(0, 3, 0)),
            Err(BeltError::BadGeometry)
        );
        assert_eq!(
            w.link_belt(P0, Pos::new(9, 9, 9)),
            Err(BeltError::MissingBlock(Pos::new(9, 9, 9)))
        );

        w.remove(P0);
        w.place(P0, shaft(Axis::Z));
        assert_eq!(w.solve().speed_at(Pos::new(5, 1, 0)), Some(0.0)); // belt dropped
    }

    #[test]
    fn speed_controller_sets_cog_speed() {
        let mut w = KineticWorld::new();
        let ctrl = Pos::new(1, 0, 0);
        let cog = Pos::new(1, 1, 0);
        w.place(P0, motor(Axis::X, 16.0));
        w.place(
            ctrl,
            KineticBlock::passive(SpeedController { target: 200 }, Axis::X),
        );
        w.place(cog, KineticBlock::passive(LargeCogwheel, Axis::Z));
        w.place(Pos::new(2, 2, 0), KineticBlock::passive(Cogwheel, Axis::Z));
        // 200 RPM on the large cog = 400 RPM on the small one -> the whole network breaks
        let s = w.solve();
        assert!(matches!(s.status_at(ctrl), Some(Status::Overspeed { .. })));
        assert_eq!(s.speed_at(cog), Some(0.0));

        w.get_mut(ctrl).unwrap().kind = SpeedController { target: 64 };
        let s = w.solve();
        assert_eq!(s.speed_at(cog), Some(64.0));
        assert_eq!(s.speed_at(Pos::new(2, 2, 0)), Some(-128.0));
        assert_eq!(s.speed_at(P0), Some(16.0));

        // cog axis parallel to the controller axis: no link
        w.place(cog, KineticBlock::passive(LargeCogwheel, Axis::X));
        assert_eq!(w.solve().speed_at(cog), Some(0.0));
    }

    #[test]
    fn stress_is_reported_per_block() {
        let mut w = KineticWorld::new();
        w.place(P0, motor(Axis::X, 16.0));
        w.place(
            Pos::new(1, 0, 0),
            shaft(Axis::X).with_role(NodeRole::Consumer { impact: 4.0 }),
        );
        assert_eq!(w.solve().stress_at(Pos::new(1, 0, 0)), Some((64.0, 256.0)));
    }

    #[test]
    fn machine_takes_rotation_only_from_its_back() {
        // drill facing +X at (1,0,0): shaft behind it at (0,0,0) connects, the block in
        // front (2,0,0) doesn't
        let mut w = KineticWorld::new();
        let drill = Pos::new(1, 0, 0);
        w.place(P0, motor(Axis::X, 32.0));
        w.place(
            drill,
            KineticBlock::passive(
                Machine {
                    facing_positive: true,
                },
                Axis::X,
            )
            .with_role(NodeRole::Consumer { impact: 4.0 }),
        );
        w.place(Pos::new(2, 0, 0), shaft(Axis::X));
        let s = w.solve();
        assert_eq!(s.speed_at(drill), Some(32.0));
        assert_eq!(s.speed_at(Pos::new(2, 0, 0)), Some(0.0));
        assert_eq!(s.stress_at(drill), Some((128.0, 512.0)));

        // turned around, its back faces away from the motor
        w.place(
            drill,
            KineticBlock::passive(
                Machine {
                    facing_positive: false,
                },
                Axis::X,
            ),
        );
        assert_eq!(w.solve().speed_at(drill), Some(0.0));
    }
}
