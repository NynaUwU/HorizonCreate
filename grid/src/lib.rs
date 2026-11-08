//! DC circuit solver using Modified Nodal Analysis (MNA), the same method SPICE uses.
//!
//! Intended as the electric core of a Power Grid-style mod, engine-independent:
//! the game layer maps wires/machines/generators to [`Element`]s, calls
//! [`Circuit::solve`] (e.g. once per tick or when the circuit changes) and reads
//! back voltages, currents, power and overloaded wires.
//!
//! Node `0` is always ground. A tiny conductance (`GMIN`) to ground is added on
//! every node so floating parts of the circuit don't make the matrix singular.

// Index loops read more clearly than iterator chains for matrix code.
#![allow(clippy::needless_range_loop)]

pub type Node = usize;
pub const GROUND: Node = 0;
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
}

impl Element {
    pub fn wire(a: Node, b: Node, ohms: f64, max_current: f64) -> Self {
        Element::Resistor { a, b, ohms, max_current: Some(max_current) }
    }

    /// Resistive load that draws `watts` when it sees `nominal_volts` (R = V²/P).
    pub fn load(a: Node, b: Node, nominal_volts: f64, watts: f64) -> Self {
        Element::Resistor { a, b, ohms: nominal_volts * nominal_volts / watts, max_current: None }
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
    /// Resistors: a -> b. Voltage sources: flowing out of `pos` into the circuit.
    /// Current sources: their set value.
    pub currents: Vec<f64>,
    /// Power for every element. Positive = absorbed (load), negative = delivered (generator).
    pub power: Vec<f64>,
    /// Indices of fusible resistors carrying more than their `max_current`.
    pub overloaded: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Circuit {
    node_count: usize,
    elements: Vec<Element>,
}

impl Circuit {
    /// Circuit with only the ground node.
    pub fn new() -> Self {
        Self { node_count: 1, elements: Vec::new() }
    }

    pub fn add_node(&mut self) -> Node {
        self.node_count += 1;
        self.node_count - 1
    }

    pub fn add(&mut self, e: Element) -> usize {
        self.elements.push(e);
        self.elements.len() - 1
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub fn node_count(&self) -> usize {
        self.node_count
    }

    pub fn solve(&self) -> Result<Solution, SolveError> {
        self.validate()?;
        let n = self.node_count - 1; // unknown node voltages (ground excluded)
        let vsrc: Vec<usize> = self
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Element::VoltageSource { .. }))
            .map(|(i, _)| i)
            .collect();
        let size = n + vsrc.len();
        let mut a = vec![vec![0.0f64; size]; size];
        let mut z = vec![0.0f64; size];
        // matrix row/col for a node (None for ground)
        let idx = |node: Node| if node == GROUND { None } else { Some(node - 1) };

        for i in 0..n {
            a[i][i] += GMIN;
        }

        let mut k = 0;
        for e in &self.elements {
            match *e {
                Element::Resistor { a: na, b: nb, ohms, .. } => {
                    let g = 1.0 / ohms;
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
                }
                Element::CurrentSource { from, to, amps } => {
                    if let Some(i) = idx(from) {
                        z[i] -= amps;
                    }
                    if let Some(j) = idx(to) {
                        z[j] += amps;
                    }
                }
                Element::VoltageSource { pos, neg, volts } => {
                    // extra unknown: current flowing from `pos` through the source to `neg`
                    let row = n + k;
                    if let Some(i) = idx(pos) {
                        a[i][row] += 1.0;
                        a[row][i] += 1.0;
                    }
                    if let Some(j) = idx(neg) {
                        a[j][row] -= 1.0;
                        a[row][j] -= 1.0;
                    }
                    z[row] = volts;
                    k += 1;
                }
            }
        }

        let x = gaussian_solve(a, z).ok_or(SolveError::Singular)?;

        let mut voltages = vec![0.0; self.node_count];
        voltages[1..].copy_from_slice(&x[..n]);

        let mut currents = Vec::with_capacity(self.elements.len());
        let mut power = Vec::with_capacity(self.elements.len());
        let mut overloaded = Vec::new();
        let mut k = 0;
        for (i, e) in self.elements.iter().enumerate() {
            match *e {
                Element::Resistor { a: na, b: nb, ohms, max_current } => {
                    let v = voltages[na] - voltages[nb];
                    let c = v / ohms;
                    currents.push(c);
                    power.push(v * c);
                    if max_current.is_some_and(|m| c.abs() > m) {
                        overloaded.push(i);
                    }
                }
                Element::CurrentSource { from, to, amps } => {
                    currents.push(amps);
                    // current enters `to`: source delivers power when V(to) > V(from)
                    power.push(-(voltages[to] - voltages[from]) * amps);
                }
                Element::VoltageSource { pos, neg, .. } => {
                    // MNA unknown is current pos -> (through source) -> neg; flip it so
                    // positive means the source pushes current out of `pos`.
                    let c = -x[n + k];
                    currents.push(c);
                    power.push(-(voltages[pos] - voltages[neg]) * c);
                    k += 1;
                }
            }
        }

        Ok(Solution { voltages, currents, power, overloaded })
    }

    fn validate(&self) -> Result<(), SolveError> {
        for (index, e) in self.elements.iter().enumerate() {
            let nodes: &[Node] = match e {
                Element::Resistor { a, b, ohms, .. } => {
                    if !(ohms.is_finite() && *ohms > 0.0) {
                        return Err(SolveError::InvalidElement { index, reason: "resistance must be > 0" });
                    }
                    &[*a, *b]
                }
                Element::VoltageSource { pos, neg, .. } => &[*pos, *neg],
                Element::CurrentSource { from, to, .. } => &[*from, *to],
            };
            if nodes.iter().any(|&nd| nd >= self.node_count) {
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
}
