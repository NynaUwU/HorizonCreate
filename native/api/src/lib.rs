//! horizon_create native trait mod: the Horizon side of the kinetics port.
//!
//! All the logic lives in `engine` (copies of the `kinetics` and `hcglue` crates,
//! tested in this repository); this file only moves data between Horizon's hook
//! points and it:
//!
//! - server: `level_tick_pre` -> each block entity's `block_entity_tick` reports its
//!   block and stores the last readout in its NBT -> `level_tick_post` solves;
//! - client: `block_entity_submit` draws shafts and cogwheels turning (`spin`) and the
//!   gauges' readings, from what the server sent (`block_entity_update_tag`).
//!
//! Written against Horizon's mod API (`horizon_client::mod_api`); it is compiled by
//! the launcher together with the game.

mod engine;

use engine::hcglue::drill::{self, DrillAction, DrillState};
use engine::hcglue::setup;
use engine::hcglue::{self, Engine, Readout, Spinner};
use engine::kinetics::Axis;
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, Hooks, Host, LevelTickPostEvent,
    LevelTickPreEvent, ModBlockEntity, ModsEvent, NativeMod, PlayerLoggedInEvent, StandaloneModelsEvent, Tag, Text,
};
use std::collections::HashMap;
use std::sync::Mutex;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Mod::default())
}

#[derive(Default)]
struct Mod {
    /// Server: every level's kinetic network.
    engine: Mutex<Engine>,
    /// Server: the level being ticked (block entity ticks don't say which).
    dimension: Mutex<String>,
    /// Client: angle continuity of every turning part.
    spinners: Mutex<HashMap<[i32; 3], Spinner>>,
}

const KEYS: [&str; 4] = ["rpm", "impact", "capacity", "status"];
const DRILL_HEAD: &str = "horizon_create:block/drill_head";

fn read<'a>(nbt: &'a Compound, key: &str) -> Option<&'a Tag> {
    nbt.iter().find(|(k, _)| k.0.iter().copied().eq(key.encode_utf16())).map(|(_, v)| v)
}

fn read_f32(nbt: &Compound, key: &str) -> f32 {
    match read(nbt, key) {
        Some(Tag::Float(bits)) => f32::from_bits(*bits),
        Some(Tag::Byte(b)) => *b as f32,
        _ => 0.0,
    }
}

fn read_i32(nbt: &Compound, key: &str) -> i32 {
    match read(nbt, key) {
        Some(Tag::Int(v)) => *v,
        _ => 0,
    }
}

fn drill_state_of(nbt: &Compound) -> DrillState {
    DrillState {
        progress: read_i32(nbt, "drill_progress"),
        ticks_until: read_i32(nbt, "drill_ticks"),
        lazy: read_i32(nbt, "drill_lazy"),
    }
}

/// Unit offset and axis vector of a `facing` property value.
fn facing_offset(facing: &str) -> Option<([i32; 3], [f32; 3])> {
    Some(match facing {
        "east" => ([1, 0, 0], [1.0, 0.0, 0.0]),
        "west" => ([-1, 0, 0], [-1.0, 0.0, 0.0]),
        "up" => ([0, 1, 0], [0.0, 1.0, 0.0]),
        "down" => ([0, -1, 0], [0.0, -1.0, 0.0]),
        "south" => ([0, 0, 1], [0.0, 0.0, 1.0]),
        "north" => ([0, 0, -1], [0.0, 0.0, -1.0]),
        _ => return None,
    })
}

fn readout_of(nbt: &Compound) -> Readout {
    Readout {
        rpm: read_f32(nbt, "rpm"),
        impact: read_f32(nbt, "impact"),
        capacity: read_f32(nbt, "capacity"),
        status: read_f32(nbt, "status") as u8,
    }
}

fn readout_tags(r: &Readout) -> [(&'static str, Tag); 4] {
    [
        (KEYS[0], Tag::Float(r.rpm.to_bits())),
        (KEYS[1], Tag::Float(r.impact.to_bits())),
        (KEYS[2], Tag::Float(r.capacity.to_bits())),
        (KEYS[3], Tag::Byte(r.status as i8)),
    ]
}

fn spinning_model(name: &str) -> Option<&'static str> {
    hcglue::SPINNING_MODELS.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
}

/// Shown in Options > Mods, so players can see the native half is in their build.
/// Its own id: the content mod's row (`horizon_create`) keeps its switch.
const NATIVE_ID: &str = "horizon_create_native";
const VERSION: &str = "0.2.4";

impl Hooks for Mod {
    /// Without the content mod nothing shows up and the game only logs missing models:
    /// say which step is missing when a player joins.
    fn player_logged_in(&self, host: &mut dyn Host, event: &mut PlayerLoggedInEvent<'_>) {
        let instance = host.instance_dir();
        if let Some(line) = setup::status_message(setup::content_status(&instance), &instance) {
            host.tell(event.player, Text::literal(line));
        }
    }

    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent<'_>) {
        event.mods.push(NativeMod {
            id: NATIVE_ID.to_string(),
            title: format!("Horizon Create (native) {VERSION}"),
            description: "Kinetics for Horizon Create: turning shafts, gauges and machines. \
                          Needs the Horizon Create content mod turned on."
                .to_string(),
            keys: Vec::new(),
        });
    }

    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for (_, model) in hcglue::SPINNING_MODELS {
            event.models.push((*model).to_string());
        }
        event.models.push(DRILL_HEAD.to_string());
    }

    // "replace" hook: return the block entity (id, saved compound) for our blocks
    fn block_entity(&self, _host: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        hcglue::local_name(event.block).map(|_| (event.block.to_string(), Compound::new()))
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if hcglue::local_name(event.block).is_some() {
            event.ticks = true;
        }
    }

    fn level_tick_pre(&self, _host: &mut dyn Host, event: &mut LevelTickPreEvent<'_>) {
        *self.dimension.lock().unwrap() = event.dimension.to_string();
        self.engine.lock().unwrap().begin_level(event.dimension);
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        let pos = event.pos;
        let id = host.block(pos);
        let Some(name) = hcglue::local_name(&id) else { return };
        let dimension = self.dimension.lock().unwrap().clone();
        let signal = host.neighbor_signal(&dimension, pos);

        // clutch / gearshift show their state with the `powered` property
        if matches!(name, "clutch" | "gearshift") {
            let want = if signal > 0 { "true" } else { "false" };
            if host.block_property(pos, "powered").as_deref() != Some(want) {
                host.set_block_property(pos, "powered", want);
            }
        }

        let props = |key: &str| host.block_property(pos, key);
        let block = hcglue::block_from_state(&id, &props, signal);
        let readout = {
            let mut engine = self.engine.lock().unwrap();
            engine.report(pos, block);
            engine.readout(pos)
        };

        // only touch the entity when something changed: a mutable borrow marks it
        // changed and sends it to clients again
        let entity: &mut ModBlockEntity = &mut *event.entity;
        if readout_of(&entity.nbt) != readout {
            for (key, tag) in readout_tags(&readout) {
                entity.nbt.insert(key.into(), tag);
            }
            // Changes made in a block entity's tick aren't sent on their own (as in vanilla):
            // ask Horizon to send the update tag again, so clients see the new speed.
            host.block_entity_sync(pos);
        }

        if name == "mechanical_drill" {
            let Some((off, _)) = host.block_property(pos, "facing").as_deref().and_then(facing_offset) else {
                return;
            };
            let target = [pos[0] + off[0], pos[1] + off[1], pos[2] + off[2]];
            let hardness = drill::hardness(&host.block(target));
            let mut state = drill_state_of(&entity.nbt);
            let before = state;
            let breaker = drill::breaker_id(pos);
            match state.tick(readout.rpm, hardness) {
                DrillAction::Nothing => {}
                DrillAction::Crack(stage) => host.set_break_progress(breaker, target, stage),
                DrillAction::Reset => host.set_break_progress(breaker, target, -1),
                DrillAction::Break => {
                    host.set_break_progress(breaker, target, -1);
                    host.destroy_block(target, None, false);
                }
            }
            if state != before {
                entity.nbt.insert("drill_progress".into(), Tag::Int(state.progress));
                entity.nbt.insert("drill_ticks".into(), Tag::Int(state.ticks_until));
                entity.nbt.insert("drill_lazy".into(), Tag::Int(state.lazy));
            }
        }
    }

    fn level_tick_post(&self, _host: &mut dyn Host, _event: &mut LevelTickPostEvent<'_>) {
        self.engine.lock().unwrap().end_level();
    }

    // "replace" hook: return what clients get of our block entities
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        hcglue::local_name(&event.entity.id)?;
        let r = readout_of(&event.entity.nbt);
        Some(readout_tags(&r).into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    fn block_entity_render_bounding_box(
        &self,
        _host: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        match hcglue::local_name(event.block) {
            // large cogwheels reach a block and a half from their centre
            Some("large_cogwheel") => *event.bounds = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0],
            // gauge text floats above the block
            Some("speedometer" | "stressometer") => event.bounds[4] = 2.0,
            _ => {}
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        let Some(name) = hcglue::local_name(event.block) else { return };
        let readout = readout_of(&event.entity.nbt);
        let draw = &mut *event.draw;

        if let Some(model) = spinning_model(name) {
            let axis = match host.block_state_property(event.state, "axis").as_deref() {
                Some("x") => Axis::X,
                Some("z") => Axis::Z,
                _ => Axis::Y,
            };
            let spin = self.spinners.lock().unwrap().entry(event.pos).or_default().update(readout.rpm, draw.game_time);
            let centre = [0.5, 0.5, 0.5];
            draw.push();
            draw.spin(hcglue::axis_vector(axis), centre, spin.degrees_per_tick, spin.phase);
            // the models are built around Y: turn them onto the block's axis
            match axis {
                Axis::X => draw.rotate_about([0.0, 0.0, 1.0], -90.0, centre),
                Axis::Z => draw.rotate_about([1.0, 0.0, 0.0], 90.0, centre),
                Axis::Y => {}
            }
            host.draw_model(draw, model, 0xFFFFFF);
            draw.still();
            draw.pop();
        }

        if name == "mechanical_drill" {
            let facing = host.block_state_property(event.state, "facing").map(|f| f.to_string());
            if let Some((_, dir)) = facing.as_deref().and_then(facing_offset) {
                let spin = self.spinners.lock().unwrap().entry(event.pos).or_default().update(readout.rpm, draw.game_time);
                let centre = [0.5, 0.5, 0.5];
                draw.push();
                draw.spin(dir, centre, spin.degrees_per_tick, spin.phase);
                // the bit model points up: turn it to the drill's facing
                match facing.as_deref() {
                    Some("down") => draw.rotate_about([1.0, 0.0, 0.0], 180.0, centre),
                    Some("north") => draw.rotate_about([1.0, 0.0, 0.0], -90.0, centre),
                    Some("south") => draw.rotate_about([1.0, 0.0, 0.0], 90.0, centre),
                    Some("east") => draw.rotate_about([0.0, 0.0, 1.0], -90.0, centre),
                    Some("west") => draw.rotate_about([0.0, 0.0, 1.0], 90.0, centre),
                    _ => {}
                }
                host.draw_model(draw, DRILL_HEAD, 0xFFFFFF);
                draw.still();
                draw.pop();
            }
        }

        if let Some(line) = hcglue::gauge_text(name, &readout) {
            let color = if readout.status == hcglue::status::OVERSTRESSED { 0xFFFF5555 } else { 0xFFFFFFFF };
            let text = Text::literal(line.clone());
            let half_width = line.chars().count() as f32 * 3.0;
            // the same line on the four sides, like a sign seen from anywhere
            for quarter in 0..4 {
                draw.push();
                draw.rotate_about([0.0, 1.0, 0.0], 90.0 * quarter as f32, [0.5, 0.5, 0.5]);
                draw.translate([0.5, 1.35, 0.5]);
                draw.scale([1.0 / 64.0, -1.0 / 64.0, 1.0 / 64.0]);
                host.draw_text(draw, &text, [-half_width, 0.0], color, false, 0xFF000000);
                draw.pop();
            }
        }
    }
}
