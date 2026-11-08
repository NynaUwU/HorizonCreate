//! Power Grid's generator: a rotor assembly driven by a Create shaft through a
//! generator clutch, inside windings whose field turns its speed into voltage.
//!
//! Ported from `RotorBehaviour` (rotor dynamics, friction, PD controller, overspeed),
//! `GeneratorClutchBlockEntity` (generator mode: stress taken from the shaft, the force
//! it can give), `WindingBlockEntity.fieldStrength` (field with saturation),
//! `InductionRotorBlockEntity` (field averaged over the four sides) and
//! `GeneratorCoupling` (EMF = field · ω, back-EMF resistance, braking force).
//!
//! Units are Power Grid's: rotor speed in RPM, "force" in its own unit
//! (`torqueForStress` × stress / 30 × π), one call of [`Rotor::tick`] per game tick.
//!
//! Per tick, the game layer:
//! 1. reads the shaft speed from kinetics into [`GeneratorClutch::shaft_rpm`];
//! 2. calls [`Rotor::tick`] with the clutch (PD drive towards the shaft speed);
//! 3. sets the circuit's generator source to [`Coupling::emf`] in series with
//!    [`Coupling::resistance`], steps the circuit, and gives the delivered current
//!    back with [`Coupling::apply_current`] (it brakes the rotor);
//! 4. puts [`GeneratorClutch::stress_impact`] on the clutch's kinetic node.

use crate::MotorParams;

/// `CGenerator` / `CKinetics` defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratorParams {
    /// `rotorKp` = 0.85
    pub kp: f32,
    /// `rotorKd` = 0.002
    pub kd: f32,
    /// `rotorRPMMax` = 272
    pub rotor_rpm_max: f32,
    /// `rotorSegmentFriction` = 0.25
    pub segment_friction: f32,
    /// `windingCoilConstant` = 5
    pub coil_constant: f32,
    /// `fieldSaturationCurrent` = 2 A
    pub field_saturation_current: f32,
    /// `torqueForStress` = 15
    pub torque_for_stress: f32,
}

impl Default for GeneratorParams {
    fn default() -> Self {
        Self {
            kp: 0.85,
            kd: 0.002,
            rotor_rpm_max: 272.0,
            segment_friction: 0.25,
            coil_constant: 5.0,
            field_saturation_current: 2.0,
            torque_for_stress: MotorParams::default().torque_for_stress as f32,
        }
    }
}

/// Inertia of each rotor part (`CGenerator`).
pub mod inertia {
    pub const INDUCTION_ROTOR: f32 = 0.5;
    pub const LARGE_INDUCTION_ROTOR: f32 = 2.0;
    pub const COMMUTATOR: f32 = 0.1;
    pub const CLUTCH: f32 = 0.1;
}

/// Stress impact (SU/RPM) of each rotor part (`ModdedBlocks`).
pub mod impact {
    pub const INDUCTION_ROTOR: f32 = 32.0;
    pub const LARGE_INDUCTION_ROTOR: f32 = 128.0;
    pub const CLUTCH: f32 = 32.0;
}

/// Resistance of one winding coil (Ω, `CResistance`).
pub const WINDING_RESISTANCE: f64 = 7.5;

/// Ticks a rotor may stay above max speed before it breaks (`OVERSPEED_TICKS`).
pub const OVERSPEED_TICKS: u32 = 5;
const TICKS_PER_SECOND: f32 = 20.0;

/// `WindingBlockEntity.fieldStrength`: the field a winding makes with `current`
/// amps in its coil, saturating around `fieldSaturationCurrent`.
pub fn field_strength(p: &GeneratorParams, current: f64) -> f64 {
    let i_sat = p.field_saturation_current as f64;
    let current = i_sat * (1.5 * current / i_sat).tanh() + current * 0.05;
    current * p.coil_constant as f64 + 0.001
}

/// `InductionRotorBlockEntity.recalculateField`: the fields of the windings on the
/// rotor's four sides (`None` = no winding there), averaged; ×4 for the large rotor.
pub fn rotor_field(sides: [Option<f64>; 4], large: bool) -> f64 {
    let sum: f64 = sides.iter().flatten().sum();
    sum / 4.0 * if large { 4.0 } else { 1.0 }
}

/// A generator clutch in generator mode: it links a Create shaft to the rotor.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorClutch {
    /// Speed of the kinetic network at the clutch (RPM).
    pub shaft_rpm: f32,
    /// The kinetic network is overstressed (no force then).
    pub overstressed: bool,
    /// Redstone 0..15 weakens the coupling: 15 disconnects it.
    pub redstone: u8,
    /// Sum of the stress impacts of the rotor's segments (`stressSum`).
    pub stress_sum: f32,
    /// Share of the available force used last tick (0..1), what its redstone
    /// output and goggles show.
    pub load: f32,
}

impl GeneratorClutch {
    pub fn new(stress_sum: f32) -> Self {
        Self {
            shaft_rpm: 0.0,
            overstressed: false,
            redstone: 0,
            stress_sum,
            load: 0.0,
        }
    }

    /// Stress impact (SU/RPM) the clutch puts on its kinetic network
    /// (`calculateStressApplied`).
    pub fn stress_impact(&self) -> f32 {
        self.stress_sum * (15 - self.redstone.min(15)) as f32 / 15.0
    }

    /// Most force it can give the rotor this tick (`sourceForce`).
    pub fn source_force(&self, p: &GeneratorParams) -> f32 {
        if self.shaft_rpm == 0.0 || self.overstressed {
            return 0.0;
        }
        p.torque_for_stress * self.stress_impact() / 30.0 * std::f32::consts::PI
    }

    /// `getRedstoneOutput`: 0..15 from the load.
    pub fn redstone_output(&self) -> u8 {
        (self.load * 14.0).floor() as u8 + u8::from(self.load > 0.001)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rotor {
    pub inertia: f32,
    pub segments: u32,
    /// RPM.
    pub angular_velocity: f32,
    old_angular_velocity: f32,
    total_force: f32,
    overspeed_ticks: u32,
    /// Mechanical power the clutch put in last tick (force · ω).
    pub power: f32,
    /// Set when the rotor went too fast for too long: Power Grid destroys it.
    pub broken: bool,
}

impl Rotor {
    /// A rotor assembly made of parts with these inertias (clutch included).
    pub fn new(parts: &[f32]) -> Self {
        Self {
            inertia: parts.iter().sum(),
            segments: parts.len() as u32,
            angular_velocity: 0.0,
            old_angular_velocity: 0.0,
            total_force: 0.0,
            overspeed_ticks: 0,
            power: 0.0,
            broken: false,
        }
    }

    pub fn angular_velocity_radians(&self) -> f32 {
        self.angular_velocity * std::f32::consts::PI / 30.0
    }

    /// `applyTickForce`: add a force for the next tick (generator braking, motor drive).
    pub fn apply_tick_force(&mut self, force: f32) {
        if self.inertia > 0.0 && force.abs() > 0.001 {
            self.total_force += force;
        }
    }

    /// `RotorBehaviour.tick` for the controller segment.
    pub fn tick(&mut self, p: &GeneratorParams, clutch: Option<&mut GeneratorClutch>) {
        if self.broken || self.inertia <= 0.0 {
            return;
        }
        let old = self.old_angular_velocity;
        let velocity = self.angular_velocity;

        let friction = (velocity * TICKS_PER_SECOND * self.inertia)
            .abs()
            .min(self.segments as f32 * p.segment_friction);
        self.total_force -= velocity.signum() * friction * f32::from(velocity != 0.0);

        self.angular_velocity += self.total_force / TICKS_PER_SECOND / self.inertia;
        if self.angular_velocity.abs() < 0.01 || self.angular_velocity.is_nan() {
            self.angular_velocity = 0.0;
        }

        self.power = 0.0;
        if let Some(clutch) = clutch {
            let target = clutch.shaft_rpm;
            let mut delta = target - self.angular_velocity;
            if target < 0.0 {
                delta = -delta;
            }
            let delta = delta.max(0.0);
            let delta_av = old - self.angular_velocity;
            let max_force = clutch.source_force(p);
            let force = (p.kp * delta + p.kd * delta_av) * TICKS_PER_SECOND * self.inertia;
            let force = force.abs().min(max_force) * java_signum(target);
            self.angular_velocity += force / TICKS_PER_SECOND / self.inertia;
            clutch.load = if max_force == 0.0 {
                0.0
            } else {
                (force / max_force).abs()
            };
            self.total_force += force;
            self.power = force * self.angular_velocity_radians();
        }
        self.total_force = 0.0;

        let v = self.angular_velocity.abs();
        if v > p.rotor_rpm_max {
            self.overspeed_ticks += 1;
            if self.overspeed_ticks > OVERSPEED_TICKS {
                self.broken = true;
                self.angular_velocity = 0.0;
            } else {
                self.angular_velocity = p.rotor_rpm_max * self.angular_velocity.signum();
            }
        } else {
            self.overspeed_ticks = 0;
        }
        self.old_angular_velocity = self.angular_velocity;
    }
}

fn java_signum(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// The electrical side of a generator (`GeneratorCoupling`): a voltage source in
/// series with a resistance, both recomputed every tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coupling {
    /// Winding resistance (Ω): coil count × [`WINDING_RESISTANCE`].
    pub base_resistance: f64,
    /// Field strength from the windings ([`rotor_field`]).
    pub field: f64,
}

impl Coupling {
    /// EMF = field · ω (rad/s).
    pub fn emf(&self, rotor: &Rotor) -> f64 {
        self.field * rotor.angular_velocity_radians() as f64
    }

    /// `base + field² · dt / inertia`: the back-EMF term makes a heavy load slow the
    /// rotor smoothly instead of oscillating.
    pub fn resistance(&self, rotor: &Rotor, dt: f64) -> f64 {
        let back = if rotor.inertia != 0.0 {
            self.field * self.field * dt / rotor.inertia as f64
        } else {
            0.0
        };
        self.base_resistance.max(1e-6) + back
    }

    /// The current the generator delivered (out of its positive terminal) brakes the
    /// rotor with force `field · I`.
    pub fn apply_current(&self, rotor: &mut Rotor, amps: f64) {
        rotor.apply_tick_force((-self.field * amps) as f32 * rotor.angular_velocity.signum());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_saturates() {
        let p = GeneratorParams::default();
        let f1 = field_strength(&p, 1.0);
        let f10 = field_strength(&p, 10.0);
        assert!((field_strength(&p, 0.0) - 0.001).abs() < 1e-9);
        // 10x the current gives far less than 10x the field
        assert!(f10 / f1 < 3.0, "{f1} -> {f10}");
    }

    #[test]
    fn clutch_stress_and_redstone() {
        let mut c = GeneratorClutch::new(impact::CLUTCH + impact::INDUCTION_ROTOR);
        assert_eq!(c.stress_impact(), 64.0);
        c.redstone = 15;
        assert_eq!(c.stress_impact(), 0.0);
        c.load = 0.5;
        assert_eq!(c.redstone_output(), 8);
    }

    #[test]
    fn free_rotor_spins_up_to_shaft_speed() {
        let p = GeneratorParams::default();
        let mut rotor = Rotor::new(&[inertia::CLUTCH, inertia::INDUCTION_ROTOR]);
        let mut clutch = GeneratorClutch::new(impact::CLUTCH + impact::INDUCTION_ROTOR);
        clutch.shaft_rpm = 64.0;
        for _ in 0..200 {
            rotor.tick(&p, Some(&mut clutch));
        }
        // friction keeps it a little below the shaft
        assert!(
            rotor.angular_velocity > 55.0 && rotor.angular_velocity <= 64.0,
            "{}",
            rotor.angular_velocity
        );
        assert!(
            clutch.load < 0.2,
            "idle rotor needs little force: {}",
            clutch.load
        );
    }

    #[test]
    fn rotor_coasts_down_without_drive() {
        let p = GeneratorParams::default();
        let mut rotor = Rotor::new(&[inertia::CLUTCH, inertia::INDUCTION_ROTOR]);
        rotor.angular_velocity = 100.0;
        // friction is capped at 0.25 per segment: ~0.04 RPM lost per tick, so it
        // keeps turning like a flywheel for about two minutes
        for _ in 0..1000 {
            rotor.tick(&p, None);
        }
        assert!(rotor.angular_velocity > 50.0 && rotor.angular_velocity < 70.0);
        for _ in 0..2000 {
            rotor.tick(&p, None);
        }
        assert_eq!(rotor.angular_velocity, 0.0);
    }

    #[test]
    fn overspeed_breaks_the_rotor() {
        let p = GeneratorParams::default();
        let mut rotor = Rotor::new(&[inertia::INDUCTION_ROTOR]);
        rotor.angular_velocity = 300.0;
        for _ in 0..=OVERSPEED_TICKS {
            assert!(!rotor.broken);
            rotor.apply_tick_force(1000.0);
            rotor.tick(&p, None);
        }
        assert!(rotor.broken);
    }
}
