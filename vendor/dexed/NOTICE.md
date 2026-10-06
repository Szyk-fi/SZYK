# Dexed's synth engine (msfa)

`msfa/` is the FM synthesis engine from **Dexed** by Pascal Gauthier
(https://github.com/asb2m10/dexed, `Source/msfa`), which is itself Google's
"music-synthesizer-for-android" engine (Raph Levien, Google Inc.) with Dexed's
accuracy fixes to the envelopes, LFO, pitch envelope and operator math. Those
files carry an **Apache License 2.0** header (see `LICENSE-APACHE-2.0`) and are
used unmodified, except that `tuning.cc` (which needs JUCE) is left out and its
one function, the standard tuning table, is rebuilt in `shim/`.

Not taken: the rest of Dexed (plugin shell, UI, the Mark I and OPL engines,
cartridge handling), which is GPL-3.0. The engine used is Dexed's "Modern"
one, msfa's `FmCore`. Not bundled: any DX7 voice banks (Yamaha's factory ROM
sounds are Yamaha's); put `.syx` banks in `dx7_presets/`.
