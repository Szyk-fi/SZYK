// ORCA's simulation (vendor/orca: sim.c, gbuffer.c, vmio.c from the C
// implementation by Hundredrabbits, MIT) is plain C and is called straight
// from src/apps/orca.rs through its own functions (orca_run, oevent_list_*,
// mbuffer_clear). Nothing to wrap, so this file only exists because the build
// pairs every sources list with a bridge file.
