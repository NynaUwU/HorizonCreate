//! Kinetic <-> electric bridge, ported from Create: Power Grid.
//!
//! [`ElectricMotor`] reproduces `ElectricMotorBlockEntity`:
//!
//! - electrically it's a coil (resistor) between two terminals;
//! - every tick it accumulates `rpm = P / torque * (60π/2)`, where
//!   `P = V² / R_nominal` and `torque = stress capacity * torqueForStress`;
//!   the sign follows the coil current;
//! - every [`AVERAGING_TICKS`] ticks the average becomes the generated speed
//!   (truncated to an integer and clamped to ±max RPM, like the Java code);
//! - with *dynamic resistance* the coil resistance becomes `R / load`, where
//!   `load = clamp(stress / capacity of its kinetic network, min_load, 1)`, so an
//!   idle motor draws little current and a fully loaded one draws the nominal current.
//!
//! The game layer drives it once per tick:
//!
//! 1. `circuit.step(TICK)` -> voltage across / current through the coil;
//! 2. [`ElectricMotor::tick`] with them; on `Some(rpm)` update the motor block's
//!    [`kinetics::NodeRole`] with [`ElectricMotor::role`] and re-solve kinetics;
//! 3. [`ElectricMotor::update_load`] with the network's (impact, capacity), then
//!    write [`ElectricMotor::coil_resistance`] back into the circuit.
//!
//! See `tests/motor_loop.rs` for the whole loop.

use kinetics::NodeRole;

/// `ElectricMotorBlockEntity.AVERAGING_TICKS`.
pub const AVERAGING_TICKS: u32 = 5;
/// `ElectricMotorBlockEntity.CONVERSION_CONSTANT` = 60π/2.
pub const CONVERSION_CONSTANT: f64 = 60.0 * std::f64::consts::PI / 2.0;

/// Power Grid defaults for the `electric_motor` block and its configs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotorParams {
    /// Coil resistance (Ω), `CResistance` = 25.6.
    pub resistance: f64,
    /// Stress capacity (SU/RPM), `CStress` = 64.
    pub stress_capacity: f32,
    /// `kinetics.torqueForStress` = 15.
    pub torque_for_stress: f64,
    /// Create's `maxRotationSpeed` = 256.
    pub max_rpm: i32,
    /// `electricity.motorDynamicResistance` = true.
    pub dynamic_resistance: bool,
    /// `electricity.motorMinimumLoad` = 0.05.
    pub min_load: f32,
}

impl Default for MotorParams {
    fn default() -> Self {
        Self {
            resistance: 25.6,
            stress_capacity: 64.0,
            torque_for_stress: 15.0,
            max_rpm: 256,
            dynamic_resistance: true,
            min_load: 0.05,
        }
    }
}

impl MotorParams {
    pub fn torque(&self) -> f64 {
        self.stress_capacity as f64 * self.torque_for_stress
    }

    /// `calculateSpeed(power, torque)`: RPM produced by `watts`.
    pub fn speed_for_power(&self, watts: f64) -> f64 {
        watts / self.torque() * CONVERSION_CONSTANT
    }

    /// Power needed to reach max RPM (shown in Power Grid's tooltip).
    pub fn max_power(&self) -> f64 {
        self.max_rpm as f64 * self.torque() / CONVERSION_CONSTANT
    }

    /// Voltage at which the motor reaches max RPM (`Voltage.max` in the tooltip).
    pub fn max_voltage(&self) -> f64 {
        (self.max_power() * self.resistance).sqrt()
    }
}

#[derive(Clone, Debug)]
pub struct ElectricMotor {
    pub params: MotorParams,
    generated_rpm: i32,
    accumulated: f64,
    ticks: u32,
    load: f32,
}

impl Default for ElectricMotor {
    fn default() -> Self {
        Self::new(MotorParams::default())
    }
}

impl ElectricMotor {
    pub fn new(params: MotorParams) -> Self {
        Self { params, generated_rpm: 0, accumulated: 0.0, ticks: 0, load: params.min_load }
    }

    /// Restore from saved data (Power Grid saves `GeneratedSpeed`).
    pub fn with_generated_rpm(mut self, rpm: i32) -> Self {
        self.generated_rpm = rpm;
        self
    }

    /// Speed the motor currently generates (before the block's facing is applied).
    pub fn generated_rpm(&self) -> i32 {
        self.generated_rpm
    }

    /// Kinetic role of the motor block. `sign` is +1/-1 from the block's facing
    /// (Create's `convertToDirection`).
    pub fn role(&self, sign: f32) -> NodeRole {
        NodeRole::Source { rpm: self.generated_rpm as f32 * sign, capacity: self.params.stress_capacity }
    }

    pub fn load(&self) -> f32 {
        self.load
    }

    /// Resistance the circuit should use for the coil right now.
    pub fn coil_resistance(&self) -> f64 {
        if self.params.dynamic_resistance {
            self.params.resistance / self.load as f64
        } else {
            self.params.resistance
        }
    }

    /// `updateFromNetwork(maxStress, currentStress)`: feed back the stress of the
    /// kinetic network the motor drives (both in SU).
    pub fn update_load(&mut self, impact: f32, capacity: f32) {
        self.load = if capacity != 0.0 {
            (impact / capacity).clamp(self.params.min_load, 1.0)
        } else {
            self.params.min_load
        };
    }

    /// One game tick with the coil's voltage and current. Returns the new generated
    /// RPM when it changed (every [`AVERAGING_TICKS`] ticks at most).
    pub fn tick(&mut self, coil_volts: f64, coil_amps: f64) -> Option<i32> {
        let power = coil_volts * coil_volts / self.params.resistance;
        self.accumulated += self.params.speed_for_power(power) * sign(coil_amps);
        self.ticks += 1;
        if self.ticks < AVERAGING_TICKS {
            return None;
        }
        // Java: (int) (avgSpeed / AVERAGING_TICKS) -> truncation towards zero
        let avg = (self.accumulated / AVERAGING_TICKS as f64) as i32;
        self.accumulated = 0.0;
        self.ticks = 0;
        let new = avg.clamp(-self.params.max_rpm, self.params.max_rpm);
        (new != self.generated_rpm).then(|| {
            self.generated_rpm = new;
            new
        })
    }
}

/// Java's `Math.signum` (0 for 0).
fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_power_grid_tooltip() {
        let p = MotorParams::default();
        assert_eq!(p.torque(), 960.0);
        // 256 RPM * 960 / (30π) ≈ 2607.6 W, at ≈ 258.4 V
        assert!((p.max_power() - 2607.6).abs() < 0.1);
        assert!((p.max_voltage() - 258.4).abs() < 0.1);
    }

    #[test]
    fn speed_updates_every_five_ticks() {
        let mut m = ElectricMotor::default();
        // 230 V -> 2066 W -> 202.8 RPM
        for _ in 0..4 {
            assert_eq!(m.tick(230.0, 1.0), None);
        }
        assert_eq!(m.tick(230.0, 1.0), Some(202));
        // same input: no change reported
        for _ in 0..5 {
            assert_eq!(m.tick(230.0, 1.0), None);
        }
        assert_eq!(m.generated_rpm(), 202);
    }

    #[test]
    fn reverse_current_reverses_and_overvoltage_clamps() {
        let mut m = ElectricMotor::default();
        for _ in 0..5 {
            m.tick(1000.0, -3.0);
        }
        assert_eq!(m.generated_rpm(), -256);
    }

    #[test]
    fn dynamic_resistance_follows_load() {
        let mut m = ElectricMotor::default();
        m.update_load(0.0, 0.0);
        assert!((m.coil_resistance() - 512.0).abs() < 1e-3); // 25.6 / 0.05
        m.update_load(1024.0, 4096.0);
        assert!((m.coil_resistance() - 102.4).abs() < 1e-4); // 25% load
        m.update_load(9999.0, 4096.0);
        assert!((m.coil_resistance() - 25.6).abs() < 1e-9); // clamped to 100%

        m.params.dynamic_resistance = false;
        m.update_load(0.0, 4096.0);
        assert_eq!(m.coil_resistance(), 25.6);
    }
}
