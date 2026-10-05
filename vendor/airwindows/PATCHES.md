# Changes to the vendored Airwindows source

Airwindows plugins are used as published (MIT, (c) Chris Johnson), with the
fewest changes that make them safe to run at any setting. Each is listed here
so a re-sync from upstream can reapply or drop it.

- `src/PunchyGuitar/PunchyGuitar.h`: `bip[bip_total][12]` became `[17]`.
  `PunchyGuitarProc.cpp` indexes it with `x < poles`, and
  `poles = (int)((A + 0.618) * 10)` reaches 16, and 13 at the default A = 0.7,
  so the stock array is overrun at its own default setting (found with
  UBSan). The overrun reads and writes neighbouring members, which is what
  produced NaN output here.

Not a source change, but worth knowing: `shim/audioeffectx.h` makes every
effect's `operator new` return zeroed memory, because some plugins (also
PunchyGuitar's `gateL`/`gateR`) never initialise a member and rely on a host's
fresh heap.
