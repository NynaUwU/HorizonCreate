#!/usr/bin/env python3
"""Procedural 16x16 textures for the horizon_create content mod.

Same approach as CounterBlox's `tile_px` and GamaMake's detail textures: every
pixel comes from code (hash noise, panel borders, rivets, plank rows with their
own tone, wood grain), nothing is painted by hand and nothing is copied from
Create. Re-run after editing; the output is deterministic.

    python3 tools/gen_textures.py
"""
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent.parent / "mod/assets/horizon_create/textures"
N = 16


def hash32(x: int) -> int:
    x &= 0xFFFFFFFF
    x ^= x >> 16
    x = (x * 0x7FEB352D) & 0xFFFFFFFF
    x ^= x >> 15
    x = (x * 0x846CA68B) & 0xFFFFFFFF
    x ^= x >> 16
    return x


def noise(x: int, y: int, seed: int) -> float:
    """0..1, like CounterBlox's tex_noise."""
    return (hash32((x * 73856093) ^ (y * 19349663) ^ (seed * 83492791)) & 255) / 255.0


def shade(rgb, k: float):
    return tuple(max(0, min(255, int(c * k))) for c in rgb)


def mix(a, b, t: float):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


# palette
ANDESITE = (138, 141, 136)
WOOD = (166, 124, 74)
WOOD_DARK = (112, 80, 46)
IRON = (176, 180, 186)
IRON_DARK = (96, 100, 108)
BRASS = (214, 168, 74)
BRASS_DARK = (150, 106, 40)
COPPER = (196, 112, 76)
RED = (196, 52, 46)
RED_DIM = (96, 40, 38)
CREATIVE = (150, 92, 180)
CREATIVE_DARK = (90, 52, 112)
BLACK = (34, 32, 30)
GLASS = (206, 226, 222)


# ---------------------------------------------------------------- base surfaces
def stone(x, y, base, seed=1):
    """Stone grain: fine noise plus soft 4x4 blotches (GamaMake's G channel)."""
    v = 0.86 + noise(x, y, seed) * 0.12
    if noise(x // 4, y // 4, seed + 90) > 0.8:
        v *= 0.93
    return shade(base, v)


def planks(x, y, base, seed=2, vertical=False):
    """Plank rows: own tone per row, darker seams, staggered butt joints, grain."""
    a, b = (x, y) if not vertical else (y, x)
    row = b // 4
    tone = 0.88 + noise(row, 0, seed) * 0.16
    if b % 4 == 3:
        return shade(base, 0.62)
    joint = (row * 7 + 3) % 16
    if a == joint:
        return shade(base, 0.7)
    grain = 0.93 + 0.07 * noise(a // 3, b, seed + 5)
    if noise(a, b, seed + 9) > 0.9:
        grain *= 0.9
    return shade(base, tone * grain)


def metal(x, y, base, seed=3):
    v = 0.9 + noise(x, y, seed) * 0.07
    return shade(base, v)


# ---------------------------------------------------------------- building blocks
def frame(x, y, inner, frame_rgb=WOOD, width=2, nails=True):
    """Wooden frame with nails at the corners around an inner surface."""
    edge = x < width or y < width or x >= N - width or y >= N - width
    if edge:
        if x in (0, N - 1) or y in (0, N - 1):
            return shade(frame_rgb, 0.68)
        c = planks(x, y, frame_rgb, seed=11, vertical=(x < width or x >= N - width))
        if nails and (x, y) in ((1, 1), (N - 2, 1), (1, N - 2), (N - 2, N - 2)):
            return shade(IRON, 1.08)
        return c
    if x in (width, N - 1 - width) or y in (width, N - 1 - width):
        return shade(inner(x, y), 0.8)  # inner shadow
    return inner(x, y)


def rivets(x, y, c, positions, rgb=IRON):
    if (x, y) in positions:
        return shade(rgb, 1.15)
    if (x - 1, y - 1) in positions:
        return shade(c, 0.75)  # rivet shadow
    return c


def ring(x, y, r_out, r_in, cx=7.5, cy=7.5):
    d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
    return r_in <= d <= r_out


def andesite_casing(x, y):
    return frame(x, y, lambda a, b: stone(a, b, ANDESITE))


def brass_casing(x, y):
    return frame(x, y, lambda a, b: metal(a, b, BRASS_DARK, 4), frame_rgb=BRASS)


def gearbox(x, y):
    c = andesite_casing(x, y)
    if 4 <= x <= 11 and 4 <= y <= 11:
        c = metal(x, y, IRON_DARK)
        if x in (4, 11) or y in (4, 11):
            c = shade(IRON_DARK, 0.7)
        if 6 <= x <= 9 and 6 <= y <= 9:
            c = shade(BLACK, 1.0 + noise(x, y, 5) * 0.2)  # shaft hole
        c = rivets(x, y, c, [(5, 5), (10, 5), (5, 10), (10, 10)])
    return c


def indicator(base_fn, lamp):
    def f(x, y):
        c = base_fn(x, y)
        if 6 <= x <= 9 and 2 <= y <= 4:
            c = shade(lamp, 1.0 + 0.15 * (y == 2))
            if x in (6, 9) or y in (2, 4):
                c = shade(lamp, 0.7)
        return c
    return f


def clutch_side(powered):
    def inner(x, y):
        c = andesite_casing(x, y)
        if 3 <= x <= 12 and 7 <= y <= 8:  # split line through the middle
            c = shade(BLACK, 1.2)
        return c
    return indicator(inner, RED if powered else RED_DIM)


def gearshift_side(powered):
    def inner(x, y):
        c = andesite_casing(x, y)
        # arrow: right when off, left when on
        cx = 7
        dx = x - cx if not powered else cx + 1 - x
        if y in (9, 10, 11) and -3 <= dx <= 1:
            c = shade(IRON, 1.05)
        if (dx == 2 and 8 <= y <= 12) or (dx == 3 and 9 <= y <= 11) or (dx == 4 and y == 10):
            c = shade(IRON, 1.05)
        return c
    return indicator(inner, RED if powered else RED_DIM)


def chain_side(x, y):
    c = andesite_casing(x, y)
    if 3 <= x <= 12 and 3 <= y <= 12:
        c = metal(x, y, IRON_DARK, 8)
        # chain links running vertically, alternating orientation
        link = (y - 3) // 3
        ly = (y - 3) % 3
        if link % 2 == 0:
            if 6 <= x <= 9 and (ly != 1 or x in (6, 9)):
                c = shade(IRON, 1.0 + 0.1 * noise(x, y, 3))
        elif x in (7, 8):
            c = shade(IRON, 0.95)
    return c


def chain_gearshift_side(x, y):
    c = chain_side(x, y)
    if ring(x, y, 2.6, 1.2, 12, 12) and 10 <= x <= 14 and 10 <= y <= 14:
        c = RED
    return c


def speed_controller(x, y):
    c = brass_casing(x, y)
    if ring(x, y, 5.2, 0, 7.5, 7.5):
        c = shade(GLASS, 0.95 + 0.05 * noise(x, y, 6))
        if ring(x, y, 5.2, 4.3):
            c = shade(BRASS_DARK, 0.8)
        # needle from centre towards top-right
        for t in range(0, 5):
            if (x, y) == (7 + t // 1 if t < 4 else 11, 8 - t):
                c = RED
        if (x, y) in ((7, 8), (8, 8), (7, 7), (8, 7)):
            c = BLACK
    return c


def gauge_face(needle_to):
    def f(x, y):
        c = andesite_casing(x, y)
        if ring(x, y, 5.2, 0):
            c = shade(GLASS, 0.95 + 0.05 * noise(x, y, 7))
            if ring(x, y, 5.2, 4.3):
                c = shade(IRON_DARK, 0.9)
            # scale ticks
            if ring(x, y, 4.2, 3.4) and (x + y) % 3 == 0:
                c = shade(BLACK, 1.4)
            if needle_to == "right" and y in (7, 8) and 8 <= x <= 11:
                c = RED
            if needle_to == "up" and x in (7, 8) and 4 <= y <= 7:
                c = RED
            if (x, y) in ((7, 7), (8, 7), (7, 8), (8, 8)):
                c = BLACK
        return c
    return f


def creative_side(x, y):
    c = frame(x, y, lambda a, b: metal(a, b, CREATIVE_DARK, 12), frame_rgb=CREATIVE)
    if 3 <= x <= 12 and 3 <= y <= 12 and y % 3 == 0:
        c = shade(COPPER, 0.9 + 0.2 * noise(x, y, 4))  # coil windings
    return c


def creative_front(x, y):
    c = creative_side(x, y)
    if ring(x, y, 3.2, 0):
        c = metal(x, y, IRON_DARK)
        if ring(x, y, 1.5, 0):
            c = BLACK
    return c


def axis_side(x, y):
    """Shaft surface, used on 4px wide faces (u 6..10): bands along the length."""
    band = x % 4
    v = (0.78, 0.98, 1.08, 0.88)[band]
    if y % 8 == 0:
        v *= 0.85
    return shade(IRON, v * (0.95 + 0.05 * noise(x, y, 13)))


def axis_end(x, y):
    if ring(x, y, 2.2, 0):
        return shade(IRON, 0.85 + 0.1 * noise(x, y, 14))
    return shade(IRON_DARK, 0.9)


def cog_wood(x, y):
    return planks(x, y, WOOD, seed=21)


def cog_wood_end(x, y):
    d = ((x - 7.5) ** 2 + (y - 7.5) ** 2) ** 0.5
    v = 0.85 + 0.12 * ((int(d) % 2) == 0) + 0.05 * noise(x, y, 22)
    return shade(WOOD, v)


def cog_hub(x, y):
    c = metal(x, y, IRON_DARK, 23)
    if x in (0, 15) or y in (0, 15):
        c = shade(IRON_DARK, 0.75)
    return rivets(x, y, c, [(3, 3), (12, 3), (3, 12), (12, 12)])


def drill_head(x, y):
    """Iron bit with diagonal flutes (the spiral seen from the side)."""
    v = 0.92 + 0.08 * noise(x, y, 51)
    if (x + y) % 5 == 0:
        v *= 0.62  # flute
    elif (x + y) % 5 == 1:
        v *= 1.12  # edge catching the light
    return shade(IRON, v)


def drill_front(x, y):
    c = andesite_casing(x, y)
    if ring(x, y, 5.5, 0):
        c = shade(IRON_DARK, 0.85 + 0.1 * noise(x, y, 52))
        if ring(x, y, 5.5, 4.6):
            c = shade(IRON_DARK, 0.6)
    return c


def saw_blade(x, y):
    """Steel disc: brighter rim, darker hub ring, scratches running round it."""
    d = ((x - 7.5) ** 2 + (y - 7.5) ** 2) ** 0.5
    v = 0.95 + 0.06 * noise(x, y, 61)
    if d > 6.3:
        v *= 1.12  # sharpened rim
    elif 2.0 < d < 3.0:
        v *= 0.7  # hub ring
    elif int(d * 2) % 3 == 0:
        v *= 0.9  # circular scratches
    return shade(IRON, v)


def saw_front(x, y):
    c = andesite_casing(x, y)
    if 3 <= x <= 12 and 6 <= y <= 9:  # the slot the blade comes out of
        c = shade(BLACK, 1.0 + 0.2 * noise(x, y, 62))
        if y in (6, 9):
            c = shade(IRON_DARK, 0.8)
    return c


def crank_handle(x, y):
    return planks(x, y, WOOD_DARK, seed=31, vertical=True)


# ---------------------------------------------------------------- items
def andesite_alloy(x, y):
    # an ingot seen at an angle: parallelogram, light top face
    top = 5 <= y <= 7 and 4 + (7 - y) <= x <= 12 + (7 - y)
    front = 8 <= y <= 11 and 2 <= x <= 12
    side = 8 <= y <= 11 and 13 <= x <= 13 + (11 - y) // 2 and x <= 14
    if top:
        return stone(x, y, ANDESITE, 41) + (255,)
    if front:
        c = stone(x, y, shade(ANDESITE, 0.85), 42)
        if y == 11 or x == 2:
            c = shade(c, 0.75)
        if noise(x, y, 43) > 0.88:
            c = shade(IRON, 1.1)  # iron flecks
        return c + (255,)
    if side:
        return shade(ANDESITE, 0.6) + (255,)
    return (0, 0, 0, 0)


BLOCK = {
    "andesite_casing": andesite_casing,
    "brass_casing": brass_casing,
    "gearbox": gearbox,
    "clutch_off": clutch_side(False),
    "clutch_on": clutch_side(True),
    "gearshift_off": gearshift_side(False),
    "gearshift_on": gearshift_side(True),
    "chain_drive_side": chain_side,
    "chain_gearshift_side": chain_gearshift_side,
    "speed_controller": speed_controller,
    "speedometer_face": gauge_face("right"),
    "stressometer_face": gauge_face("up"),
    "creative_motor_side": creative_side,
    "creative_motor_front": creative_front,
    "axis": axis_side,
    "axis_end": axis_end,
    "cog_wood": cog_wood,
    "cog_wood_end": cog_wood_end,
    "cog_hub": cog_hub,
    "crank_handle": crank_handle,
    "drill_head": drill_head,
    "drill_front": drill_front,
    "saw_blade": saw_blade,
    "saw_front": saw_front,
}
ITEM = {"andesite_alloy": andesite_alloy}


def render(fn, rgba=False):
    img = Image.new("RGBA", (N, N))
    for y in range(N):
        for x in range(N):
            c = fn(x, y)
            img.putpixel((x, y), c if len(c) == 4 else c + (255,))
    return img


def main():
    for folder, table in (("block", BLOCK), ("item", ITEM)):
        out = OUT / folder
        out.mkdir(parents=True, exist_ok=True)
        for name, fn in table.items():
            render(fn).save(out / f"{name}.png")
    # contact sheet for reviewing (not shipped)
    names = list(BLOCK) + list(ITEM)
    sheet = Image.new("RGBA", (len(names) * 18, 18), (40, 40, 40, 255))
    for i, name in enumerate(names):
        fn = BLOCK.get(name) or ITEM[name]
        sheet.paste(render(fn), (i * 18 + 1, 1))
    preview = OUT.parent.parent.parent.parent / "docs"
    preview.mkdir(exist_ok=True)
    sheet.resize((sheet.width * 6, sheet.height * 6), Image.NEAREST).save(preview / "textures.png")
    print(f"{len(names)} textures -> {OUT}")


if __name__ == "__main__":
    main()
