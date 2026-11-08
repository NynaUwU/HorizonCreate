//! Circuit solver using Modified Nodal Analysis (MNA), the same method SPICE uses.
//!
//! Intended as the electric core of a Power Grid-style mod, engine-independent:
//! the game layer maps wires/machines/generators to [`Element`]s and either
//!
//! - calls [`Circuit::solve`] for a DC operating point (capacitors open, inductors
//!   shorted, no state change), or
//! - calls [`Circuit::step`] once per game tick (`dt = TICK` = 50 ms) for a transient
//!   simulation: capacitors/inductors keep their charge/current between steps and
//!   fuses blow. This mirrors Power Grid, which uses Backward Euler companion models
//!   (its `TRAPEZOID_APPROX` is `false`) with one solve per tick.
//!
//! Node `0` is always ground. A tiny conductance (`GMIN`) to ground is added on
//! every node so floating parts of the circuit don't make the matrix singular.

// Index loops read more clearly than iterator chains for matrix code.
#![allow(clippy::needless_range_loop)]

pub type Node = usize;
pub const GROUND: Node = 0;
/// One Minecraft tick, the time step Power Grid simulates with.
pub const TICK: f64 = 0.05;
const GMIN: f64 = 1e-12;

#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// Resistor / wire / resistive load. `max_current` (A) marks it as fusible:
    /// exceeding it is reported in [`Solution::overloaded`].
    Resistor { a: Node, b: Node, ohms: f64, max_current: Option<f64> },
    /// Ideal voltage source: `V(pos) - V(neg) = volts`.
    VoltageSource { pos: Node, neg: Node, volts: f64 },
    /// Ideal current source pushing `amps` from `from` to `to` through itself
    /// (i.e. into node `to`).
    CurrentSource { from: Node, to: Node, amps: f64 },
    /// Capacitor. Open in DC; in transient its voltage `V(a) - V(b)` carries over.
    Capacitor { a: Node, b: Node, farads: f64 },
    /// Inductor. Short in DC; in transient its current `a -> b` carries over.
    Inductor { a: Node, b: Node, henries: f64 },
    /// Switch / lever / relay contact: `ohms` when closed, open otherwise.
    Switch { a: Node, b: Node, ohms: f64, closed: bool },
    /// Fuse: a resistor that opens for good (until [`Circuit::reset_fuse`]) once
    /// `|I| > max_current` during a [`Circuit::step`]. Like Power Grid's
    /// `FuseSwitchWire`, the fuse opens on the step *after* the overcurrent.
    Fuse { a: Node, b: Node, ohms: f64, max_current: f64 },
}

impl Element {
    pub fn wire(a: Node, b: Node, ohms: f64, max_current: f64) -> Self {
        Element::Resistor { a, b, ohms, max_current: Some(max_current) }
    }

    /// Resistive load that draws `watts` when it sees `nominal_volts` (R = V²/P).
    pub fn load(a: Node, b: Node, nominal_volts: f64, watts: f64) -> Self {
        Element::Resistor { a, b, ohms: nominal_volts * nominal_volts / watts, max_current: None }
    }

    fn terminals(&self) -> [Node; 2] {
        match *self {
            Element::Resistor { a, b, .. }
            | Element::Capacitor { a, b, .. }
            | Element::Inductor { a, b, .. }
            | Element::Switch { a, b, .. }
            | Element::Fuse { a, b, .. } => [a, b],
            Element::VoltageSource { pos, neg, .. } => [pos, neg],
            Element::CurrentSource { from, to, .. } => [from, to],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SolveError {
    /// Matrix is singular, e.g. two ideal voltage sources in parallel with different values
    /// or a loop made only of voltage sources.
    Singular,
    InvalidElement { index: usize, reason: &'static str },
}

#[derive(Clone, Debug)]
pub struct Solution {
    /// Voltage of every node (index 0 = ground = 0 V).
    pub voltages: Vec<f64>,
    /// Current through every element, same order as the circuit's elements.
    /// Two-terminal passives: a -> b. Voltage sources: flowing out of `pos` into the
    /// circuit. Current sources: their set value. Open switches/blown fuses: 0.
    pub currents: Vec<f64>,
    /// Power for every element. Positive = absorbed (load), negative = delivered
    /// (generator; also a capacitor/inductor giving back stored energy).
    pub power: Vec<f64>,
    /// Indices of resistors with `max_current` and fuses carrying more than allowed.
    pub overloaded: Vec<usize>,
    /// Fuses that blew during this step (they're open from the next step on).
    pub blown: Vec<usize>,
}

/// Per-element dynamic state used by [`Circuit::step`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct State {
    /// Capacitor voltage, or inductor current.
    value: f64,
    blown: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Dc,
    Transient(f64),
}

#[derive(Clone, Debug, Default)]
pub struct Circuit {
    node_count: usize,
    elements: Vec<Element>,
    state: Vec<State>,
}

impl Circuit {
    /// Circuit with only the ground node.
    pub fn new() -> Self {
        Self { node_count: 1, elements: Vec::new(), state: Vec::new() }
    }

    pub fn add_node(&mut self) -> Node {
        self.node_count += 1;
        self.node_count - 1
    }

    pub fn add(&mut self, e: Element) -> usize {
        self.elements.push(e);
        self.state.push(State::default());
        self.elements.len() - 1
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    /// Replace an element in place (e.g. a generator changing its voltage, a lever
    /// flipping). Keeps its dynamic state.
    pub fn set(&mut self, index: usize, e: Element) {
        self.elements[index] = e;
    }

    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Open or close a [`Element::Switch`]. No-op for other elements.
    pub fn set_switch(&mut self, index: usize, closed: bool) {
        if let Element::Switch { closed: c, .. } = &mut self.elements[index] {
            *c = closed;
        }
    }

    pub fn is_blown(&self, index: usize) -> bool {
        self.state[index].blown
    }

    /// Replace a blown fuse.
    pub fn reset_fuse(&mut self, index: usize) {
        self.state[index].blown = false;
    }

    /// Stored capacitor voltage / inductor current.
    pub fn stored(&self, index: usize) -> f64 {
        self.state[index].value
    }

    /// Set the initial capacitor voltage / inductor current (e.g. loading a save).
    pub fn set_stored(&mut self, index: usize, value: f64) {
        self.state[index].value = value;
    }

    /// DC operating point: capacitors are open, inductors are shorts. Doesn't touch
    /// the circuit's state (fuses don't blow, but overcurrent shows in `overloaded`).
    pub fn solve(&self) -> Result<Solution, SolveError> {
        self.solve_mode(Mode::Dc)
    }

    /// Advance the transient simulation by `dt` seconds (use [`TICK`] once per tick).
    pub fn step(&mut self, dt: f64) -> Result<Solution, SolveError> {
        assert!(dt > 0.0, "dt must be > 0");
        let mut sol = self.solve_mode(Mode::Transient(dt))?;
        for (i, e) in self.elements.iter().enumerate() {
            match *e {
                Element::Capacitor { a, b, .. } => {
                    self.state[i].value = sol.voltages[a] - sol.voltages[b];
                }
                Element::Inductor { .. } => self.state[i].value = sol.currents[i],
                Element::Fuse { max_current, .. }
                    if !self.state[i].blown && sol.currents[i].abs() > max_current =>
                {
                    self.state[i].blown = true;
                    sol.blown.push(i);
                }
                _ => {}
            }
        }
        Ok(sol)
    }

    fn conducts(&self, i: usize) -> bool {
        match self.elements[i] {
            Element::Switch { closed, .. } => closed,
            Element::Fuse { .. } => !self.state[i].blown,
            _ => true,
        }
    }

    fn solve_mode(&self, mode: Mode) -> Result<Solution, SolveError> {
        self.validate()?;
        let n = self.node_count - 1; // unknown node voltages (ground excluded)
        // Elements that need an extra MNA unknown (their current): voltage sources,
        // and inductors in DC (0 V sources).
        let mut branch = vec![usize::MAX; self.elements.len()];
        let mut extra = 0;
        for (i, e) in self.elements.iter().enumerate() {
            let needs = match e {
                Element::VoltageSource { .. } => true,
                Element::Inductor { .. } => mode == Mode::Dc,
                _ => false,
            };
            if needs {
                branch[i] = n + extra;
                extra += 1;
            }
        }
        let size = n + extra;
        let mut a = vec![vec![0.0f64; size]; size];
        let mut z = vec![0.0f64; size];
        // matrix row/col for a node (None for ground)
        let idx = |node: Node| if node == GROUND { None } else { Some(node - 1) };

        for i in 0..n {
            a[i][i] += GMIN;
        }

        let stamp_g = |a: &mut Vec<Vec<f64>>, na: Node, nb: Node, g: f64| {
            if let Some(i) = idx(na) {
                a[i][i] += g;
            }
            if let Some(j) = idx(nb) {
                a[j][j] += g;
            }
            if let (Some(i), Some(j)) = (idx(na), idx(nb)) {
                a[i][j] -= g;
                a[j][i] -= g;
            }
        };
        // current `amps` leaving `from`, entering `to`
        let stamp_i = |z: &mut Vec<f64>, from: Node, to: Node, amps: f64| {
            if let Some(i) = idx(from) {
                z[i] -= amps;
            }
            if let Some(j) = idx(to) {
                z[j] += amps;
            }
        };
        let stamp_branch = |a: &mut Vec<Vec<f64>>, z: &mut Vec<f64>, row: usize, pos: Node, neg: Node, volts: f64| {
            if let Some(i) = idx(pos) {
                a[i][row] += 1.0;
                a[row][i] += 1.0;
            }
            if let Some(j) = idx(neg) {
                a[j][row] -= 1.0;
                a[row][j] -= 1.0;
            }
            z[row] = volts;
        };

        for (k, e) in self.elements.iter().enumerate() {
            if !self.conducts(k) {
                continue;
            }
            match *e {
                Element::Resistor { a: na, b: nb, ohms, .. }
                | Element::Switch { a: na, b: nb, ohms, .. }
                | Element::Fuse { a: na, b: nb, ohms, .. } => stamp_g(&mut a, na, nb, 1.0 / ohms),
                Element::CurrentSource { from, to, amps } => stamp_i(&mut z, from, to, amps),
                Element::VoltageSource { pos, neg, volts } => {
                    stamp_branch(&mut a, &mut z, branch[k], pos, neg, volts)
                }
                Element::Capacitor { a: na, b: nb, farads } => {
                    if let Mode::Transient(dt) = mode {
                        // i = C/dt * (v - v_prev): conductance + source pushing b -> a
                        let g = farads / dt;
                        stamp_g(&mut a, na, nb, g);
                        stamp_i(&mut z, nb, na, g * self.state[k].value);
                    }
                }
                Element::Inductor { a: na, b: nb, henries } => match mode {
                    Mode::Dc => stamp_branch(&mut a, &mut z, branch[k], na, nb, 0.0),
                    Mode::Transient(dt) => {
                        // i = i_prev + dt/L * v
                        stamp_g(&mut a, na, nb, dt / henries);
                        stamp_i(&mut z, na, nb, self.state[k].value);
                    }
                },
            }
        }

        let x = gaussian_solve(a, z).ok_or(SolveError::Singular)?;

        let mut voltages = vec![0.0; self.node_count];
        voltages[1..].copy_from_slice(&x[..n]);

        let mut currents = Vec::with_capacity(self.elements.len());
        let mut power = Vec::with_capacity(self.elements.len());
        let mut overloaded = Vec::new();
        for (i, e) in self.elements.iter().enumerate() {
            let [ta, tb] = e.terminals();
            let v = voltages[ta] - voltages[tb];
            let c = if !self.conducts(i) {
                0.0
            } else {
                match *e {
                    Element::Resistor { ohms, .. } | Element::Switch { ohms, .. } | Element::Fuse { ohms, .. } => {
                        v / ohms
                    }
                    Element::CurrentSource { amps, .. } => amps,
                    // MNA unknown is current pos -> (through source) -> neg; flip it so
                    // positive means the source pushes current out of `pos`.
                    Element::VoltageSource { .. } => -x[branch[i]],
                    Element::Capacitor { farads, .. } => match mode {
                        Mode::Dc => 0.0,
                        Mode::Transient(dt) => farads / dt * (v - self.state[i].value),
                    },
                    Element::Inductor { henries, .. } => match mode {
                        Mode::Dc => x[branch[i]],
                        Mode::Transient(dt) => self.state[i].value + dt / henries * v,
                    },
                }
            };
            currents.push(c);
            power.push(match e {
                // `c` flows out of `pos`, so the source delivers when v * c > 0
                Element::VoltageSource { .. } => -v * c,
                // passives: a -> b; current source: v = V(from) - V(to), so it
                // delivers (negative) when pushing current uphill into `to`
                _ => v * c,
            });
            let limit = match *e {
                Element::Resistor { max_current, .. } => max_current,
                Element::Fuse { max_current, .. } => Some(max_current),
                _ => None,
            };
            if limit.is_some_and(|m| c.abs() > m) {
                overloaded.push(i);
            }
        }

        Ok(Solution { voltages, currents, power, overloaded, blown: Vec::new() })
    }

    fn validate(&self) -> Result<(), SolveError> {
        for (index, e) in self.elements.iter().enumerate() {
            let positive = |v: f64| v.is_finite() && v > 0.0;
            let bad = match *e {
                Element::Resistor { ohms, .. } | Element::Switch { ohms, .. } | Element::Fuse { ohms, .. } => {
                    (!positive(ohms)).then_some("resistance must be > 0")
                }
                Element::Capacitor { farads, .. } => (!positive(farads)).then_some("capacitance must be > 0"),
                Element::Inductor { henries, .. } => (!positive(henries)).then_some("inductance must be > 0"),
                _ => None,
            };
            if let Some(reason) = bad {
                return Err(SolveError::InvalidElement { index, reason });
            }
            if e.terminals().iter().any(|&nd| nd >= self.node_count) {
                return Err(SolveError::InvalidElement { index, reason: "unknown node" });
            }
        }
        Ok(())
    }
}

/// Dense Gaussian elimination with partial pivoting. Fine for game-sized circuits
/// (hundreds of nodes); swap for a sparse LU if networks get large.
fn gaussian_solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-18 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            for c in col..n {
                a[row][c] -= f * a[col][c];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|c| a[row][c] * x[c]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn voltage_divider() {
        let mut c = Circuit::new();
        let top = c.add_node();
        let mid = c.add_node();
        let src = c.add(Element::VoltageSource { pos: top, neg: GROUND, volts: 12.0 });
        c.add(Element::Resistor { a: top, b: mid, ohms: 1000.0, max_current: None });
        c.add(Element::Resistor { a: mid, b: GROUND, ohms: 2000.0, max_current: None });
        let s = c.solve().unwrap();
        assert!(close(s.voltages[mid], 8.0));
        assert!(close(s.currents[src], 0.004));
        assert!(close(s.power[src], -0.048)); // delivers 48 mW
    }

    #[test]
    fn power_balance() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let b = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 230.0 });
        c.add(Element::wire(a, b, 0.5, 100.0));
        c.add(Element::load(b, GROUND, 230.0, 2000.0));
        c.add(Element::load(b, GROUND, 230.0, 1000.0));
        let s = c.solve().unwrap();
        let total: f64 = s.power.iter().sum();
        assert!(total.abs() < 1e-6, "sum of powers should be zero, got {total}");
        // wire drop lowers the voltage the loads see
        assert!(s.voltages[b] < 230.0 && s.voltages[b] > 220.0);
    }

    #[test]
    fn load_from_power_draws_nominal_power() {
        let mut c = Circuit::new();
        let a = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 120.0 });
        let l = c.add(Element::load(a, GROUND, 120.0, 600.0));
        let s = c.solve().unwrap();
        assert!(close(s.power[l], 600.0));
        assert!(close(s.currents[l], 5.0));
    }

    #[test]
    fn overloaded_wire_is_reported() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let b = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 100.0 });
        let w = c.add(Element::wire(a, b, 0.1, 5.0));
        c.add(Element::Resistor { a: b, b: GROUND, ohms: 10.0, max_current: None }); // ~9.9 A
        let s = c.solve().unwrap();
        assert_eq!(s.overloaded, vec![w]);
    }

    #[test]
    fn current_source_into_resistor() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let src = c.add(Element::CurrentSource { from: GROUND, to: a, amps: 2.0 });
        c.add(Element::Resistor { a, b: GROUND, ohms: 5.0, max_current: None });
        let s = c.solve().unwrap();
        assert!(close(s.voltages[a], 10.0));
        assert!(close(s.power[src], -20.0));
    }

    #[test]
    fn floating_node_does_not_break_solver() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let island_a = c.add_node();
        let island_b = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 5.0 });
        c.add(Element::Resistor { a, b: GROUND, ohms: 10.0, max_current: None });
        c.add(Element::Resistor { a: island_a, b: island_b, ohms: 10.0, max_current: None });
        let s = c.solve().unwrap();
        assert!(close(s.voltages[a], 5.0));
        assert!(s.voltages[island_a].abs() < 1e-6);
    }

    #[test]
    fn conflicting_parallel_sources_are_singular() {
        let mut c = Circuit::new();
        let a = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 5.0 });
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 12.0 });
        assert_eq!(c.solve().unwrap_err(), SolveError::Singular);
    }

    #[test]
    fn invalid_elements_rejected() {
        let mut c = Circuit::new();
        let a = c.add_node();
        c.add(Element::Resistor { a, b: GROUND, ohms: 0.0, max_current: None });
        assert!(matches!(c.solve(), Err(SolveError::InvalidElement { index: 0, .. })));
        let mut c = Circuit::new();
        c.add(Element::Resistor { a: 7, b: GROUND, ohms: 1.0, max_current: None });
        assert!(matches!(c.solve(), Err(SolveError::InvalidElement { .. })));
    }

    #[test]
    fn rc_charging_follows_exponential() {
        // 10 V, R = 1 kOhm, C = 1 mF -> tau = 1 s = 20 ticks
        let mut c = Circuit::new();
        let top = c.add_node();
        let mid = c.add_node();
        c.add(Element::VoltageSource { pos: top, neg: GROUND, volts: 10.0 });
        c.add(Element::Resistor { a: top, b: mid, ohms: 1000.0, max_current: None });
        let cap = c.add(Element::Capacitor { a: mid, b: GROUND, farads: 1e-3 });
        // DC: capacitor is open, sees the full voltage, no current
        let dc = c.solve().unwrap();
        assert!(close(dc.voltages[mid], 10.0) && close(dc.currents[cap], 0.0));

        let mut s = None;
        for _ in 0..20 {
            s = Some(c.step(TICK).unwrap());
        }
        let v = s.unwrap().voltages[mid];
        // exact: 10 * (1 - e^-1) = 6.32; backward Euler lags slightly behind
        assert!(v > 6.0 && v < 6.32, "v = {v}");
        assert!(close(c.stored(cap), v));
    }

    #[test]
    fn charged_capacitor_discharges_into_load() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let cap = c.add(Element::Capacitor { a, b: GROUND, farads: 0.1 });
        let r = c.add(Element::Resistor { a, b: GROUND, ohms: 10.0, max_current: None });
        c.set_stored(cap, 100.0);
        let s = c.step(TICK).unwrap();
        // tau = 1 s; after one 50 ms BE step: 100 / (1 + 0.05) = 95.24 V
        assert!((s.voltages[a] - 100.0 / 1.05).abs() < 1e-6);
        // capacitor delivers what the resistor absorbs
        assert!((s.power[cap] + s.power[r]).abs() < 1e-6);
        assert!(s.power[cap] < 0.0);
    }

    #[test]
    fn rl_current_rises_and_inductor_is_short_in_dc() {
        // 12 V, R = 2 Ohm, L = 1 H -> tau = 0.5 s, final current 6 A
        let mut c = Circuit::new();
        let a = c.add_node();
        let b = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 12.0 });
        c.add(Element::Resistor { a, b, ohms: 2.0, max_current: None });
        let l = c.add(Element::Inductor { a: b, b: GROUND, henries: 1.0 });
        assert!(close(c.solve().unwrap().currents[l], 6.0));

        let first = c.step(TICK).unwrap().currents[l];
        assert!(first > 0.0 && first < 1.0, "inductor resists sudden change: {first}");
        for _ in 0..100 {
            c.step(TICK).unwrap();
        }
        assert!((c.stored(l) - 6.0).abs() < 1e-3);
    }

    #[test]
    fn open_switch_breaks_the_circuit() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let b = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 10.0 });
        let sw = c.add(Element::Switch { a, b, ohms: 0.01, closed: false });
        let load = c.add(Element::Resistor { a: b, b: GROUND, ohms: 10.0, max_current: None });
        let s = c.solve().unwrap();
        assert!(close(s.currents[load], 0.0) && close(s.currents[sw], 0.0));
        c.set_switch(sw, true);
        assert!((c.solve().unwrap().currents[load] - 10.0 / 10.01).abs() < 1e-9);
    }

    #[test]
    fn fuse_blows_on_next_step_and_can_be_reset() {
        let mut c = Circuit::new();
        let a = c.add_node();
        let b = c.add_node();
        c.add(Element::VoltageSource { pos: a, neg: GROUND, volts: 10.0 });
        let fuse = c.add(Element::Fuse { a, b, ohms: 0.01, max_current: 5.0 });
        let load = c.add(Element::Resistor { a: b, b: GROUND, ohms: 1.0, max_current: None });

        // DC solve reports but doesn't blow
        assert_eq!(c.solve().unwrap().overloaded, vec![fuse]);
        assert!(!c.is_blown(fuse));

        let s = c.step(TICK).unwrap();
        assert_eq!(s.blown, vec![fuse]);
        assert!(s.currents[load] > 5.0); // the overcurrent tick still happens
        let s = c.step(TICK).unwrap();
        assert!(s.blown.is_empty() && close(s.currents[load], 0.0));

        c.reset_fuse(fuse);
        assert!(c.step(TICK).unwrap().currents[load] > 5.0);
    }

    #[test]
    fn lc_tank_keeps_oscillating_energy_bounded() {
        // Backward Euler is dissipative: energy must never grow.
        let mut c = Circuit::new();
        let a = c.add_node();
        let cap = c.add(Element::Capacitor { a, b: GROUND, farads: 1.0 });
        let l = c.add(Element::Inductor { a, b: GROUND, henries: 1.0 });
        c.set_stored(cap, 1.0);
        let energy = |c: &Circuit| 0.5 * c.stored(cap).powi(2) + 0.5 * c.stored(l).powi(2);
        let mut prev = energy(&c);
        let mut crossed = false;
        for _ in 0..200 {
            let s = c.step(TICK).unwrap();
            crossed |= s.voltages[a] < 0.0;
            let e = energy(&c);
            assert!(e <= prev + 1e-12);
            prev = e;
        }
        assert!(crossed, "should oscillate (period 2*pi s = ~126 ticks)");
    }

    #[test]
    fn invalid_reactive_elements_rejected() {
        let mut c = Circuit::new();
        let a = c.add_node();
        c.add(Element::Capacitor { a, b: GROUND, farads: 0.0 });
        assert!(matches!(c.solve(), Err(SolveError::InvalidElement { index: 0, .. })));
    }
}

