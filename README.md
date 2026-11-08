# create-lite

Native Rust port of the *mechanics* of **Create** (kinetics) and **Create: Power Grid**
(electrics), aimed at [Horizon](https://github.com/horizon-rust/horizon) (Minecraft Java 26.3
rewritten in Rust).

Horizon can't run Java mods — only their data/assets carry over — so the logic is
reimplemented here. Phase 1 (this repo) is **engine-independent**: plain crates with tests,
reusable whatever Horizon's mod API ends up looking like.

## Crates

| Crate      | What it does |
|------------|--------------|
| `kinetics` | Rotational networks: RPM propagation with ratios, stress units (capacity vs impact), overstress, direction conflicts, locked gear loops, 256 RPM overspeed. Block layer with Create's rules: shafts, small/large cogwheels (incl. large-to-large at right angles). |
| `grid`     | DC circuit solver (Modified Nodal Analysis): wires with resistance + max current, ideal voltage/current sources, loads by nominal power (R = V²/P), per-element current/power, overloaded-wire report, floating-node tolerance. |

```sh
cargo test          # 26 tests
cargo clippy --all-targets
```

### Quick example

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
```

## Roadmap

1. **Core crates** ✅ `kinetics` + `grid`
   - next: gearbox, encased belts, clutch/gearshift, rotation speed controller;
     AC/transient (capacitors, inductors) in `grid` if Power Grid needs them;
     kinetic <-> electric bridge (motor/generator: RPM ⇄ watts).
2. **`.hmod` content mod** — shaft, cogwheel, large cogwheel, hand crank, creative motor,
   stress gauge; blocks via `data/<id>/horizon/block/*.json` (`state_properties` for
   axis, `connects_to`, `render_shape: ENTITYBLOCK_ANIMATED`).
   ⚠️ Check Create's and Power Grid's asset licences before reusing their models/textures.
3. **`.hnmod` native mod** — glue between Horizon and these crates:

   | Horizon hook (native-mods.md) | Use |
   |---|---|
   | `native_block_changed` (draft)    | place/remove blocks in `KineticWorld` / `Circuit`, mark dirty |
   | `native_neighbor_changed` (draft) | redstone-controlled parts (clutch, gearshift, breakers) |
   | `native_server_tick` (API 4)      | re-solve dirty networks, apply machine effects |
   | `native_server_block`             | read block states on load / validation |
   | `native_tooltip` (API 12)         | goggles-style "stress X / Y SU", voltage, current |
   | `native_world_dir`                | persist network data (no documented BE-NBT hook yet) |

4. **Upstream PRs to Horizon** for hooks that don't exist yet: block-entity ticking,
   custom block-entity data (NBT), block-entity renderer (rotating shafts/cogs).
5. Later: contraptions, trains (need engine-level work).

### Known Horizon limitations (as of its docs, Nov 2026)

- Native mods aren't built by dedicated servers yet, and only client-side hooks run
  when joining a server → **singleplayer only** for now.
- Neighbour/block-change hooks are drafts; the API may change.

## Accuracy notes

- Gear ratio signs follow Create's `RotationPropagator` as closely as known; verify
  in-game against Create 6.x and adjust the tests if anything differs.
- Multiple same-direction sources: the fastest dictates the speed; every source adds
  `capacity × |its own rpm|` to the network capacity. Opposite directions = conflict.
