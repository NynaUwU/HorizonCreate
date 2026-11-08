#!/usr/bin/env python3
"""Writes the horizon_create content mod (mod/): manifest, block declarations,
blockstates, models, item models, loot tables, recipes, tags, languages and the
creative tab. Textures come from tools/gen_textures.py.

    python3 tools/gen_textures.py && python3 tools/gen_content.py
"""
import json
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "mod"
NS = "horizon_create"
VERSION = "0.1.0"


def write(rel, data):
    p = ROOT / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(data, str):
        p.write_text(data)
    else:
        p.write_text(json.dumps(data, indent=2) + "\n")


def tex(name):
    return f"{NS}:block/{name}"


def face(texture, uv=None, cull=None):
    f = {"texture": texture}
    if uv:
        f["uv"] = uv
    if cull:
        f["cullface"] = cull
    return f


def box(frm, to, side, end, side_uv=None, end_uv=None, rotation=None):
    """A box whose four sides use `side` and whose top/bottom use `end`."""
    e = {
        "from": frm,
        "to": to,
        "faces": {
            d: face(side, side_uv) for d in ("north", "south", "east", "west")
        }
        | {d: face(end, end_uv) for d in ("up", "down")},
    }
    if rotation:
        e["rotation"] = rotation
    return e


# ------------------------------------------------------------------ models (axis = y)
SHAFT = [box([6, 0, 6], [10, 16, 10], "#axis", "#axis_end", [6, 0, 10, 16], [6, 6, 10, 10])]


def cog(radius, teeth_len):
    """Disc + 8 teeth (4 bars, two of them turned 45 degrees) + hub, axis y."""
    lo, hi = 8 - radius, 8 + radius
    t_lo, t_hi = 8 - radius - teeth_len, 8 + radius + teeth_len
    disc = box([lo, 6, lo], [hi, 10, hi], "#wood", "#wood_end")
    bars = [
        box([t_lo, 6.5, 7], [t_hi, 9.5, 9], "#wood", "#wood"),
        box([7, 6.5, t_lo], [9, 9.5, t_hi], "#wood", "#wood"),
    ]
    turned = [
        box(b["from"], b["to"], "#wood", "#wood", rotation={"angle": 45, "axis": "y", "origin": [8, 8, 8]})
        for b in bars
    ]
    hub = box([5, 5.5, 5], [11, 10.5, 11], "#hub", "#hub")
    return SHAFT + [disc, hub] + bars + turned


def model_cog(radius, teeth):
    return {
        "textures": {
            "particle": "#wood",
            "axis": tex("axis"),
            "axis_end": tex("axis_end"),
            "wood": tex("cog_wood"),
            "wood_end": tex("cog_wood_end"),
            "hub": tex("cog_hub"),
        },
        "elements": cog(radius, teeth),
    }


def model_column(side, end, particle=None):
    """Full cube: `side` on the four faces around the axis, `end` on the two axis faces."""
    return {
        "parent": "minecraft:block/cube_column",
        "textures": {"side": tex(side), "end": tex(end), "particle": tex(particle or side)},
    }


# blockstate rotations that take an axis-y model to each axis
AXIS_ROT = {"y": {}, "x": {"x": 90, "y": 90}, "z": {"x": 90}}


def axis_variants(model, extra=None):
    variants = {}
    for axis, rot in AXIS_ROT.items():
        if extra is None:
            variants[f"axis={axis}"] = {"model": model} | rot
        else:
            for key, m in extra.items():
                variants[f"axis={axis},{key}"] = {"model": m} | rot
    return {"variants": variants}


# lever-shaped hand crank: base, shaft and handle, standing on the floor, facing north
CRANK = {
    "textures": {
        "particle": tex("crank_handle"),
        "axis": tex("axis"),
        "axis_end": tex("axis_end"),
        "hub": tex("cog_hub"),
        "handle": tex("crank_handle"),
    },
    "elements": [
        box([4, 0, 4], [12, 2, 12], "#hub", "#hub"),
        box([7, 2, 7], [9, 9, 9], "#axis", "#axis_end", [7, 0, 9, 7], [7, 7, 9, 9]),
        box([7, 9, 2], [9, 11, 10], "#handle", "#handle"),
        box([6.5, 11, 2], [9.5, 15, 5], "#handle", "#handle"),
    ],
}


def crank_variants():
    """Vanilla lever layout: floor x=0, ceiling x=180, wall x=90, turned by facing."""
    y_of = {"north": 0, "east": 90, "south": 180, "west": 270}
    variants = {}
    for face_ in ("floor", "wall", "ceiling"):
        for facing, y in y_of.items():
            for powered in ("false", "true"):
                v = {"model": f"{NS}:block/hand_crank"}
                if face_ == "ceiling":
                    v["x"] = 180
                    y = (y + 180) % 360
                elif face_ == "wall":
                    v["x"] = 90
                if y:
                    v["y"] = y
                variants[f"face={face_},facing={facing},powered={powered}"] = v
    return {"variants": variants}


# ------------------------------------------------------------------ blocks
AXIS_SHAPES = {
    "cog": {
        "property": "axis",
        "values": {
            "y": [[0, 0.375, 0, 1, 0.625, 1]],
            "x": [[0.375, 0, 0, 0.625, 1, 1]],
            "z": [[0, 0, 0.375, 1, 1, 0.625]],
        },
    },
}

PILLAR = "minecraft:quartz_pillar"  # axis property, no stripping, pickaxe
METAL_SOUND = "minecraft:copper_block"
WOOD_SOUND = "minecraft:oak_planks"

BLOCKS = {
    # name: (declaration, blockstate, {model name: model}, tool, english, portuguese)
    "shaft": (
        {"template": "minecraft:iron_chain",
         "properties": {"hardness": 1.5, "sounds": METAL_SOUND, "render_shape": "ENTITYBLOCK_ANIMATED"}},
        axis_variants(f"{NS}:block/shaft"),
        {"shaft": {"textures": {"particle": tex("axis"), "axis": tex("axis"), "axis_end": tex("axis_end")},
                   "elements": SHAFT}},
        "pickaxe", "Shaft", "Eixo",
    ),
    "cogwheel": (
        {"template": "minecraft:iron_chain",
         "properties": {"hardness": 1.5, "sounds": WOOD_SOUND, "render_shape": "ENTITYBLOCK_ANIMATED"},
         "shapes": AXIS_SHAPES["cog"]},
        axis_variants(f"{NS}:block/cogwheel"),
        {"cogwheel": model_cog(5, 2)},
        "axe", "Cogwheel", "Engrenagem",
    ),
    "large_cogwheel": (
        {"template": "minecraft:iron_chain",
         "properties": {"hardness": 2.0, "sounds": WOOD_SOUND, "render_shape": "ENTITYBLOCK_ANIMATED"},
         "shapes": AXIS_SHAPES["cog"]},
        axis_variants(f"{NS}:block/large_cogwheel"),
        {"large_cogwheel": model_cog(13, 3)},
        "axe", "Large Cogwheel", "Engrenagem Grande",
    ),
    "andesite_casing": (
        {"template": "minecraft:polished_andesite", "properties": {"hardness": 2.0, "sounds": WOOD_SOUND}},
        {"variants": {"": {"model": f"{NS}:block/andesite_casing"}}},
        {"andesite_casing": {"parent": "minecraft:block/cube_all", "textures": {"all": tex("andesite_casing")}}},
        "pickaxe", "Andesite Casing", "Carcaça de Andesito",
    ),
    "gearbox": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND}},
        axis_variants(f"{NS}:block/gearbox"),
        # shafts on the four faces around the axis
        {"gearbox": model_column("gearbox", "andesite_casing")},
        "pickaxe", "Gearbox", "Caixa de Engrenagens",
    ),
    "clutch": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND},
         "state_properties": {"powered": {"type": "boolean"}}},
        axis_variants(None, {"powered=false": f"{NS}:block/clutch", "powered=true": f"{NS}:block/clutch_on"}),
        {"clutch": model_column("clutch_off", "gearbox"), "clutch_on": model_column("clutch_on", "gearbox")},
        "pickaxe", "Clutch", "Embreagem",
    ),
    "gearshift": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND},
         "state_properties": {"powered": {"type": "boolean"}}},
        axis_variants(None, {"powered=false": f"{NS}:block/gearshift", "powered=true": f"{NS}:block/gearshift_on"}),
        {"gearshift": model_column("gearshift_off", "gearbox"),
         "gearshift_on": model_column("gearshift_on", "gearbox")},
        "pickaxe", "Gearshift", "Câmbio",
    ),
    "encased_chain_drive": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND}},
        axis_variants(f"{NS}:block/encased_chain_drive"),
        {"encased_chain_drive": model_column("chain_drive_side", "gearbox")},
        "pickaxe", "Encased Chain Drive", "Transmissão por Corrente",
    ),
    "adjustable_chain_gearshift": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND}},
        axis_variants(f"{NS}:block/adjustable_chain_gearshift"),
        {"adjustable_chain_gearshift": model_column("chain_gearshift_side", "gearbox")},
        "pickaxe", "Adjustable Chain Gearshift", "Câmbio de Corrente Ajustável",
    ),
    "rotation_speed_controller": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": METAL_SOUND}},
        axis_variants(f"{NS}:block/rotation_speed_controller"),
        {"rotation_speed_controller": model_column("speed_controller", "gearbox")},
        "pickaxe", "Rotation Speed Controller", "Controlador de Rotação",
    ),
    "hand_crank": (
        {"template": "minecraft:lever", "properties": {"hardness": 1.0, "sounds": WOOD_SOUND}},
        crank_variants(),
        {"hand_crank": CRANK},
        "axe", "Hand Crank", "Manivela",
    ),
    "creative_motor": (
        {"template": PILLAR, "properties": {"hardness": -1.0, "explosion_resistance": 3600000.0,
                                            "sounds": METAL_SOUND}},
        axis_variants(f"{NS}:block/creative_motor"),
        {"creative_motor": model_column("creative_motor_side", "creative_motor_front")},
        "pickaxe", "Creative Motor", "Motor Criativo",
    ),
    "speedometer": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND}},
        axis_variants(f"{NS}:block/speedometer"),
        {"speedometer": model_column("speedometer_face", "gearbox")},
        "pickaxe", "Speedometer", "Velocímetro",
    ),
    "stressometer": (
        {"template": PILLAR, "properties": {"hardness": 2.0, "sounds": WOOD_SOUND}},
        axis_variants(f"{NS}:block/stressometer"),
        {"stressometer": model_column("stressometer_face", "gearbox")},
        "pickaxe", "Stressometer", "Medidor de Stress",
    ),
}

ITEMS = {
    "andesite_alloy": ({"template": "minecraft:iron_ingot"}, "Andesite Alloy", "Liga de Andesito"),
}


# ------------------------------------------------------------------ recipes (vanilla ingredients only)
def shaped(pattern, key, result, count=1):
    return {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": pattern,
        "key": key,
        "result": {"id": f"{NS}:{result}", "count": count},
    }


def shapeless(ingredients, result, count=1):
    return {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": ingredients,
        "result": {"id": f"{NS}:{result}", "count": count},
    }


A = f"{NS}:andesite_alloy"
CASING = f"{NS}:andesite_casing"
RECIPES = {
    "andesite_alloy": shaped(["AN", "NA"], {"A": "minecraft:andesite", "N": "minecraft:iron_nugget"}, "andesite_alloy"),
    "shaft": shaped(["A", "A"], {"A": A}, "shaft", 8),
    "cogwheel": shapeless([f"{NS}:shaft", "#minecraft:planks"], "cogwheel"),
    "large_cogwheel": shapeless([f"{NS}:shaft", "#minecraft:planks", "#minecraft:planks"], "large_cogwheel"),
    "andesite_casing": shaped(["PPP", "PAP", "PPP"], {"P": "#minecraft:planks", "A": A}, "andesite_casing", 4),
    "gearbox": shapeless([CASING, f"{NS}:cogwheel"], "gearbox"),
    "clutch": shapeless([CASING, f"{NS}:shaft", "minecraft:redstone"], "clutch"),
    "gearshift": shapeless([CASING, f"{NS}:cogwheel", "minecraft:redstone"], "gearshift"),
    "encased_chain_drive": shapeless([CASING, "minecraft:iron_chain"], "encased_chain_drive"),
    "adjustable_chain_gearshift": shapeless(
        [f"{NS}:encased_chain_drive", "minecraft:redstone", "minecraft:comparator"], "adjustable_chain_gearshift"),
    "rotation_speed_controller": shapeless(
        [CASING, f"{NS}:cogwheel", "minecraft:gold_ingot", "minecraft:comparator"], "rotation_speed_controller"),
    "hand_crank": shaped(["PPP", "  A"], {"P": "#minecraft:planks", "A": A}, "hand_crank"),
    "speedometer": shapeless([CASING, "minecraft:compass"], "speedometer"),
    "stressometer": shapeless([CASING, "minecraft:clock"], "stressometer"),
}


def loot(name):
    return {
        "type": "minecraft:block",
        "pools": [{
            "rolls": 1,
            "conditions": [{"condition": "minecraft:survives_explosion"}],
            "entries": [{"type": "minecraft:item", "name": f"{NS}:{name}"}],
        }],
        "random_sequence": f"{NS}:blocks/{name}",
    }


def main():
    for sub in ("data", "assets/horizon_create/blockstates", "assets/horizon_create/models",
                "assets/horizon_create/items", "assets/horizon_create/lang"):
        shutil.rmtree(ROOT / sub, ignore_errors=True)
    write("horizon-mod.toml", f'''id = "{NS}"
name = "Horizon Create"
version = "{VERSION}"
description = "Rotational power from Create, ported to Horizon: shafts, cogwheels, gearboxes, clutches, chain drives, gauges. Needs the horizon_create native mod for the moving parts."
authors = ["NynaUwU"]
api = "0.2"
side = "both"
capabilities = ["registries"]
''')
    en, pt = {"itemGroup.horizon_create.main": "Horizon Create"}, {"itemGroup.horizon_create.main": "Horizon Create"}
    tags = {"pickaxe": [], "axe": []}
    for name, (decl, states, models, tool, english, portuguese) in BLOCKS.items():
        write(f"data/{NS}/horizon/block/{name}.json", decl)
        write(f"assets/{NS}/blockstates/{name}.json", states)
        for model_name, model in models.items():
            write(f"assets/{NS}/models/block/{model_name}.json", model)
        write(f"assets/{NS}/items/{name}.json", {"model": {"type": "minecraft:model", "model": f"{NS}:block/{name}"}})
        write(f"data/{NS}/loot_table/blocks/{name}.json", loot(name))
        tags[tool].append(f"{NS}:{name}")
        en[f"block.{NS}.{name}"] = english
        pt[f"block.{NS}.{name}"] = portuguese
    for name, (decl, english, portuguese) in ITEMS.items():
        write(f"data/{NS}/horizon/item/{name}.json", decl)
        write(f"assets/{NS}/models/item/{name}.json",
              {"parent": "minecraft:item/generated", "textures": {"layer0": f"{NS}:item/{name}"}})
        write(f"assets/{NS}/items/{name}.json", {"model": {"type": "minecraft:model", "model": f"{NS}:item/{name}"}})
        en[f"item.{NS}.{name}"] = english
        pt[f"item.{NS}.{name}"] = portuguese
    for tool, values in tags.items():
        write(f"data/minecraft/tags/block/mineable/{tool}.json", {"values": values})
    for name, recipe in RECIPES.items():
        write(f"data/{NS}/recipe/{name}.json", recipe)
    write(f"assets/{NS}/lang/en_us.json", en)
    write(f"assets/{NS}/lang/pt_br.json", pt)
    write(f"data/{NS}/horizon/creative_tab/main.json", {
        "title": "itemGroup.horizon_create.main",
        "icon": f"{NS}:cogwheel",
        "items": [f"{NS}:andesite_alloy"] + [f"{NS}:{n}" for n in BLOCKS],
    })
    print(f"{len(BLOCKS)} blocks, {len(ITEMS)} items, {len(RECIPES)} recipes -> {ROOT}")


if __name__ == "__main__":
    main()
