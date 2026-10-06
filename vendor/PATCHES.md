# Changes to vendored third-party source

Everything under `vendor/` is used as published, with the fewest changes that
make it safe to run at any setting. Each change is listed so a re-sync can
reapply or drop it. (Airwindows has its own list in `airwindows/PATCHES.md`.)

- `eurorack/tides2/ramp/ramp_extractor.cc`: the loop that wraps
  `expected_phase` below 1 never terminates when `period` is 0, which makes the
  value infinite. That happens when a gate rises on the first sample after a
  reset, and it hung Stages' PLL oscillator on the audio thread. It is now
  `fmodf` with a finite check, identical for every finite value. Shared by
  Tides, Stages and anything else that uses the ramp extractor.
