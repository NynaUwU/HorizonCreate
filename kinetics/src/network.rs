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

/// A rotation speed controller link (Create's `SpeedControllerBlockEntity`).
///
/// Whichever side is driven by something else forces the other side to spin at
/// `target` RPM (measured at that side's node). Both sides then share one stress
/// network, like in Create where the cog's source is the controller.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ControlledLink {
    a: NodeId,
    b: NodeId,
    target: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Origin {
    Source,
    Link(usize),
}

#[derive(Clone, Debug)]
pub struct Network {
    roles: Vec<NodeRole>,
    adj: Vec<Vec<(NodeId, f32)>>,
    links: Vec<ControlledLink>,
    max_rpm: f32,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            roles: Vec::new(),
            adj: Vec::new(),
            links: Vec::new(),
            max_rpm: MAX_RPM,
        }
    }
}

impl Network {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the overspeed limit (Create's `maxRotationSpeed` config, default 256).
    pub fn with_max_rpm(mut self, max_rpm: f32) -> Self {
        self.max_rpm = max_rpm;
        self
    }

    pub fn max_rpm(&self) -> f32 {
        self.max_rpm
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

    /// Speed-controller link: when one side turns because of something else, the other
    /// side is driven at `target` RPM. `target == 0` disables the link.
    pub fn connect_controlled(&mut self, a: NodeId, b: NodeId, target: f32) {
        self.links.push(ControlledLink { a, b, target });
    }

    pub fn solve(&self) -> Solution {
        let n = self.len();

        // 1. Rigid components: nodes tied together by fixed ratios.
        //    factor[i]: speed(i) = factor[i] * X, X = speed of the component's root.
        let mut comp_of = vec![usize::MAX; n];
        let mut factor = vec![0.0f32; n];
        let mut comps: Vec<Rigid> = Vec::new();
        for root in 0..n {
            if comp_of[root] != usize::MAX {
                continue;
            }
            let id = comps.len();
            let mut c = Rigid::default();
            comp_of[root] = id;
            factor[root] = 1.0;
            let mut queue = VecDeque::from([root]);
            while let Some(cur) = queue.pop_front() {
                c.nodes.push(cur);
                for &(next, ratio) in &self.adj[cur] {
                    let expected = factor[cur] * ratio;
                    if comp_of[next] == usize::MAX {
                        comp_of[next] = id;
                        factor[next] = expected;
                        queue.push_back(next);
                    } else if c.conflict.is_none() && !approx(factor[next], expected) {
                        c.conflict = Some((cur, next));
                    }
                }
            }
            for &i in &c.nodes {
                if let NodeRole::Source { rpm, capacity } = self.roles[i] {
                    c.capacity += capacity * rpm.abs();
                    if rpm.abs() >= EPS {
                        c.drivers.push((rpm / factor[i], i, Origin::Source));
                    }
                }
            }
            comps.push(c);
        }

        // 2. Speed controllers: a side that is driven by something other than this
        //    link drives the other side at the target speed. Drivers only get added,
        //    so this reaches a fixed point after at most 2 * links passes.
        let mut active = vec![false; self.links.len()];
        let mut added = vec![[false; 2]; self.links.len()];
        loop {
            let mut changed = false;
            for (li, l) in self.links.iter().enumerate() {
                if l.target.abs() < EPS {
                    continue;
                }
                for (dir, (from, to)) in [(l.a, l.b), (l.b, l.a)].into_iter().enumerate() {
                    if added[li][dir] {
                        continue;
                    }
                    let driven = comps[comp_of[from]]
                        .drivers
                        .iter()
                        .any(|&(_, _, o)| o != Origin::Link(li));
                    if driven {
                        let x = l.target / factor[to];
                        comps[comp_of[to]].drivers.push((x, to, Origin::Link(li)));
                        added[li][dir] = true;
                        active[li] = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // 3. Stress groups: rigid components joined by active controllers.
        let mut uf: Vec<usize> = (0..comps.len()).collect();
        for (li, l) in self.links.iter().enumerate() {
            if active[li] {
                let (ra, rb) = (find(&mut uf, comp_of[l.a]), find(&mut uf, comp_of[l.b]));
                uf[ra] = rb;
            }
        }
        let mut group_index = vec![usize::MAX; comps.len()];
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for c in 0..comps.len() {
            let r = find(&mut uf, c);
            if group_index[r] == usize::MAX {
                group_index[r] = groups.len();
                groups.push(Vec::new());
            }
            groups[group_index[r]].push(c);
        }

        // 4. Resolve each group.
        let mut speed = vec![0.0f32; n];
        let mut subnetwork_of = vec![usize::MAX; n];
        let mut subnetworks = Vec::with_capacity(groups.len());
        for (gid, members) in groups.iter().enumerate() {
            let mut conflict = None;
            let mut xs = Vec::with_capacity(members.len());
            for &ci in members {
                let c = &comps[ci];
                if conflict.is_none() {
                    conflict = c.conflict;
                }
                // The fastest driver dictates (Create behaviour); opposite directions conflict.
                let mut x: Option<(f32, NodeId)> = None;
                for &(xi, node, _) in &c.drivers {
                    match x {
                        None => x = Some((xi, node)),
                        Some((x0, j)) => {
                            if xi.signum() != x0.signum() {
                                conflict.get_or_insert((j, node));
                            } else if xi.abs() > x0.abs() {
                                x = Some((xi, node));
                            }
                        }
                    }
                }
                xs.push(x.map_or(0.0, |(v, _)| v));
            }

            let nodes: Vec<NodeId> = members
                .iter()
                .flat_map(|&ci| comps[ci].nodes.iter().copied())
                .collect();
            let capacity: f32 = members.iter().map(|&ci| comps[ci].capacity).sum();
            let mut impact = 0.0;
            let mut overspeed = None;
            for (k, &ci) in members.iter().enumerate() {
                for &i in &comps[ci].nodes {
                    let v = factor[i] * xs[k];
                    if let NodeRole::Consumer { impact: imp } = self.roles[i] {
                        impact += imp * v.abs();
                    }
                    if overspeed.is_none() && v.abs() > self.max_rpm + EPS {
                        overspeed = Some(i);
                    }
                }
            }

            let status = if let Some((a, b)) = conflict {
                Status::Conflict { a, b }
            } else if xs.iter().all(|x| x.abs() < EPS) {
                Status::Stopped
            } else if let Some(node) = overspeed {
                Status::Overspeed { node }
            } else if impact > capacity + EPS {
                Status::Overstressed
            } else {
                Status::Running
            };

            for (k, &ci) in members.iter().enumerate() {
                for &i in &comps[ci].nodes {
                    subnetwork_of[i] = gid;
                    if status == Status::Running {
                        speed[i] = factor[i] * xs[k];
                    }
                }
            }
            subnetworks.push(Subnetwork {
                nodes,
                status,
                capacity,
                impact,
            });
        }

        Solution {
            speed,
            subnetwork_of,
            subnetworks,
        }
    }
}

#[derive(Default)]
struct Rigid {
    nodes: Vec<NodeId>,
    conflict: Option<(NodeId, NodeId)>,
    /// (root speed X this driver demands, node, origin)
    drivers: Vec<(f32, NodeId, Origin)>,
    capacity: f32,
}

fn find(uf: &mut [usize], mut x: usize) -> usize {
    while uf[x] != x {
        uf[x] = uf[uf[x]];
        x = uf[x];
    }
    x
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS * a.abs().max(b.abs()).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motor(rpm: f32) -> NodeRole {
        NodeRole::Source {
            rpm,
            capacity: 16.0,
        } // creative motor-ish: 16 SU/RPM
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
    #[test]
    fn speed_controller_drives_output_at_target() {
        let mut n = Network::new();
        let m = n.add_node(motor(16.0));
        let ctrl = n.add_node(NodeRole::Passive);
        let cog = n.add_node(NodeRole::Passive);
        let out = n.add_node(NodeRole::Consumer { impact: 1.0 });
        n.connect(m, ctrl, 1.0);
        n.connect(cog, out, -2.0);
        n.connect_controlled(ctrl, cog, 100.0);
        let s = n.solve();
        assert_eq!(s.status_of(out), &Status::Running);
        assert_eq!(s.speed[cog], 100.0);
        assert_eq!(s.speed[out], -200.0);
        assert_eq!(s.speed[m], 16.0); // input side keeps its own speed
                                      // one shared stress network: 200 SU impact vs 256 SU capacity
        assert_eq!(s.subnetwork_of[m], s.subnetwork_of[out]);
        assert_eq!(s.subnetworks[s.subnetwork_of[m]].impact, 200.0);
    }

    #[test]
    fn speed_controller_needs_input_rotation() {
        let mut n = Network::new();
        let ctrl = n.add_node(NodeRole::Passive);
        let cog = n.add_node(NodeRole::Passive);
        n.connect_controlled(ctrl, cog, 64.0);
        let s = n.solve();
        assert_eq!(s.speed[cog], 0.0);
        assert_ne!(s.subnetwork_of[ctrl], s.subnetwork_of[cog]);
    }

    #[test]
    fn speed_controller_output_overstress_stops_input_too() {
        let mut n = Network::new();
        let m = n.add_node(motor(16.0)); // 256 SU
        let cog = n.add_node(NodeRole::Consumer { impact: 4.0 });
        n.connect_controlled(m, cog, 128.0); // 512 SU
        let s = n.solve();
        assert_eq!(s.status_of(m), &Status::Overstressed);
        assert_eq!(s.speed[m], 0.0);
    }

    #[test]
    fn custom_max_rpm() {
        let mut n = Network::new().with_max_rpm(64.0);
        let m = n.add_node(motor(64.0));
        let b = n.add_node(NodeRole::Passive);
        n.connect(m, b, -2.0);
        assert_eq!(n.solve().status_of(m), &Status::Overspeed { node: b });
    }
}
