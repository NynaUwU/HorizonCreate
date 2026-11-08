#!/usr/bin/env python3
"""Offline checks for the horizon_create content mod, mirroring what Horizon's
loader refuses (unknown keys, missing files) as far as we can without the game.

    python3 tools/check_mod.py        # exit 1 on any problem
"""
import json
import sys
import tomllib
from itertools import product
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "mod"
NS = "horizon_create"
errors = []


def err(msg):
    errors.append(msg)


def load(p):
    try:
        return json.loads(p.read_text())
    except Exception as e:  # noqa: BLE001
        err(f"{p.relative_to(ROOT)}: bad JSON: {e}")
        return None


MANIFEST_KEYS = {"id", "name", "version", "description", "authors", "api", "side", "capabilities", "namespaces"}
BLOCK_KEYS = {"template", "properties", "shapes", "collision_shapes", "outline_context_shapes",
              "collision_context_shapes", "interaction_shapes", "state_properties", "default_state", "item",
              "components", "connects_to", "thickness", "wood_type", "wall_block"}
PROP_KEYS = {"hardness", "explosion_resistance", "light", "requires_tool", "friction", "map_color", "sounds",
             "random_ticks", "render_shape", "occludes"}
# vanilla state properties of the templates we use
TEMPLATE_STATES = {
    "minecraft:iron_chain": {"axis": ["x", "y", "z"], "waterlogged": ["false", "true"]},
    "minecraft:quartz_pillar": {"axis": ["x", "y", "z"]},
    "minecraft:polished_andesite": {},
    "minecraft:lightning_rod": {"facing": ["north", "east", "south", "west", "up", "down"],
                                "powered": ["false", "true"], "waterlogged": ["false", "true"]},
    "minecraft:lever": {"face": ["floor", "wall", "ceiling"], "facing": ["north", "south", "west", "east"],
                        "powered": ["false", "true"]},
}
IGNORED_IN_BLOCKSTATES = {"waterlogged"}


def model_path(ref):
    ns, path = ref.split(":", 1) if ":" in ref else ("minecraft", ref)
    return ns, ROOT / f"assets/{ns}/models/{path}.json"


def check_model(ref, seen):
    if ref in seen:
        return
    seen.add(ref)
    ns, p = model_path(ref)
    if ns != NS:
        return  # vanilla parent
    if not p.exists():
        err(f"missing model {ref}")
        return
    m = load(p)
    if m is None:
        return
    if "parent" in m:
        check_model(m["parent"], seen)
    textures = m.get("textures", {})
    for key, t in textures.items():
        if t.startswith("#"):
            if t[1:] not in textures:
                err(f"{ref}: texture #{key} -> {t} unresolved")
            continue
        tns, tpath = t.split(":", 1)
        if tns == NS and not (ROOT / f"assets/{tns}/textures/{tpath}.png").exists():
            err(f"{ref}: missing texture {t}")
    for e in m.get("elements", []):
        for c in e["from"] + e["to"]:
            if not -16 <= c <= 32:
                err(f"{ref}: element coordinate {c} outside -16..32")
        rot = e.get("rotation")
        if rot and rot["angle"] not in (-45, -22.5, 0, 22.5, 45):
            err(f"{ref}: rotation angle {rot['angle']}")
        for fname, f in e["faces"].items():
            if f["texture"].startswith("#") and f["texture"][1:] not in textures:
                err(f"{ref}: face {fname} uses undefined {f['texture']}")


def main():
    manifest = tomllib.loads((ROOT / "horizon-mod.toml").read_text())
    if set(manifest) - MANIFEST_KEYS:
        err(f"manifest: unknown keys {set(manifest) - MANIFEST_KEYS}")
    if manifest.get("id") != NS or manifest.get("side") != "both" or "registries" not in manifest.get("capabilities", []):
        err("manifest: id/side/capabilities")

    ids = set()
    models_seen = set()
    lang = load(ROOT / f"assets/{NS}/lang/en_us.json") or {}
    pt = load(ROOT / f"assets/{NS}/lang/pt_br.json") or {}
    for p in sorted((ROOT / f"data/{NS}/horizon/block").glob("*.json")):
        name = p.stem
        ids.add(f"{NS}:{name}")
        d = load(p) or {}
        if set(d) - BLOCK_KEYS:
            err(f"block {name}: unknown keys {set(d) - BLOCK_KEYS}")
        if set(d.get("properties", {})) - PROP_KEYS:
            err(f"block {name}: unknown properties {set(d['properties']) - PROP_KEYS}")
        tpl = d.get("template")
        if tpl not in TEMPLATE_STATES:
            err(f"block {name}: template {tpl} not in the checked list")
            continue
        states = {k: v for k, v in TEMPLATE_STATES[tpl].items() if k not in IGNORED_IN_BLOCKSTATES}
        for k, sp in d.get("state_properties", {}).items():
            if k in TEMPLATE_STATES[tpl]:
                err(f"block {name}: declared property {k} clashes with the template")
            states[k] = ["false", "true"] if sp["type"] == "boolean" else sp["values"]
        shapes = d.get("shapes")
        if shapes and "property" in shapes:
            if set(shapes["values"]) != set(states.get(shapes["property"], [])):
                err(f"block {name}: shapes don't cover {shapes['property']}")
        bs = load(ROOT / f"assets/{NS}/blockstates/{name}.json")
        if bs is None:
            err(f"block {name}: no blockstate")
            continue
        variants = bs["variants"]
        keys = sorted(states)
        for combo in product(*(states[k] for k in keys)):
            want = dict(zip(keys, combo))
            match = [v for vk, v in variants.items()
                     if all(want.get(kv.split("=")[0]) == kv.split("=")[1] for kv in vk.split(",") if kv)]
            if not match:
                err(f"block {name}: no variant for {want}")
        for v in variants.values():
            check_model(v["model"], models_seen)
        if f"block.{NS}.{name}" not in lang or f"block.{NS}.{name}" not in pt:
            err(f"block {name}: missing translation")
        if not (ROOT / f"data/{NS}/loot_table/blocks/{name}.json").exists():
            err(f"block {name}: no loot table")
    for p in sorted((ROOT / f"data/{NS}/horizon/item").glob("*.json")):
        ids.add(f"{NS}:{p.stem}")
        if f"item.{NS}.{p.stem}" not in lang:
            err(f"item {p.stem}: missing translation")
    for p in sorted((ROOT / f"assets/{NS}/items").glob("*.json")):
        if f"{NS}:{p.stem}" not in ids:
            err(f"item model for unknown {p.stem}")
        check_model((load(p) or {})["model"]["model"], models_seen)
    for p in sorted((ROOT / f"data/{NS}/recipe").glob("*.json")):
        r = load(p) or {}
        if r["result"]["id"] not in ids:
            err(f"recipe {p.stem}: unknown result")
        used = list(r.get("key", {}).values()) + r.get("ingredients", [])
        for i in used:
            if i.startswith(NS) and i not in ids:
                err(f"recipe {p.stem}: unknown ingredient {i}")
    for p in sorted((ROOT / "data/minecraft/tags/block").rglob("*.json")):
        for v in (load(p) or {})["values"]:
            if v not in ids:
                err(f"tag {p.name}: unknown {v}")
    for ref in (f"{NS}:block/shaft", f"{NS}:block/cogwheel", f"{NS}:block/large_cogwheel", f"{NS}:block/drill_head"):
        check_model(ref, models_seen)
    tab = load(ROOT / f"data/{NS}/horizon/creative_tab/main.json") or {}
    for i in tab.get("items", []) + [tab.get("icon")]:
        if i not in ids:
            err(f"creative tab: unknown {i}")
    # every file must sit in an allowed namespace
    for p in ROOT.rglob("*"):
        if p.is_file() and p.name != "horizon-mod.toml":
            parts = p.relative_to(ROOT).parts
            if parts[0] not in ("assets", "data") or parts[1] not in (NS, "minecraft", "c"):
                err(f"file outside the mod's namespaces: {p.relative_to(ROOT)}")
            if parts[1] == "minecraft" and parts[0] == "data" and parts[2] != "tags":
                err(f"only tags may go in data/minecraft: {p.relative_to(ROOT)}")
    print(f"checked {len(ids)} ids, {len(models_seen)} models")
    for e in errors:
        print("ERROR:", e)
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
