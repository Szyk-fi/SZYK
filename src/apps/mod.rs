//! The app modules. There is no list to maintain here: build.rs finds every
//! app file (or folder) in src/apps and generates the module declarations
//! and the factory table (`FACTORIES`) that registry.rs builds apps from.
//! See docs/ADDING_AN_APP.md.

include!(concat!(env!("OUT_DIR"), "/apps_gen.rs"));
