# create-lite

Native Rust port of the *mechanics* of **Create** (kinetics) and **Create: Power Grid**
(electrics), aimed at [Horizon](https://github.com/horizon-rust/horizon) (Minecraft Java 26.3
rewritten in Rust).

Horizon can't run Java mods — only their data/assets carry over — so the logic is
reimplemented here. Phase 1 (this repo) is **engine-independent**: plain crates with tests,
reusable whatever Horizon's mod API ends up looking like.

## Crates

| Crate            | What it does |
|------------------|--------------|
| `kinetics`       | Rotational networks with Create's rules: shafts, small/large cogwheels, **gearbox**, **clutch**, **gearshift**, **encased chain drive**, **adjustable chain gearshift**, **belts**, **rotation speed controller**. Stress units (capacity vs impact), overstress, direction conflicts, locked gear loops, configurable overspeed (256 RPM default). `presets` = Create's default stress values. |
| `grid`           | Circuit solver (Modified Nodal Analysis). DC operating point **and per-tick transient simulation** like Power Grid: resistors/wires with max current, voltage/current sources, resistive loads (R = V²/P), **capacitors, inductors, switches, fuses**. |
| `electrokinetic` | Bridge between both: Power Grid's **electric motor** (watts -> RPM, 5-tick averaging, dynamic coil resistance from stress load). |

```sh
cargo test          # 54 tests
cargo clippy --all-targets
```

### Quick examples

```rust
use kinetics::*;

let mut w = KineticWorld::new();
w.place(Pos::new(0, 0, 0), KineticBlock {
    kind: BlockKind::LargeCogwheel, axis: Axis::Y,
    role: NodeRole::Source { rpm: 16.0, capacity: 16.0 },   // 256 SU
});
w.place(Pos::new(1, 0, 1), KineticBlock::passive(BlockKind::Cogwheel, Axis::Y));
let s = w.solve();
assert_eq!(s.speed_at(Pos::new(1, 0, 1)), Some(-32.0));     // large -> small = x2, reversed

// redstone on a gearshift: just change the block and re-solve
w.place(Pos::new(0, 1, 0), KineticBlock::passive(BlockKind::Gearshift { powered: true }, Axis::Y));
```

```rust
use grid::*;

let mut c = Circuit::new();
let a = c.add_node();
let cap = c.add(Element::Capacitor { a, b: GROUND, farads: 0.1 });
c.add(Element::Resistor { a, b: GROUND, ohms: 10.0, max_current: None });
c.set_stored(cap, 100.0);           // charged to 100 V
let s = c.step(TICK).unwrap();      // one game tick later: ~95 V
```

The full motor loop (circuit step -> motor tick -> kinetics solve -> load feedback)
is in `electrokinetic/tests/motor_loop.rs`.

### Modelling notes

- **Split shafts** (clutch, gearshift) are two network nodes, one per half; a powered
  clutch removes the link, a powered gearshift makes it -1. This is equivalent to
  Create's source-relative `getRotationSpeedModifier` without tracking sources.
- **Gearbox** ports: opposite faces reverse; perpendicular faces reverse when both point
  the same way along their axes (`GearboxBlockEntity` axis modifier). Use
  `WorldSolution::face_speed` to read a specific face.
- **Speed controller**: a "controlled link" — whichever side is driven by something
  else forces the other to the target RPM; both sides share one stress network.
- **Belts** are explicit links between two parallel shafts (`KineticWorld::link_belt`).

## Roadmap

1. **Core crates** ✅ `kinetics` + `grid` + `electrokinetic`
   - ✅ gearbox, clutch/gearshift, chain drives, belts, speed controller, presets
   - ✅ transient circuits (C, L, switch, fuse); ✅ electric motor
   - next (Power Grid): **generator** (rotor assembly with inertia + PD controller
     from `RotorBehaviour`, `GeneratorClutch` force source, windings/field coil,
     `GeneratorCoupling` back-EMF), **diodes / PN junctions** (Newton-Raphson),
     **transformers** (`TransformerCoupling`), **thermal model** (`ThermalBehaviour`:
     wires/motors heat up and burn), constant-speed motor
   - next (Create): encased cogwheels, sequenced gearshift, flywheel/clock stress quirks
2. **`.hmod` content mod** — shaft, cogwheel, large cogwheel, gearbox, clutch, gearshift,
   hand crank, creative motor, stress gauge; blocks via `data/<id>/horizon/block/*.json`
   (`state_properties` for axis/powered, `connects_to`, `render_shape: ENTITYBLOCK_ANIMATED`).
   ⚠️ **Create's assets are All Rights Reserved** (only its code is MIT): models and
   textures must be made from scratch. Power Grid is Apache-2.0 (assets included,
   as far as its repo shows) — keep its NOTICE if reusing anything. See `NOTICE.md`.
3. **`.hnmod` native mod** — glue between Horizon and these crates:

   | Horizon hook (native-mods.md) | Use |
   |---|---|
   | `native_block_changed` (draft)    | place/remove blocks in `KineticWorld` / `Circuit`, mark dirty |
   | `native_neighbor_changed` (draft) | redstone: clutch/gearshift `powered`, chain gearshift `signal`, switches |
   | `native_server_tick` (API 4)      | `Circuit::step(TICK)`, motor ticks, re-solve dirty kinetic networks |
   | `native_server_block`             | read block states on load / validation |
   | `native_tooltip` (API 12)         | goggles-style "stress X / Y SU", voltage, current |
   | `native_world_dir`                | persist network data (capacitor charge, motor RPM, blown fuses) |

4. **Upstream PRs to Horizon** for hooks that don't exist yet: block-entity ticking,
   custom block-entity data (NBT), block-entity renderer (rotating shafts/cogs).
5. Later: contraptions, trains (need engine-level work).

### Known Horizon limitations (as of its docs, Nov 2026)

- Native mods aren't built by dedicated servers yet, and only client-side hooks run
  when joining a server → **singleplayer only** for now.
- Neighbour/block-change hooks are drafts; the API may change.

## Accuracy notes

- Connection rules and ratios are ported line by line from Create's `RotationPropagator`
  (checked against the default branch @ 4148fd8, Oct 2026); still worth a side-by-side in-game test.
- Multiple same-direction sources: the fastest dictates the speed; every source adds
  `capacity × |its own rpm|` to the network capacity. Opposite directions = conflict.
- Create breaks the offending block on conflict/overspeed; here the whole network is
  reported (`Status::Conflict` / `Overspeed`) and the game layer decides what to pop.
- Power Grid caps capacitor/inductor state with a `0.99999` leak per tick; not copied
  (pure physics here) — add it in the game layer if save/load drift shows up.
