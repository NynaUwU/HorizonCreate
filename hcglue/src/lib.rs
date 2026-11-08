//! What the `horizon_create` native mod does, without Horizon types, so it can be tested here.
//!
//! Flow on the server, once per level tick pass (Horizon hook points in brackets):
//!
//! 1. [`Engine::begin_level`] (`level_tick_pre`): start collecting this level's blocks;
//! 2. for each of our block entities that ticks (`block_entity_tick`):
//!    [`block_from_state`] turns the block's id, properties and redstone signal into a
//!    [`KineticBlock`], [`Engine::report`] records it, and [`Engine::readout`] gives the
//!    values from the last solve to store in the block entity (sent to clients);
//! 3. [`Engine::end_level`] (`level_tick_post`): blocks that didn't report are gone
//!    (broken or unloaded), chain drives get their connection axis from their neighbours,
//!    and the network is solved again if anything changed.
//!
//! On the client, [`Spinner`] turns a readout's RPM into `BlockEntityDraw::spin`
//! arguments without the part jumping when the speed changes.

pub mod drill;
pub mod setup;

use kinetics::presets::{capacity, impact};
use kinetics::{Axis, BlockKind, KineticBlock, KineticWorld, NodeRole, Pos, Status, WorldSolution};
use std::collections::{HashMap, HashSet};

pub const MOD_ID: &str = "horizon_create";

/// Every block of the mod that takes part in kinetics (each gets a block entity).
pub const KINETIC_BLOCKS: &[&str] = &[
    "shaft",
    "cogwheel",
    "large_cogwheel",
    "gearbox",
    "clutch",
    "gearshift",
    "encased_chain_drive",
    "adjustable_chain_gearshift",
    "rotation_speed_controller",
    "hand_crank",
    "creative_motor",
    "speedometer",
    "stressometer",
    "mechanical_drill",
];

/// Blocks drawn turning by the block entity (their block model is left out:
/// `render_shape: ENTITYBLOCK_ANIMATED`). Value: the standalone model drawn.
pub const SPINNING_MODELS: &[(&str, &str)] = &[
    ("shaft", "horizon_create:block/shaft"),
    ("cogwheel", "horizon_create:block/cogwheel"),
    ("large_cogwheel", "horizon_create:block/large_cogwheel"),
];

/// Hand crank output (Create: 32 RPM, 8 SU/RPM).
pub const CRANK_RPM: f32 = 32.0;

/// The id without our namespace, if it's one of ours.
pub fn local_name(id: &str) -> Option<&str> {
    let name = id.strip_prefix(MOD_ID)?.strip_prefix(':')?;
    KINETIC_BLOCKS.contains(&name).then_some(name)
}

fn axis_of(v: &str) -> Option<Axis> {
    match v {
        "x" => Some(Axis::X),
        "y" => Some(Axis::Y),
        "z" => Some(Axis::Z),
        _ => None,
    }
}

/// Redstone signal 0..15 -> RPM: 16 RPM unpowered, +16 per level, 256 at 15.
/// Used by the creative motor and the speed controller, which have no scroll
/// value panel in Horizon yet.
pub fn signal_rpm(signal: i32) -> i32 {
    16 * (1 + signal.clamp(0, 15))
}

/// Map one of our blocks to its kinetic definition. `prop` reads a block state
/// property; `signal` is the strongest redstone signal reaching the block.
pub fn block_from_state(
    id: &str,
    prop: &dyn Fn(&str) -> Option<String>,
    signal: i32,
) -> Option<KineticBlock> {
    let name = local_name(id)?;
    let axis = || prop("axis").as_deref().and_then(axis_of);
    let powered = signal > 0;
    let b = match name {
        "shaft" | "speedometer" | "stressometer" => {
            KineticBlock::passive(BlockKind::Shaft, axis()?)
        }
        "cogwheel" => KineticBlock::passive(BlockKind::Cogwheel, axis()?),
        "large_cogwheel" => KineticBlock::passive(BlockKind::LargeCogwheel, axis()?),
        "gearbox" => KineticBlock::passive(BlockKind::Gearbox, axis()?),
        "clutch" => KineticBlock::passive(BlockKind::Clutch { powered }, axis()?),
        "gearshift" => KineticBlock::passive(BlockKind::Gearshift { powered }, axis()?),
        // the connection axis is resolved from neighbours in `end_level`
        "encased_chain_drive" => {
            let a = axis()?;
            KineticBlock::passive(
                BlockKind::ChainDrive {
                    connection: default_connection(a),
                },
                a,
            )
        }
        "adjustable_chain_gearshift" => {
            let a = axis()?;
            let signal = signal.clamp(0, 15) as u8;
            KineticBlock::passive(
                BlockKind::ChainGearshift {
                    connection: default_connection(a),
                    signal,
                },
                a,
            )
        }
        // lightning rod template: `facing` is the side it drills
        "mechanical_drill" => {
            let (axis, positive) = match prop("facing").as_deref()? {
                "east" => (Axis::X, true),
                "west" => (Axis::X, false),
                "up" => (Axis::Y, true),
                "down" => (Axis::Y, false),
                "south" => (Axis::Z, true),
                "north" => (Axis::Z, false),
                _ => return None,
            };
            KineticBlock::passive(
                BlockKind::Machine {
                    facing_positive: positive,
                },
                axis,
            )
            .with_role(NodeRole::Consumer {
                impact: impact::MECHANICAL_DRILL,
            })
        }
        "rotation_speed_controller" => KineticBlock::passive(
            BlockKind::SpeedController {
                target: signal_rpm(signal),
            },
            axis()?,
        ),
        "creative_motor" => {
            KineticBlock::passive(BlockKind::Shaft, axis()?).with_role(NodeRole::Source {
                rpm: signal_rpm(signal) as f32,
                capacity: capacity::CREATIVE_MOTOR,
            })
        }
        // lever template: `face` floor/ceiling/wall, `facing` n/s/e/w, `powered` = turning
        "hand_crank" => {
            let a = match prop("face").as_deref() {
                Some("wall") => match prop("facing").as_deref()? {
                    "north" | "south" => Axis::Z,
                    _ => Axis::X,
                },
                _ => Axis::Y,
            };
            let turning = prop("powered").as_deref() == Some("true");
            let rpm = if turning { CRANK_RPM } else { 0.0 };
            KineticBlock::passive(BlockKind::Shaft, a).with_role(NodeRole::Source {
                rpm,
                capacity: capacity::HAND_CRANK,
            })
        }
        _ => return None,
    };
    Some(b)
}

fn default_connection(axis: Axis) -> Axis {
    Axis::ALL.into_iter().find(|&a| a != axis).unwrap()
}

/// What a block entity stores for clients: its speed and its network's stress.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Readout {
    pub rpm: f32,
    pub impact: f32,
    pub capacity: f32,
    pub status: u8,
}

pub mod status {
    pub const STOPPED: u8 = 0;
    pub const RUNNING: u8 = 1;
    pub const OVERSTRESSED: u8 = 2;
    pub const CONFLICT: u8 = 3;
    pub const OVERSPEED: u8 = 4;
}

fn status_code(s: &Status) -> u8 {
    match s {
        Status::Stopped => status::STOPPED,
        Status::Running => status::RUNNING,
        Status::Overstressed => status::OVERSTRESSED,
        Status::Conflict { .. } => status::CONFLICT,
        Status::Overspeed { .. } => status::OVERSPEED,
    }
}

#[derive(Default)]
struct Level {
    world: KineticWorld,
    seen: HashSet<Pos>,
    solution: Option<WorldSolution>,
    dirty: bool,
}

/// All levels' kinetic networks. One per server (keep it in a `Mutex`).
#[derive(Default)]
pub struct Engine {
    levels: HashMap<String, Level>,
    current: Option<String>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_level(&mut self, dimension: &str) {
        let level = self.levels.entry(dimension.to_owned()).or_default();
        level.seen.clear();
        self.current = Some(dimension.to_owned());
    }

    fn level(&mut self) -> Option<&mut Level> {
        let d = self.current.as_ref()?;
        self.levels.get_mut(d)
    }

    /// A block entity of ours ticked at `pos` with this definition (`None` when the
    /// state couldn't be read: the block counts as gone).
    pub fn report(&mut self, pos: [i32; 3], block: Option<KineticBlock>) {
        let Some(level) = self.level() else { return };
        let p = Pos::new(pos[0], pos[1], pos[2]);
        let Some(mut block) = block else { return };
        // keep the connection axis `end_level` resolved for this chain drive
        let resolved = level
            .world
            .get(p)
            .and_then(|old| old.kind.chain_connection());
        if let (Some(c), Some(slot)) = (resolved, block.kind.chain_connection_mut()) {
            *slot = c;
        }
        level.seen.insert(p);
        if level.world.get(p) != Some(&block) {
            level.world.place(p, block);
            level.dirty = true;
        }
    }

    /// Readout from the last solve, for the block at `pos` in the current level.
    pub fn readout(&mut self, pos: [i32; 3]) -> Readout {
        let Some(level) = self.level() else {
            return Readout::default();
        };
        let p = Pos::new(pos[0], pos[1], pos[2]);
        let Some(sol) = &level.solution else {
            return Readout::default();
        };
        let (Some(rpm), Some(st), Some((impact, capacity))) =
            (sol.speed_at(p), sol.status_at(p), sol.stress_at(p))
        else {
            return Readout::default();
        };
        Readout {
            rpm,
            impact,
            capacity,
            status: status_code(st),
        }
    }

    /// End of the level's pass: drop blocks that didn't tick, re-solve if needed.
    /// Returns whether the network was solved again.
    pub fn end_level(&mut self) -> bool {
        let Some(level) = self.level() else {
            return false;
        };
        let gone: Vec<Pos> = level
            .world
            .blocks()
            .map(|(p, _)| *p)
            .filter(|p| !level.seen.contains(p))
            .collect();
        for p in gone {
            level.world.remove(p);
            level.dirty = true;
        }
        if !level.dirty {
            return false;
        }
        resolve_chains(&mut level.world);
        level.solution = Some(level.world.solve());
        level.dirty = false;
        true
    }

    pub fn block_count(&self, dimension: &str) -> usize {
        self.levels
            .get(dimension)
            .map_or(0, |l| l.world.blocks().count())
    }
}

trait ChainExt {
    fn chain_connection(&self) -> Option<Axis>;
    fn chain_connection_mut(&mut self) -> Option<&mut Axis>;
}

impl ChainExt for BlockKind {
    fn chain_connection(&self) -> Option<Axis> {
        match *self {
            BlockKind::ChainDrive { connection } | BlockKind::ChainGearshift { connection, .. } => {
                Some(connection)
            }
            _ => None,
        }
    }
    fn chain_connection_mut(&mut self) -> Option<&mut Axis> {
        match self {
            BlockKind::ChainDrive { connection } | BlockKind::ChainGearshift { connection, .. } => {
                Some(connection)
            }
            _ => None,
        }
    }
}

/// Create's `ChainDriveBlock.updateShape`, simplified: a chain drive links along the
/// first axis (perpendicular to its own) where a neighbouring chain drive also has a
/// perpendicular axis. Chains of more than two follow since each pair agrees.
fn resolve_chains(world: &mut KineticWorld) {
    let chains: Vec<(Pos, Axis)> = world
        .blocks()
        .filter(|(_, b)| b.kind.chain_connection().is_some())
        .map(|(p, b)| (*p, b.axis))
        .collect();
    let is_chain_with_axis_not = |w: &KineticWorld, p: Pos, a: Axis| {
        w.get(p)
            .is_some_and(|b| b.kind.chain_connection().is_some() && b.axis != a)
    };
    let mut decided = Vec::new();
    for (p, axis) in chains {
        let mut conn = default_connection(axis);
        for a in Axis::ALL.into_iter().filter(|&a| a != axis) {
            let step = |s: i32| match a {
                Axis::X => Pos::new(s, 0, 0),
                Axis::Y => Pos::new(0, s, 0),
                Axis::Z => Pos::new(0, 0, s),
            };
            if is_chain_with_axis_not(world, p + step(1), a)
                || is_chain_with_axis_not(world, p + step(-1), a)
            {
                conn = a;
                break;
            }
        }
        decided.push((p, conn));
    }
    for (p, conn) in decided {
        if let Some(slot) = world.get_mut(p).and_then(|b| b.kind.chain_connection_mut()) {
            *slot = conn;
        }
    }
}

/// Client side: turns RPM into `spin(axis, pivot, degrees_per_tick, phase)` so the
/// angle carries on when the speed changes (Horizon computes
/// `angle = game_time * degrees_per_tick + phase`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spinner {
    pub degrees_per_tick: f32,
    pub phase: f32,
}

impl Spinner {
    /// RPM -> degrees per game tick: rpm * 360 / 60 s / 20 ticks.
    pub fn degrees_per_tick(rpm: f32) -> f32 {
        rpm * 0.3
    }

    /// Update for the current speed at `game_time`; returns the values to draw with.
    pub fn update(&mut self, rpm: f32, game_time: i64) -> Spinner {
        let dpt = Self::degrees_per_tick(rpm);
        if dpt != self.degrees_per_tick {
            let t = game_time as f64;
            let angle = (t * self.degrees_per_tick as f64 + self.phase as f64).rem_euclid(360.0);
            self.phase = (angle - t * dpt as f64).rem_euclid(360.0) as f32;
            self.degrees_per_tick = dpt;
        }
        *self
    }
}

/// Unit vector of an axis, for `spin` / `rotate_about`.
pub fn axis_vector(axis: Axis) -> [f32; 3] {
    match axis {
        Axis::X => [1.0, 0.0, 0.0],
        Axis::Y => [0.0, 1.0, 0.0],
        Axis::Z => [0.0, 0.0, 1.0],
    }
}

/// The two lines a gauge shows above itself.
pub fn gauge_text(name: &str, r: &Readout) -> Option<String> {
    match name {
        "speedometer" => Some(format!("{:.0} RPM", r.rpm.abs())),
        "stressometer" => Some(match r.status {
            status::OVERSTRESSED => format!("OVERSTRESSED {:.0}/{:.0} SU", r.impact, r.capacity),
            _ => format!("{:.0}/{:.0} SU", r.impact, r.capacity),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(list: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            list.iter()
                .find(|(a, _)| *a == k)
                .map(|(_, v)| v.to_string())
        }
    }

    const OW: &str = "minecraft:overworld";
    type Props = &'static [(&'static str, &'static str)];
    type Placed<'a> = ([i32; 3], &'a str, Props, i32);

    fn tick(e: &mut Engine, blocks: &[Placed<'_>]) {
        e.begin_level(OW);
        for &(pos, id, p, signal) in blocks {
            e.report(pos, block_from_state(id, &props(p), signal));
        }
        e.end_level();
    }

    #[test]
    fn drill_loads_the_network() {
        let x: Props = &[("axis", "x")];
        let east: Props = &[("facing", "east")];
        let mut e = Engine::new();
        tick(
            &mut e,
            &[
                ([0, 0, 0], "horizon_create:creative_motor", x, 0),
                ([1, 0, 0], "horizon_create:mechanical_drill", east, 0),
            ],
        );
        e.begin_level(OW);
        let r = e.readout([1, 0, 0]);
        assert_eq!(r.rpm, 16.0);
        assert_eq!(r.impact, 64.0); // 4 SU/RPM at 16 RPM
    }

    #[test]
    fn foreign_blocks_are_ignored() {
        assert_eq!(block_from_state("minecraft:stone", &props(&[]), 0), None);
        assert_eq!(
            block_from_state("horizon_create:andesite_casing", &props(&[]), 0),
            None
        );
        assert!(block_from_state("horizon_create:shaft", &props(&[("axis", "x")]), 0).is_some());
    }

    #[test]
    fn creative_motor_drives_shafts_and_redstone_sets_speed() {
        let x = &[("axis", "x")];
        let line = |signal| {
            vec![
                (
                    [0, 64, 0],
                    "horizon_create:creative_motor",
                    x as &'static [_],
                    signal,
                ),
                ([1, 64, 0], "horizon_create:shaft", x, 0),
                ([2, 64, 0], "horizon_create:speedometer", x, 0),
            ]
        };
        let mut e = Engine::new();
        tick(&mut e, &line(0));
        e.begin_level(OW);
        assert_eq!(e.readout([2, 64, 0]).rpm, 16.0);
        assert_eq!(e.readout([2, 64, 0]).status, status::RUNNING);
        tick(&mut e, &line(15));
        e.begin_level(OW);
        assert_eq!(e.readout([2, 64, 0]).rpm, 256.0);
        let r = e.readout([2, 64, 0]);
        assert_eq!(gauge_text("speedometer", &r).unwrap(), "256 RPM");
    }

    #[test]
    fn broken_blocks_drop_out_after_a_tick() {
        let x: &'static [(&str, &str)] = &[("axis", "x")];
        let mut e = Engine::new();
        tick(
            &mut e,
            &[
                ([0, 0, 0], "horizon_create:creative_motor", x, 0),
                ([1, 0, 0], "horizon_create:shaft", x, 0),
                ([2, 0, 0], "horizon_create:shaft", x, 0),
            ],
        );
        // the middle shaft is broken: it stops reporting
        tick(
            &mut e,
            &[
                ([0, 0, 0], "horizon_create:creative_motor", x, 0),
                ([2, 0, 0], "horizon_create:shaft", x, 0),
            ],
        );
        e.begin_level(OW);
        assert_eq!(e.readout([2, 0, 0]).rpm, 0.0);
        assert_eq!(e.block_count(OW), 2);
    }

    #[test]
    fn levels_are_separate() {
        let x: &'static [(&str, &str)] = &[("axis", "x")];
        let mut e = Engine::new();
        tick(
            &mut e,
            &[([0, 0, 0], "horizon_create:creative_motor", x, 0)],
        );
        e.begin_level("minecraft:the_nether");
        e.report(
            [1, 0, 0],
            block_from_state("horizon_create:shaft", &props(x), 0),
        );
        e.end_level();
        e.begin_level("minecraft:the_nether");
        assert_eq!(e.readout([1, 0, 0]).rpm, 0.0);
        assert_eq!(e.block_count(OW), 1);
    }

    #[test]
    fn hand_crank_turns_when_lever_is_on() {
        let wall_on: &'static [(&str, &str)] =
            &[("face", "wall"), ("facing", "east"), ("powered", "true")];
        let b = block_from_state("horizon_create:hand_crank", &props(wall_on), 0).unwrap();
        assert_eq!(b.axis, Axis::X);
        assert_eq!(
            b.role,
            NodeRole::Source {
                rpm: 32.0,
                capacity: 8.0
            }
        );
        let floor_off: &'static [(&str, &str)] =
            &[("face", "floor"), ("facing", "north"), ("powered", "false")];
        let b = block_from_state("horizon_create:hand_crank", &props(floor_off), 0).unwrap();
        assert_eq!(
            (b.axis, b.role),
            (
                Axis::Y,
                NodeRole::Source {
                    rpm: 0.0,
                    capacity: 8.0
                }
            )
        );
    }

    #[test]
    fn clutch_follows_redstone() {
        let x: &'static [(&str, &str)] = &[("axis", "x")];
        let world = |signal| {
            vec![
                ([0, 0, 0], "horizon_create:creative_motor", x, 0),
                ([1, 0, 0], "horizon_create:clutch", x, signal),
                ([2, 0, 0], "horizon_create:shaft", x, 0),
            ]
        };
        let mut e = Engine::new();
        tick(&mut e, &world(0));
        e.begin_level(OW);
        assert_eq!(e.readout([2, 0, 0]).rpm, 16.0);
        tick(&mut e, &world(7));
        e.begin_level(OW);
        assert_eq!(e.readout([2, 0, 0]).rpm, 0.0);
    }

    #[test]
    fn chain_drives_find_their_connection_axis() {
        // two X-axis chain drives stacked on Y, motor on the first, shaft after the second
        let x: &'static [(&str, &str)] = &[("axis", "x")];
        let mut e = Engine::new();
        tick(
            &mut e,
            &[
                ([-1, 0, 0], "horizon_create:creative_motor", x, 0),
                ([0, 0, 0], "horizon_create:encased_chain_drive", x, 0),
                ([0, 1, 0], "horizon_create:encased_chain_drive", x, 0),
                ([1, 1, 0], "horizon_create:shaft", x, 0),
            ],
        );
        e.begin_level(OW);
        assert_eq!(e.readout([1, 1, 0]).rpm, 16.0);
    }

    #[test]
    fn stressometer_reports_capacity() {
        let y: &'static [(&str, &str)] =
            &[("face", "floor"), ("facing", "north"), ("powered", "true")];
        let ya: &'static [(&str, &str)] = &[("axis", "y")];
        let mut e = Engine::new();
        tick(
            &mut e,
            &[
                ([0, 0, 0], "horizon_create:hand_crank", y, 0),
                ([0, 1, 0], "horizon_create:stressometer", ya, 0),
            ],
        );
        e.begin_level(OW);
        let r = e.readout([0, 1, 0]);
        assert_eq!((r.impact, r.capacity), (0.0, 256.0));
        assert_eq!(gauge_text("stressometer", &r).unwrap(), "0/256 SU");
    }

    #[test]
    fn spinner_keeps_angle_continuous() {
        let mut s = Spinner::default();
        s.update(32.0, 0);
        let angle =
            |s: &Spinner, t: i64| (t as f32 * s.degrees_per_tick + s.phase).rem_euclid(360.0);
        let before = angle(&s, 100);
        s.update(64.0, 100);
        let after = angle(&s, 100);
        assert!((before - after).abs() < 1e-2, "{before} vs {after}");
        assert_eq!(s.degrees_per_tick, 19.2);
    }
}
