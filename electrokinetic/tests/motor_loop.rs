//! Full per-tick loop: a voltage source powers an electric motor which drives a
//! mechanical press through a shaft. This is the pattern the Horizon native mod
//! will run in `native_server_tick`.

use electrokinetic::ElectricMotor;
use grid::{Circuit, Element, GROUND, TICK};
use kinetics::presets::{impact, machine};
use kinetics::{Axis, BlockKind, KineticBlock, KineticWorld, Pos, Status};

#[test]
fn motor_spins_up_and_drives_a_press() {
    // electric side: 230 V supply -> wire -> motor coil
    let mut circuit = Circuit::new();
    let supply = circuit.add_node();
    let terminal = circuit.add_node();
    circuit.add(Element::VoltageSource {
        pos: supply,
        neg: GROUND,
        volts: 230.0,
    });
    circuit.add(Element::wire(supply, terminal, 0.05, 50.0));
    let mut motor = ElectricMotor::default();
    let coil = circuit.add(Element::Resistor {
        a: terminal,
        b: GROUND,
        ohms: motor.coil_resistance(),
        max_current: None,
    });

    // kinetic side: motor shaft -> shaft -> press
    let motor_pos = Pos::new(0, 0, 0);
    let press_pos = Pos::new(2, 0, 0);
    let mut world = KineticWorld::new();
    world.place(
        motor_pos,
        KineticBlock::passive(BlockKind::Shaft, Axis::X).with_role(motor.role(1.0)),
    );
    world.place(
        Pos::new(1, 0, 0),
        KineticBlock::passive(BlockKind::Shaft, Axis::X),
    );
    world.place(
        press_pos,
        KineticBlock::passive(BlockKind::Shaft, Axis::X)
            .with_role(machine(impact::MECHANICAL_PRESS)),
    );

    let mut idle_current = None;
    let mut solved = world.solve();
    for tick in 0..20 {
        let e = circuit.step(TICK).unwrap();
        if tick == 0 {
            idle_current = Some(e.currents[coil]);
        }
        let volts = e.voltages[terminal];
        if motor.tick(volts, e.currents[coil]).is_some() {
            world.get_mut(motor_pos).unwrap().role = motor.role(1.0);
            solved = world.solve();
        }
        let (imp, cap) = solved.stress_at(motor_pos).unwrap();
        motor.update_load(imp, cap);
        circuit.set(
            coil,
            Element::Resistor {
                a: terminal,
                b: GROUND,
                ohms: motor.coil_resistance(),
                max_current: None,
            },
        );
    }

    // ~230 V at the terminals -> ~202 RPM
    let rpm = solved.speed_at(press_pos).unwrap();
    assert!((200.0..=203.0).contains(&rpm), "rpm = {rpm}");
    assert_eq!(solved.status_at(press_pos), Some(&Status::Running));

    // press 8 SU/RPM vs motor 64 SU/RPM -> 12.5 % load (idle was the 5 % minimum)
    let loaded_current = circuit.step(TICK).unwrap().currents[coil];
    assert!((motor.load() - 0.125).abs() < 1e-6);
    let ratio = loaded_current / idle_current.unwrap();
    assert!(
        (ratio - 2.5).abs() < 0.01,
        "0.125 / 0.05 = 2.5x idle current, got {ratio}"
    );
}
