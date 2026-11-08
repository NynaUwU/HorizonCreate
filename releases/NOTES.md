## Horizon Create

Create's rotational power (and Power Grid's physics, in the libraries) for Horizon 26.3.

**Install** (game folder = the one with `options.txt`):

1. `horizon_create-<version>.hmod` → `horizon-mods/`, then Options → Mods → turn on → restart.
2. `horizon_create-<version>.hnmod` → `horizon-native-mods/`. The launcher compiles it into the game
   (native mods are shipped as source; the first build takes a while). Needs a Horizon with native
   hook points API 411 or later (Nightly 2026-11-07 onwards).

Both files are needed: without the native mod the blocks exist but nothing turns.
New in 0.2.0: the **mechanical drill** (4 SU/RPM, breaks the block in front, faster with speed).

Single player / LAN host only for now. Full guide: `docs/TESTAR-NO-JOGO.md`.
