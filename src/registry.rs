//! Maps an app manifest's `id` to the Rust code that implements it.
//!
//! Manifests on disk (see manifest.rs) are real, loaded data — the app
//! list, names, and menu order all come from `apps/`, not a hardcoded Vec.
//! What's *not* dynamic is the code behind each `id`: that's still a
//! built-in Rust type. True load-from-disk code (native plugins via
//! `libloading`, or a WASM/bytecode runtime) is a much bigger, riskier
//! undertaking than this needs, and it wouldn't even carry over to the
//! real STM32N6 firmware, which will load apps some other way entirely.

use crate::app::{App, AppContext, AppFactory};
#[path = "app_runtime.rs"]
mod runtime;
use crate::audio_bus::AudioBus;
use crate::manifest::AppManifest;
use crate::modbus::ModBus;
use crate::note_bus::NoteBus;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::rc::Rc;

type Constructor = Rc<dyn Fn() -> Box<dyn App>>;

pub struct Registry {
    ctx: AppContext,
    factories: HashMap<String, AppFactory>,
    audio_bus: Arc<AudioBus>,
    modbus: Arc<ModBus>,
    notes: Arc<NoteBus>,
}

impl Registry {
    /// `ctx` carries every shared service apps are built from. It must
    /// provide the three buses (AudioBus, ModBus, NoteBus); anything else
    /// is between the host and the apps that ask for it.
    pub fn new(ctx: AppContext) -> Self {
        Self::with_factories(ctx, crate::apps::FACTORIES)
    }

    pub fn with_factories(ctx: AppContext, factories: &[(&str, AppFactory)]) -> Self {
        let audio_bus = ctx.get::<AudioBus>();
        let modbus = ctx.get::<ModBus>();
        let notes = ctx.try_get::<NoteBus>().unwrap_or_else(|| Arc::new(NoteBus::new()));
        Self { ctx, factories: factories.iter().map(|(k, f)| (k.to_string(), *f)).collect(), audio_bus, modbus, notes }
    }

    /// Whether this build has code for a manifest.
    pub fn implements(&self, m: &AppManifest) -> bool {
        self.factories.contains_key(m.module())
    }

    fn constructor(&self, m: &AppManifest) -> Option<Constructor> {
        let factory = *self.factories.get(m.module())?;
        let mut ctx = self.ctx.clone();
        ctx.provide_named("manifest", Arc::new(m.clone()));
        let id = m.id.clone();
        Some(Rc::new(move || factory(&ctx, &id)))
    }

    /// Builds the launcher's app list from discovered manifests, in the
    /// order given. A manifest with no matching implementation is skipped
    /// with a warning rather than panicking the whole OS, and nothing about
    /// one app (its outputs, inputs, notes) depends on any other being
    /// installed.
    pub fn build(&self, manifests: &[AppManifest]) -> Vec<(String, Box<dyn App>)> {
        let installed: Vec<&AppManifest> = manifests.iter().filter(|m| self.implements(m)).collect();
        // Everything an installed app offers other apps, declared before
        // any app exists, so every picker lists it and using it wakes the
        // app (see modbus.rs, audio_bus.rs, note_bus.rs).
        for m in &installed {
            for name in &m.audio_outputs {
                self.audio_bus.declare(&m.id, name);
            }
            for name in &m.mod_inputs {
                self.modbus.declare(&m.id, name);
            }
            if let Some(client) = &m.grid_client {
                crate::apps::grid_kit::grid().declare(&m.id, client);
            }
            if m.notes_in {
                self.notes.declare_instrument(&m.id, &m.name);
            }
            for name in &m.note_outputs {
                self.notes.declare_source(&m.id, name);
            }
        }
        let mut seen = HashSet::new();
        manifests
            .iter()
            .filter(|m| {
                if seen.insert(m.id.clone()) { true } else {
                    eprintln!("apps: duplicate id '{}' skipped", m.id); false
                }
            })
            .filter_map(|m| match self.constructor(m) {
                Some(make) => {
                    let inbox = if m.notes_in { self.notes.instrument_index(&m.name).map(|i| self.notes.inbox_ref(i)) } else { None };
                    let mut app = runtime::LazyApp::new(m.id.clone(), make, Arc::clone(&self.audio_bus)).with_modbus(Arc::clone(&self.modbus)).with_notes(Arc::clone(&self.notes), inbox);
                    if m.grid_client.is_some() {
                        app = app.with_grid();
                    }
                    Some((m.name.clone(), Box::new(app) as Box<dyn App>))
                }
                None => {
                    eprintln!("apps: no implementation for '{}' (module '{}')", m.id, m.module());
                    None
                }
            })
            .collect()
    }
}

/// The services the OS provides apps, in one place for the device binary,
/// the GUI and tests.
#[allow(clippy::too_many_arguments)]
pub fn standard_context(
    cutoff: Arc<crate::util::AtomicF32>,
    devices: Arc<crate::audio_devices::AudioDeviceState>,
    sensitivity: Arc<crate::util::AtomicF32>,
    nav_speed: Arc<crate::util::AtomicF32>,
    modbus: Arc<ModBus>,
    audio_bus: Arc<AudioBus>,
    master_volume: Arc<crate::util::AtomicF32>,
    mixer_bus: Arc<crate::mixer_bus::MixerBus>,
    prism_cc: Arc<crate::apps::prism::PrismCcTargets>,
    show_cpu: Arc<std::sync::atomic::AtomicBool>,
    midi_map: Arc<crate::midi_map::MidiMap>,
    accent: Arc<crate::theme::ThemeColor>,
    background: Arc<crate::theme::ThemeColor>,
    notes: Arc<NoteBus>,
) -> AppContext {
    let mut ctx = AppContext::new();
    ctx.provide(devices).provide(modbus).provide(audio_bus).provide(mixer_bus).provide(prism_cc).provide(midi_map).provide(notes);
    ctx.provide_named("cutoff", cutoff)
        .provide_named("sensitivity", sensitivity)
        .provide_named("nav_speed", nav_speed)
        .provide_named("master_volume", master_volume)
        .provide_named("show_cpu", show_cpu)
        .provide_named("accent", accent)
        .provide_named("background", background);
    ctx
}

#[cfg(test)]
mod installation_contract_tests {
    use super::*;
    use crate::app::{Input, SystemRole};
    use crate::display::FrameBuffer;
    struct OptionalApp;
    impl App for OptionalApp {
        fn tick(&mut self, _: &Input) {}
        fn draw(&mut self, _: &mut FrameBuffer) {}
        fn system_role(&self) -> Option<SystemRole> { Some(SystemRole::Mixer) }
    }
    #[test]
    fn missing_duplicate_and_renamed_apps_are_independent() {
        fn optional(_: &AppContext, _: &str) -> Box<dyn App> {
            Box::new(OptionalApp)
        }
        let mut ctx = AppContext::new();
        ctx.provide(Arc::new(AudioBus::new())).provide(Arc::new(ModBus::new()));
        let registry = Registry::with_factories(ctx, &[("mixer", optional)]);
        let m = |id: &str, name: &str| AppManifest { id: id.into(), name: name.into(), ..Default::default() };
        let manifests = vec![m("removed", "Unavailable"), m("mixer", "Renamed service"), m("mixer", "Duplicate")];
        let mut apps = registry.build(&manifests);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].0, "Renamed service");
        assert_eq!(apps[0].1.system_role(), Some(SystemRole::Mixer));
        apps[0].1.tick(&Input::default());
        assert!(registry.build(&[]).is_empty());
        assert!(registry.build(&manifests[..1]).is_empty());
    }
}

/// The manifest is the patching contract: each app's `mod_inputs` must
/// list exactly what the app registers on the ModBus when built, so its
/// inputs can be offered before it's ever opened. Run with
/// `PORTAMAX_WRITE_MANIFESTS=1` to rewrite the manifests from the code.
#[cfg(test)]
mod manifest_contract_tests {
    use super::*;
    use crate::apps::prism::PrismCcTargets;
    use crate::audio_devices::AudioDeviceState;
    use crate::midi_map::MidiMap;
    use crate::mixer_bus::MixerBus;
    use crate::theme;
    use crate::util::AtomicF32;

    pub(crate) fn test_context(modbus: Arc<ModBus>) -> AppContext {
        standard_context(
            Arc::new(AtomicF32::new(1000.0)),
            Arc::new(AudioDeviceState::new("test".into())),
            Arc::new(AtomicF32::new(0.1)),
            Arc::new(AtomicF32::new(3.0)),
            modbus,
            Arc::new(AudioBus::new()),
            Arc::new(AtomicF32::new(1.0)),
            Arc::new(MixerBus::new()),
            Arc::new(PrismCcTargets::new()),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(MidiMap::new()),
            Arc::new(theme::ThemeColor::new(theme::ACCENT_DEFAULT_HUE, theme::ACCENT_DEFAULT_SAT, theme::ACCENT_DEFAULT_VAL)),
            Arc::new(theme::ThemeColor::new(theme::BG_DEFAULT_HUE, theme::BG_DEFAULT_SAT, theme::BG_DEFAULT_VAL)),
            Arc::new(NoteBus::new()),
        )
    }

    fn fresh_registry(modbus: Arc<ModBus>) -> Registry {
        Registry::new(test_context(modbus))
    }

    /// The manifest's own text with its `mod_inputs` replaced (every other
    /// field, comment and line kept as written).
    fn manifest_text(old: &str, inputs: &[String]) -> String {
        let mut s = String::new();
        let mut skipping = false;
        for line in old.lines() {
            let t = line.trim_start();
            if skipping {
                if t.starts_with(']') {
                    skipping = false;
                }
                continue;
            }
            if t.starts_with("# Modulation inputs this app registers") {
                continue;
            }
            if t.starts_with("mod_inputs") {
                skipping = !t.contains(']');
                continue;
            }
            s.push_str(line);
            s.push('\n');
        }
        if !inputs.is_empty() {
            s.push_str("# Modulation inputs this app registers (checked by registry.rs's tests).\nmod_inputs = [\n");
            for n in inputs {
                s.push_str(&format!("    {n:?},\n"));
            }
            s.push_str("]\n");
        }
        s
    }

    #[test]
    fn every_manifest_declares_exactly_the_inputs_its_app_registers() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let write = std::env::var_os("PORTAMAX_WRITE_MANIFESTS").is_some();
        let mut wrong = Vec::new();
        for m in crate::manifest::discover(dir) {
            let modbus = Arc::new(ModBus::new());
            let registry = fresh_registry(Arc::clone(&modbus));
            let Some(make) = registry.constructor(&m) else { continue };
            let _app = make();
            let registered = modbus.names();
            if registered != m.mod_inputs {
                if write {
                    let path = if dir.join(&m.id).is_dir() { dir.join(&m.id).join("manifest.toml") } else { dir.join(format!("{}.toml", m.id)) };
                    let old = std::fs::read_to_string(&path).unwrap_or_default();
                    std::fs::write(path, manifest_text(&old, &registered)).unwrap();
                }
                wrong.push(format!("{}: manifest {:?} vs code {:?}", m.id, m.mod_inputs, registered));
            }
        }
        assert!(write || wrong.is_empty(), "manifests out of date: run tools/sync_manifests.sh (it rewrites mod_inputs from the code) and commit:\n{}", wrong.join("\n"));
    }

    /// The drop-in promise, checked for the whole `apps/` folder: every
    /// manifest has code behind it, ids and names are unique (the buses key
    /// on names, so two apps sharing one would share a slot), everything a
    /// manifest offers other apps is really declared, and every app can be
    /// entered, ticked, drawn and run without panicking or emitting NaN.
    #[test]
    fn every_installed_app_is_wired_up_and_runs() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let manifests = crate::manifest::discover(dir);
        assert!(!manifests.is_empty());
        let ctx = test_context(Arc::new(ModBus::new()));
        let notes = ctx.get::<NoteBus>();
        let registry = Registry::new(ctx);

        let missing: Vec<_> = manifests.iter().filter(|m| !registry.implements(m)).map(|m| format!("{} (module {:?})", m.id, m.module())).collect();
        assert!(missing.is_empty(), "manifests with no `pub fn create` in src/apps: {missing:?}");
        let mut seen = HashSet::new();
        for m in &manifests {
            assert!(seen.insert(("id", m.id.clone())), "duplicate app id {}", m.id);
            assert!(seen.insert(("name", m.name.clone())), "duplicate app name {:?} (the buses key on names)", m.name);
            assert!(!m.category.is_empty() && !m.description.is_empty(), "{}: the launcher needs a category and a description", m.id);
        }

        let mut apps = registry.build(&manifests);
        assert_eq!(apps.len(), manifests.len(), "every manifest becomes an app");
        let instruments = notes.instruments();
        let sources = notes.sources();
        for m in &manifests {
            if m.notes_in {
                assert!(instruments.iter().any(|(_, n, owner)| n == &m.name && owner == &m.id), "{}: notes_in but not an instrument on the note bus", m.id);
            }
            for out in &m.note_outputs {
                assert!(sources.iter().any(|(n, owner, _)| n == out && owner == &m.id), "{}: note output {out:?} is not declared", m.id);
            }
        }

        let engine = crate::audio::new_engine(Arc::new(AtomicF32::new(1.0)));
        // The O&C apps each claim a process-wide firmware slot while they are
        // running, which their own tests (running in parallel) also need, so
        // they are only checked as far as building and declaring above.
        let exclusive = |i: usize| manifests[i].module() == "oc";
        for (i, (_, app)) in apps.iter_mut().enumerate() {
            if exclusive(i) {
                continue;
            }
            if let Some(p) = app.audio_processor() {
                engine.add(p);
            }
        }
        let mut fb = crate::display::FrameBuffer::new();
        for (i, (name, app)) in apps.iter_mut().enumerate() {
            if exclusive(i) {
                continue;
            }
            app.on_enter();
            app.tick(&crate::app::Input::default());
            app.background_tick();
            app.draw(&mut fb);
            let _ = app.slint_rows();
            let _ = name;
        }
        let mut out = [0.0f32; 1024];
        for _ in 0..4 {
            engine.process(&mut out, 2, 48_000.0);
            assert!(out.iter().all(|x| x.is_finite()), "an app produced NaN or infinity");
        }
    }

    /// The whole point: an app nobody has opened still shows its inputs,
    /// a source can write into them, and that write wakes the app, which
    /// picks up the very handle the source was writing.
    #[test]
    fn an_unopened_apps_inputs_are_patchable_and_wake_it() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let manifests: Vec<_> = crate::manifest::discover(dir).into_iter().filter(|m| m.id == "plaits" || m.id == "turing_machine").collect();
        let modbus = Arc::new(ModBus::new());
        let registry = fresh_registry(Arc::clone(&modbus));
        let mut apps = registry.build(&manifests);
        let idx = modbus.index_of("Plaits: Harmonics").expect("declared before Plaits exists");
        let groups = modbus.apps();
        assert!(groups.iter().any(|(a, v)| a == "Plaits" && v.contains(&idx)), "{groups:?}");
        assert!(!modbus.is_claimed(idx), "Plaits isn't built yet");
        // a source writes into it...
        modbus.get(idx).unwrap().set(0.25);
        // ...and the next background tick builds Plaits, which registers
        // its Harmonics input and gets the patched handle back.
        let before = modbus.len();
        for (_, app) in apps.iter_mut() {
            app.background_tick();
        }
        assert!(modbus.is_claimed(idx), "the write woke Plaits");
        assert_eq!(modbus.len(), before, "registering reused the declared inputs");
        assert_eq!(modbus.get(idx).unwrap().get(), 0.25);
    }

    /// The note bus end to end: Bloom, routed to Voltage, plays Voltage
    /// -- which is never opened -- and Bloom's own voices stay silent.
    #[test]
    fn bloom_plays_an_unopened_instrument_through_the_note_bus() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let manifests: Vec<_> = crate::manifest::discover(dir).into_iter().filter(|m| m.id == "bloom" || m.id == "voltage").collect();
        let ctx = test_context(Arc::new(ModBus::new()));
        let notes = ctx.get::<NoteBus>();
        let registry = Registry::new(ctx);
        let mut apps = registry.build(&manifests);
        let engine = crate::audio::new_engine(Arc::new(AtomicF32::new(1.0)));
        for (_, app) in apps.iter_mut() {
            engine.add(app.audio_processor().unwrap());
        }
        let voltage = notes.instrument_index("Voltage").expect("Voltage is declared as an instrument");
        let bloom_routes: Vec<_> = notes.sources().into_iter().filter(|(n, _, _)| n.starts_with("Bloom Shape ")).map(|(_, _, r)| r).collect();
        assert_eq!(bloom_routes.len(), 8, "each of Bloom's eight shapes has its own output");
        let bloom = apps.iter().position(|(n, _)| n == "Bloom").unwrap();
        apps[bloom].1.on_enter();
        for r in &bloom_routes {
            r.store(voltage, std::sync::atomic::Ordering::Relaxed);
        }
        apps[bloom].1.toggle_running();
        let mut heard = 0.0f32;
        let mut voltage_woke = false;
        for _ in 0..600 {
            apps[bloom].1.tick(&crate::app::Input::default());
            for (_, app) in apps.iter_mut() {
                app.background_tick();
            }
            voltage_woke |= notes.requested("voltage");
            let mut out = [0.0f32; 1024];
            engine.process(&mut out, 2, 48_000.0);
            heard = heard.max(out.iter().fold(0.0, |m, x| m.max(x.abs())));
        }
        assert!(voltage_woke, "Bloom's notes reached Voltage");
        assert!(heard > 0.01, "and Voltage sounded them ({heard})");
    }

    /// A source lists and edits the settings of the instrument it plays
    /// without touching the other app: asking builds and wakes the
    /// instrument, which publishes its settings; an edit queued on the bus is
    /// applied by the instrument's own wrapper on its next tick.
    #[test]
    fn a_source_lists_and_edits_the_settings_of_the_instrument_it_plays() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let manifests: Vec<_> = crate::manifest::discover(dir).into_iter().filter(|m| m.id == "synth").collect();
        let ctx = test_context(Arc::new(ModBus::new()));
        let notes = ctx.get::<NoteBus>();
        let registry = Registry::new(ctx);
        let mut apps = registry.build(&manifests);
        let slot = notes.instrument_index("Synth").expect("Synth is an instrument");
        assert!(notes.instrument_settings(slot).is_empty(), "nothing is published before the first ask");
        for (_, app) in apps.iter_mut() {
            app.background_tick();
        }
        let rows = notes.instrument_settings(slot);
        assert!(rows.len() >= 2, "the instrument woke and published its settings: {rows:?}");
        let (i, before) = rows.iter().enumerate().find(|(_, r)| r.label.to_lowercase().contains("cutoff")).map(|(i, r)| (i, r.value.clone())).expect("a cutoff setting");
        notes.adjust_instrument_setting(slot, i, -5);
        for (_, app) in apps.iter_mut() {
            app.background_tick();
        }
        let after = notes.instrument_settings(slot)[i].value.clone();
        assert_ne!(before, after, "the queued edit reached the instrument");
    }

    /// The user-facing result: a source's own menu shows the settings of the
    /// instrument it plays right under its Plays row, and editing one of
    /// those rows changes the instrument.
    #[test]
    fn a_sources_menu_lists_the_instruments_settings_under_plays() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let manifests: Vec<_> = crate::manifest::discover(dir).into_iter().filter(|m| m.id == "hum" || m.id == "synth").collect();
        let ctx = test_context(Arc::new(ModBus::new()));
        let notes = ctx.get::<NoteBus>();
        let registry = Registry::new(ctx);
        let mut apps = registry.build(&manifests);
        let synth = notes.instrument_index("Synth").expect("Synth is an instrument");
        let route = notes.sources().into_iter().find(|(n, _, _)| n == "Hum").expect("Hum's output is declared").2;
        route.store(synth, std::sync::atomic::Ordering::Relaxed);
        let hum = apps.iter().position(|(n, _)| n == "Hum").unwrap();
        apps[hum].1.on_enter();
        let rows_of = |apps: &mut Vec<(String, Box<dyn App>)>| {
            // The menu asking (every frame it is drawn) is what starts the
            // instrument publishing, so ask first, then let the frames run.
            let _ = apps[hum].1.slint_rows();
            for _ in 0..3 {
                for (_, app) in apps.iter_mut() {
                    app.background_tick();
                }
            }
            apps[hum].1.slint_rows()
        };
        let rows = rows_of(&mut apps);
        let plays = rows.iter().position(|r| r.0 == "Plays").expect("a Plays row");
        assert_eq!(plays, 0, "Plays leads the menu");
        assert!(rows[plays + 1].0.starts_with("  "), "the instrument's settings come right under it: {rows:?}");
        let cutoff = rows.iter().position(|r| r.0.trim().to_lowercase().contains("cutoff")).expect("Synth's cutoff is listed");
        let before = rows[cutoff].1.clone();
        // Select that row and turn knob 2 down.
        for _ in 0..cutoff {
            apps[hum].1.tick(&crate::app::Input { navigation_steps: 1, ..Default::default() });
        }
        for _ in 0..4 {
            apps[hum].1.tick(&crate::app::Input { knob2: -1, ..Default::default() });
            for (_, app) in apps.iter_mut() {
                app.background_tick();
            }
        }
        let after = rows_of(&mut apps)[cutoff].1.clone();
        assert_ne!(before, after, "editing the listed row changed the instrument");
    }

    /// The grid is playable from the first frame: Teletype is chosen in the
    /// Grid app without ever having been opened, handing it the grid builds
    /// it, and with a scene that has no G ops it still shows (and runs)
    /// something instead of staying dark.
    #[test]
    fn handing_the_grid_to_an_unopened_teletype_builds_it_and_lights_the_grid() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let manifests: Vec<_> = crate::manifest::discover(dir).into_iter().filter(|m| m.id == "teletype" || m.id == "kria").collect();
        let ctx = test_context(Arc::new(ModBus::new()));
        let registry = Registry::new(ctx);
        let mut apps = registry.build(&manifests);
        let grid = crate::apps::grid_kit::grid();
        assert!(grid.clients().contains(&"Teletype".to_string()), "declared from its manifest, before it is built");
        assert_eq!(grid.focus(), None, "declaring takes nothing");
        grid.set_focus("Teletype");
        for _ in 0..3 {
            for (_, app) in apps.iter_mut() {
                app.background_tick();
            }
        }
        let s = grid.snapshot();
        assert!(s.hint.is_some(), "Teletype was built and says how it uses the grid");
        assert_eq!(s.leds[0], 5, "the first script key is lit dimly");
        grid.press(0, 0, true);
        for (_, app) in apps.iter_mut() {
            app.background_tick();
        }
        assert_eq!(grid.snapshot().leds[0], 15, "pressing it lights it as the script runs");
    }

    /// Drag and drop: a folder holding a manifest and a patch is a new
    /// instrument -- no code, no rebuild -- with its own name and mixer
    /// channel, and other apps can play it at once.
    #[test]
    fn a_cartridge_folder_is_a_new_playable_instrument() {
        let dir = std::env::temp_dir().join(format!("portamax-cartridges-{}", std::process::id()));
        let cart = dir.join("glass_keys");
        std::fs::create_dir_all(&cart).unwrap();
        std::fs::write(cart.join("patch.json"), include_str!("../assets/atlas/presets/evolving_glass.json")).unwrap();
        std::fs::write(
            cart.join("manifest.toml"),
            "id = \"glass_keys\"\nname = \"Glass Keys\"\nmodule = \"atlas\"\ncategory = \"instrument\"\naudio_outputs = [\"Glass Keys\"]\nnotes_in = true\ndata = \"patch.json\"\n",
        )
        .unwrap();
        let manifests = crate::manifest::discover(&dir);
        let ctx = test_context(Arc::new(ModBus::new()));
        let notes = ctx.get::<NoteBus>();
        let mixer = ctx.get::<MixerBus>();
        let registry = Registry::new(ctx);
        let mut apps = registry.build(&manifests);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].0, "Glass Keys");
        let engine = crate::audio::new_engine(Arc::new(AtomicF32::new(1.0)));
        engine.add(apps[0].1.audio_processor().unwrap());
        let slot = notes.instrument_index("Glass Keys").expect("declared from the manifest");
        let mut keys = notes.register_source("test keys");
        keys.set_route(slot);
        keys.note_on(60, 100);
        let mut heard = 0.0f32;
        for _ in 0..200 {
            apps[0].1.background_tick();
            let mut out = [0.0f32; 1024];
            engine.process(&mut out, 2, 48_000.0);
            heard = heard.max(out.iter().fold(0.0, |m, x| m.max(x.abs())));
        }
        assert!(heard > 0.01, "the cartridge played the note ({heard})");
        assert!(mixer.names().iter().any(|n| n == "Glass Keys"), "with its own mixer channel");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Opening an app never starts sound by itself: until you play it or
    /// press F3, it's silent.
    #[test]
    fn no_app_plays_by_itself_when_opened() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps"));
        let mut loud = Vec::new();
        for m in crate::manifest::discover(dir) {
            let ctx = test_context(Arc::new(ModBus::new()));
            let registry = Registry::new(ctx);
            let mut apps = registry.build(std::slice::from_ref(&m));
            let Some((name, app)) = apps.first_mut() else { continue };
            let engine = crate::audio::new_engine(Arc::new(AtomicF32::new(1.0)));
            engine.add(app.audio_processor().unwrap());
            app.on_enter();
            let mut peak = 0.0f32;
            for _ in 0..120 {
                app.tick(&crate::app::Input::default());
                app.background_tick();
                let mut out = [0.0f32; 1024];
                engine.process(&mut out, 2, 48_000.0);
                peak = peak.max(out.iter().fold(0.0, |a, x| a.max(x.abs())));
            }
            // (Plaits' idle engines leave a few thousandths of residue)
            if peak > 0.01 {
                loud.push(format!("{name} ({peak:.3})"));
            }
        }
        assert!(loud.is_empty(), "these play on open: {}", loud.join(", "));
    }
}
