//! Abstract rotational network: nodes + edges with speed ratios.
//!
//! An edge `a -> b` with ratio `r` means `speed(b) = speed(a) * r`.
//! The reverse direction is implied (`speed(a) = speed(b) / r`).
//!
//! Stress model (same as Create):
//! - a source with `capacity` (SU per RPM) running at `rpm` provides `capacity * |rpm|` SU;
//! - a consumer with `impact` (SU per RPM) spinning at `speed` uses `impact * |speed|` SU;
//! - if total impact > total capacity the whole network is overstressed and stops.

use std::collections::VecDeque;

pub type NodeId = usize;

/// Create's hard speed limit. Anything faster breaks.
pub const MAX_RPM: f32 = 256.0;
const EPS: f32 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeRole {
    /// Shafts, cogwheels, gearboxes... transmit rotation only.
    Passive,
    /// Generator (motor, water wheel, engine). `capacity` is in SU per RPM.
    Source { rpm: f32, capacity: f32 },
    /// Machine (press, mixer, fan...). `impact` is in SU per RPM.
    Consumer { impact: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Connected network without any active source.
    Stopped,
    /// Everything fine.
    Running,
    /// Stress impact exceeds capacity: all speeds forced to 0.
    Overstressed,
    /// Two sources disagree on direction, or a gear loop has incompatible ratios.
    /// In Create the offending block pops off; here we report it and stop.
    Conflict { a: NodeId, b: NodeId },
    /// Some node would exceed [`MAX_RPM`].
    Overspeed { node: NodeId },
}

#[derive(Clone, Debug)]
pub struct Subnetwork {
    pub nodes: Vec<NodeId>,
    pub status: Status,
    /// Total stress capacity (SU).
    pub capacity: f32,
    /// Total stress impact (SU), computed at the theoretical speeds.
    pub impact: f32,
}

#[derive(Clone, Debug)]
pub struct Solution {
    /// Final speed of every node (0 when its subnetwork isn't `Running`).
    pub speed: Vec<f32>,
    /// Index into `subnetworks` for every node.
    pub subnetwork_of: Vec<usize>,
    pub subnetworks: Vec<Subnetwork>,
}

impl Solution {
    pub fn status_of(&self, node: NodeId) -> &Status {
        &self.subnetworks[self.subnetwork_of[node]].status
    }
}

#[derive(Clone, Debug, Default)]
pub struct Network {
    roles: Vec<NodeRole>,
    adj: Vec<Vec<(NodeId, f32)>>,
}

impl Network {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, role: NodeRole) -> NodeId {
        self.roles.push(role);
        self.adj.push(Vec::new());
        self.roles.len() - 1
    }

    pub fn set_role(&mut self, node: NodeId, role: NodeRole) {
        self.roles[node] = role;
    }

    pub fn role(&self, node: NodeId) -> NodeRole {
        self.roles[node]
    }

    pub fn len(&self) -> usize {
        self.roles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }

    /// Connect `a` and `b` so that `speed(b) = speed(a) * ratio`. `ratio` must be non-zero.
    pub fn connect(&mut self, a: NodeId, b: NodeId, ratio: f32) {
        assert!(ratio.abs() > EPS, "ratio must be non-zero");
        self.adj[a].push((b, ratio));
        self.adj[b].push((a, 1.0 / ratio));
    }

    pub fn solve(&self) -> Solution {
        let n = self.len();
        let mut speed = vec![0.0f32; n];
        let mut subnetwork_of = vec![usize::MAX; n];
        // factor[i]: speed(i) = factor[i] * X, where X is the speed of the BFS root.
        let mut factor = vec![0.0f32; n];
        let mut subnetworks = Vec::new();

        for root in 0..n {
            if subnetwork_of[root] != usize::MAX {
                continue;
            }
            let id = subnetworks.len();
            let mut nodes = Vec::new();
            let mut conflict: Option<(NodeId, NodeId)> = None;

            // 1. Relative factors via BFS; detect loops with inconsistent ratios.
            subnetwork_of[root] = id;
            factor[root] = 1.0;
            let mut queue = VecDeque::from([root]);
            while let Some(cur) = queue.pop_front() {
                nodes.push(cur);
                for &(next, ratio) in &self.adj[cur] {
                    let expected = factor[cur] * ratio;
                    if subnetwork_of[next] == usize::MAX {
                        subnetwork_of[next] = id;
                        factor[next] = expected;
                        queue.push_back(next);
                    } else if conflict.is_none() && !approx(factor[next], expected) {
                        conflict = Some((cur, next));
                    }
                }
            }

            // 2. Sources must agree on the root speed X. The fastest one dictates
            //    (Create behaviour); opposite directions are a conflict.
            let mut x: Option<(f32, NodeId)> = None;
            let mut capacity = 0.0;
            for &i in &nodes {
                if let NodeRole::Source { rpm, capacity: cap } = self.roles[i] {
                    capacity += cap * rpm.abs();
                    if rpm.abs() < EPS {
                        continue;
                    }
                    let xi = rpm / factor[i];
                    match x {
                        None => x = Some((xi, i)),
                        Some((x0, j)) => {
                            if xi.signum() != x0.signum() {
                                conflict.get_or_insert((j, i));
                            } else if xi.abs() > x0.abs() {
                                x = Some((xi, i));
                            }
                        }
                    }
                }
            }

            let x = x.map(|(v, _)| v).unwrap_or(0.0);
            let impact: f32 = nodes
                .iter()
                .map(|&i| match self.roles[i] {
                    NodeRole::Consumer { impact } => impact * (factor[i] * x).abs(),
                    _ => 0.0,
                })
                .sum();

            let status = if let Some((a, b)) = conflict {
                Status::Conflict { a, b }
            } else if x.abs() < EPS {
                Status::Stopped
            } else if let Some(&node) = nodes
                .iter()
                .find(|&&i| (factor[i] * x).abs() > MAX_RPM + EPS)
            {
                Status::Overspeed { node }
            } else if impact > capacity + EPS {
                Status::Overstressed
            } else {
                Status::Running
            };

            if status == Status::Running {
                for &i in &nodes {
                    speed[i] = factor[i] * x;
                }
            }
            subnetworks.push(Subnetwork { nodes, status, capacity, impact });
        }

        Solution { speed, subnetwork_of, subnetworks }
    }
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS * a.abs().max(b.abs()).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motor(rpm: f32) -> NodeRole {
        NodeRole::Source { rpm, capacity: 16.0 } // creative motor-ish: 16 SU/RPM
    }

    #[test]
    fn chain_propagates_with_ratios() {
        let mut n = Network::new();
        let m = n.add_node(motor(32.0));
        let a = n.add_node(NodeRole::Passive);
        let b = n.add_node(NodeRole::Passive);
        n.connect(m, a, 1.0);
        n.connect(a, b, -2.0);
        let s = n.solve();
        assert_eq!(s.status_of(m), &Status::Running);
        assert_eq!(s.speed[a], 32.0);
        assert_eq!(s.speed[b], -64.0);
    }

    #[test]
    fn no_source_is_stopped() {
        let mut n = Network::new();
        let a = n.add_node(NodeRole::Passive);
        let b = n.add_node(NodeRole::Consumer { impact: 4.0 });
        n.connect(a, b, 1.0);
        let s = n.solve();
        assert_eq!(s.status_of(a), &Status::Stopped);
        assert_eq!(s.speed[b], 0.0);
    }

    #[test]
    fn overstress_stops_everything() {
        // 16 SU/RPM * 16 RPM = 256 SU capacity; press 8 SU/RPM * 64 RPM = 512 SU.
        let mut n = Network::new();
        let m = n.add_node(motor(16.0));
        let c = n.add_node(NodeRole::Consumer { impact: 8.0 });
        n.connect(m, c, 4.0);
        let s = n.solve();
        assert_eq!(s.status_of(m), &Status::Overstressed);
        assert_eq!(s.subnetworks[0].capacity, 256.0);
        assert_eq!(s.subnetworks[0].impact, 512.0);
        assert_eq!(s.speed[c], 0.0);
    }

    #[test]
    fn exactly_at_capacity_runs() {
        let mut n = Network::new();
        let m = n.add_node(motor(16.0));
        let c = n.add_node(NodeRole::Consumer { impact: 16.0 });
        n.connect(m, c, 1.0);
        assert_eq!(n.solve().status_of(m), &Status::Running);
    }

    #[test]
    fn opposing_sources_conflict() {
        let mut n = Network::new();
        let m1 = n.add_node(motor(32.0));
        let m2 = n.add_node(motor(32.0));
        n.connect(m1, m2, -1.0); // m2 must spin at -32 but wants +32
        assert!(matches!(n.solve().status_of(m1), Status::Conflict { .. }));
    }

    #[test]
    fn faster_source_dictates_same_direction() {
        let mut n = Network::new();
        let m1 = n.add_node(motor(16.0));
        let m2 = n.add_node(motor(64.0));
        n.connect(m1, m2, 1.0);
        let s = n.solve();
        assert_eq!(s.status_of(m1), &Status::Running);
        assert_eq!(s.speed[m1], 64.0);
        // both contribute capacity at their own rated speed
        assert_eq!(s.subnetworks[0].capacity, 16.0 * 16.0 + 16.0 * 64.0);
    }

    #[test]
    fn inconsistent_gear_loop_conflicts() {
        // triangle of meshing cogs: -1 * -1 * -1 != 1 -> gears lock
        let mut n = Network::new();
        let a = n.add_node(motor(8.0));
        let b = n.add_node(NodeRole::Passive);
        let c = n.add_node(NodeRole::Passive);
        n.connect(a, b, -1.0);
        n.connect(b, c, -1.0);
        n.connect(c, a, -1.0);
        assert!(matches!(n.solve().status_of(a), Status::Conflict { .. }));
    }

    #[test]
    fn consistent_loop_is_fine() {
        // square of meshing cogs: (-1)^4 = 1
        let mut n = Network::new();
        let ids: Vec<_> = (0..4).map(|_| n.add_node(NodeRole::Passive)).collect();
        n.set_role(ids[0], motor(8.0));
        for i in 0..4 {
            n.connect(ids[i], ids[(i + 1) % 4], -1.0);
        }
        let s = n.solve();
        assert_eq!(s.status_of(ids[0]), &Status::Running);
        assert_eq!(s.speed[ids[2]], 8.0);
        assert_eq!(s.speed[ids[3]], -8.0);
    }

    #[test]
    fn overspeed_detected() {
        let mut n = Network::new();
        let m = n.add_node(motor(256.0));
        let small = n.add_node(NodeRole::Passive);
        n.connect(m, small, -2.0); // 512 RPM
        assert_eq!(n.solve().status_of(m), &Status::Overspeed { node: small });
    }

    #[test]
    fn separate_networks_are_independent() {
        let mut n = Network::new();
        let m = n.add_node(motor(32.0));
        let a = n.add_node(NodeRole::Passive);
        let lone = n.add_node(NodeRole::Passive);
        n.connect(m, a, 1.0);
        let s = n.solve();
        assert_eq!(s.subnetworks.len(), 2);
        assert_eq!(s.status_of(lone), &Status::Stopped);
        assert_eq!(s.speed[a], 32.0);
    }
}
