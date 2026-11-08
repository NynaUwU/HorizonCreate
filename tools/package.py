#!/usr/bin/env python3
"""Builds the two release files into dist/ and checks them with the rules Horizon's
readers apply (docs/api/mod-files.md, horizon_mods::index::native::Bundle):

- dist/horizon_create-<version>.hmod   content mod (mod/)
- dist/horizon_create-<version>.hnmod  native mod (native/), source the launcher compiles

    python3 tools/package.py
"""
import json
import re
import sys
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
GAME = "26.3"
# horizon-mixins revision of the launcher at the tested Nightly (launcher/src/native_mods.rs MIXINS_REV)
MIXINS_REV = "5a64af938ec86b00d53d16a408aabc0038fb3e57"
# NATIVE_HOOKS_API of the tested Nightly: every hook point and helper the mod uses exists there
HOOKS_API = 411
REV = re.compile(r"^[0-9a-f]{40}$")
ID = re.compile(r"^[a-z0-9_]{1,64}$")


def fail(msg):
    sys.exit(f"package: {msg}")


def safe(path):
    parts = path.split("/")
    return not (path.startswith("/") or "\\" in path or ":" in path or any(p in ("", ".", "..") for p in parts))


def native_allowed(rest):
    name = rest.rsplit("/", 1)[-1]
    text = any(name.startswith(p) for p in ("README", "LICENSE", "LICENCE", "COPYING", "NOTICE")) or name == ".gitignore"
    data = rest.startswith("data/") or (rest.count("/") >= 2 and rest.split("/")[1] == "data")
    return rest.endswith(".rs") or rest.endswith(".toml") or data or (text and rest.count("/") <= 1)


def zip_files(out, files):
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for arc, src in files:
            if not safe(arc):
                fail(f"bad path {arc}")
            if isinstance(src, bytes):
                z.writestr(arc, src)
            else:
                z.write(src, arc)


def main():
    content = tomllib.loads((ROOT / "mod/horizon-mod.toml").read_text())
    mixin = tomllib.loads((ROOT / "native/horizon-mixin.toml").read_text())
    mod_id, version = content["id"], content["version"]
    if (mixin["mod"]["id"], mixin["mod"]["version"]) != (mod_id, version):
        fail("mod/ and native/ must have the same id and version")
    horizon = mixin["compatibility"]["horizon_revision"]
    if not (ID.match(mod_id) and REV.match(horizon) and REV.match(MIXINS_REV)):
        fail("invalid id or revisions")
    DIST.mkdir(exist_ok=True)

    hmod = DIST / f"{mod_id}-{version}.hmod"
    zip_files(hmod, [(p.relative_to(ROOT / "mod").as_posix(), p)
                     for p in sorted((ROOT / "mod").rglob("*")) if p.is_file()])

    manifest = {"schema": 1, "id": mod_id, "version": version, "game": GAME,
                "horizon": horizon, "mixins": MIXINS_REV, "hooks": HOOKS_API}
    files = [("horizon-native.json", json.dumps(manifest, indent=2).encode() + b"\n")]
    for p in sorted((ROOT / "native").rglob("*")):
        if p.is_file():
            rest = p.relative_to(ROOT / "native").as_posix()
            if not native_allowed(rest):
                fail(f"not allowed in a native bundle: native/{rest}")
            files.append((f"mixins/{mod_id}/{rest}", p))
    hnmod = DIST / f"{mod_id}-{version}.hnmod"
    zip_files(hnmod, files)

    # read back as Horizon would
    with zipfile.ZipFile(hnmod) as z:
        names = z.namelist()
        if len(names) != len(set(names)) or f"mixins/{mod_id}/horizon-mixin.toml" not in names:
            fail("bundle layout")
        m = json.loads(z.read("horizon-native.json"))
        t = tomllib.loads(z.read(f"mixins/{mod_id}/horizon-mixin.toml").decode())
        if (t["mod"]["id"], t["mod"]["version"], t["compatibility"]["horizon_revision"]) != (m["id"], m["version"], m["horizon"]):
            fail("native manifest and mixin identity/revision differ")
    for f in (hmod, hnmod):
        print(f"{f.relative_to(ROOT)}: {f.stat().st_size} bytes")


if __name__ == "__main__":
    main()
