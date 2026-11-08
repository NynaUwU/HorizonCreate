//! A Create shaft drives a Power Grid generator that lights a 10 Ω load: the full
//! per-tick loop (kinetics -> clutch -> rotor -> circuit -> braking force).

use electrokinetic::generator::*;
use grid::{Circuit, Element, GROUND, TICK};
use kinetics::presets::{creative_motor, hand_crank};
use kinetics::{Network, NodeRole, Status};

struct Run {
    rotor_rpm: f32,
    load_watts: f64,
    status: Status,
    clutch_load: f32,
    mech_watts: f64,
    electric_watts: f64,
}

fn run(source: NodeRole, ticks: usize) -> Run {
    let p = GeneratorParams::default();
    let mut rotor = Rotor::new(&[inertia::CLUTCH, inertia::INDUCTION_ROTOR]);
    let mut clutch = GeneratorClutch::new(impact::CLUTCH + impact::INDUCTION_ROTOR);
    // four windings around the rotor, each with 2 A in its field coil
    let f = field_strength(&p, 2.0);
    let coupling = Coupling {
        base_resistance: WINDING_RESISTANCE,
        field: rotor_field([Some(f); 4], false),
    };

    let mut net = Network::new();
    let src = net.add_node(source);
    let clutch_node = net.add_node(NodeRole::Consumer {
        impact: clutch.stress_impact(),
    });
    net.connect(src, clutch_node, 1.0);

    let mut c = Circuit::new();
    let emf_node = c.add_node();
    let out = c.add_node();
    let emf = c.add(Element::VoltageSource {
        pos: emf_node,
        neg: GROUND,
        volts: 0.0,
    });
    let r_int = c.add(Element::Resistor {
        a: emf_node,
        b: out,
        ohms: 1.0,
        max_current: None,
    });
    let load = c.add(Element::Resistor {
        a: out,
        b: GROUND,
        ohms: 10.0,
        max_current: None,
    });

    let mut status = Status::Stopped;
    let mut load_watts = 0.0;
    let (mut mech, mut elec) = (0.0, 0.0);
    for _ in 0..ticks {
        let s = net.solve();
        status = s.status_of(clutch_node).clone();
        clutch.shaft_rpm = s.speed[clutch_node];
        clutch.overstressed = status == Status::Overstressed;
        rotor.tick(&p, Some(&mut clutch));

        c.set(
            emf,
            Element::VoltageSource {
                pos: emf_node,
                neg: GROUND,
                volts: coupling.emf(&rotor),
            },
        );
        c.set(
            r_int,
            Element::Resistor {
                a: emf_node,
                b: out,
                ohms: coupling.resistance(&rotor, TICK),
                max_current: None,
            },
        );
        let e = c.step(TICK).unwrap();
        coupling.apply_current(&mut rotor, e.currents[emf]);
        load_watts = e.power[load];
        mech = rotor.power as f64;
        elec = -e.power[emf];
    }
    Run {
        rotor_rpm: rotor.angular_velocity,
        load_watts,
        status,
        clutch_load: clutch.load,
        mech_watts: mech,
        electric_watts: elec,
    }
}

#[test]
fn creative_motor_drives_generator_under_load() {
    let r = run(creative_motor(64.0), 400);
    assert_eq!(r.status, Status::Running);
    // under load the rotor runs a bit below the shaft, but close
    assert!(
        r.rotor_rpm > 50.0 && r.rotor_rpm <= 64.0,
        "rotor {}",
        r.rotor_rpm
    );
    // EMF ~ 9.5 * 6.7 rad/s ~ 60 V; internal 7.5 Ohm winding + 7.6 Ohm back-EMF term,
    // plus the 10 Ohm load -> ~2.4 A -> ~60 W in the load
    assert!(
        r.load_watts > 40.0 && r.load_watts < 90.0,
        "load {} W",
        r.load_watts
    );
    // energy balance: what the shaft puts in covers the electrical output
    assert!(
        r.mech_watts >= r.electric_watts * 0.95,
        "mech {} W < electric {} W",
        r.mech_watts,
        r.electric_watts
    );
    assert!(
        r.clutch_load > 0.1 && r.clutch_load <= 1.0,
        "clutch load {}",
        r.clutch_load
    );
}

#[test]
fn weak_source_overstresses_and_generator_stops() {
    // hand crank: 256 SU; generator: 64 SU/RPM * 32 RPM = 2048 SU -> overstressed
    let r = run(hand_crank(false), 200);
    assert_eq!(r.status, Status::Overstressed);
    assert_eq!(r.rotor_rpm, 0.0);
    assert!(r.load_watts.abs() < 1e-9);
}
