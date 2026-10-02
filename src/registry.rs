//! Maps an app manifest's `id` to the Rust code that implements it.
//!
//! Manifests on disk (see manifest.rs) are real, loaded data — the app
//! list, names, and menu order all come from `apps/`, not a hardcoded Vec.
//! What's *not* dynamic is the code behind each `id`: that's still a
//! built-in Rust type. True load-from-disk code (native plugins via
//! `libloading`, or a WASM/bytecode runtime) is a much bigger, riskier
//! undertaking than this needs, and it wouldn't even carry over to the
//! real STM32N6 firmware, which will load apps some other way entirely.

use crate::app::App;
#[path = "app_runtime.rs"]
mod runtime;
use crate::apps::{
    analyzer::AnalyzerApp, beads::BeadsApp, black_hole::BlackHoleApp, bloom::BloomApp, cascade::CascadeApp, clouds::CloudsApp, cv_out::CvOutApp, madness::MadnessApp,
    magnito::MagnitoApp, midi_learn::MidiLearnApp, mixer::MixerApp, natural_gate::NaturalGateApp, nautilus::NautilusApp, nebula::NebulaApp, pams::PamsApp, plaits::PlaitsApp, prism::PrismApp,
    queen_of_pentacles::QueenOfPentaclesApp, rainmaker::RainmakerApp, sample_drum::SampleDrumApp, sequencer::SequencerApp, settings::SettingsApp, singularity::SingularityApp,
    starlab::StarlabApp, synth::SynthApp, tape::TapeApp, tonestack::TonestackApp, turing_machine::TuringMachineApp, visualizer::VisualizerApp, voltage::VoltageApp,
    warps::WarpsApp,
    oracle::OracleApp, pulsar::PulsarApp, tinkertone::TinkertoneApp, norns::NornsApp, controller_setup::ControllerSetupApp,
};
use crate::audio_bus::AudioBus;
use crate::audio_devices::AudioDeviceState;
use crate::manifest::AppManifest;
use crate::apps::prism::PrismCcTargets;
use crate::midi_map::MidiMap;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::theme::ThemeColor;
use crate::util::AtomicF32;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::rc::Rc;

type Constructor = Rc<dyn Fn() -> Box<dyn App>>;

pub struct Registry {
    constructors: HashMap<String, Constructor>,
    audio_bus: Arc<AudioBus>,
    modbus: Arc<ModBus>,
}

impl Registry {
    pub fn new(
        cutoff: Arc<AtomicF32>,
        devices: Arc<AudioDeviceState>,
        sensitivity: Arc<AtomicF32>,
        nav_speed: Arc<AtomicF32>,
        modbus: Arc<ModBus>,
        audio_bus: Arc<AudioBus>,
        master_volume: Arc<AtomicF32>,
        mixer_bus: Arc<MixerBus>,
        prism_cc: Arc<PrismCcTargets>,
        show_cpu: Arc<std::sync::atomic::AtomicBool>,
        midi_map: Arc<MidiMap>,
        accent: Arc<ThemeColor>,
        background: Arc<ThemeColor>,
    ) -> Self {
        let mut constructors: HashMap<String, Constructor> = HashMap::new();

        constructors.insert("mixer".into(), {
            let master_volume = Arc::clone(&master_volume);
            let mixer_bus = Arc::clone(&mixer_bus);
            let nav_speed = Arc::clone(&nav_speed);
            let audio_bus = Arc::clone(&audio_bus);
            Rc::new(move || {
                Box::new(MixerApp::new(Arc::clone(&master_volume), Arc::clone(&mixer_bus), Arc::clone(&nav_speed), Arc::clone(&audio_bus))) as Box<dyn App>
            })
        });

        constructors.insert("synth".into(), {
            let cutoff = Arc::clone(&cutoff);
            let sensitivity = Arc::clone(&sensitivity);
            Rc::new(move || Box::new(SynthApp::new(Arc::clone(&cutoff), Arc::clone(&sensitivity))) as Box<dyn App>)
        });
        constructors.insert("midi_learn".into(), {
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let midi_map = Arc::clone(&midi_map);
            Rc::new(move || Box::new(MidiLearnApp::new(Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&midi_map))) as Box<dyn App>)
        });
        constructors.insert("settings".into(), {
            let devices = Arc::clone(&devices);
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let show_cpu = Arc::clone(&show_cpu);
            let accent = Arc::clone(&accent);
            let background = Arc::clone(&background);
            Rc::new(move || {
                Box::new(SettingsApp::new(
                    Arc::clone(&devices),
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&show_cpu),
                    Arc::clone(&accent),
                    Arc::clone(&background),
                )) as Box<dyn App>
            })
        });
        constructors.insert("controller".into(), {
            let nav_speed = Arc::clone(&nav_speed);
            Rc::new(move || Box::new(ControllerSetupApp::new(Arc::clone(&nav_speed))) as Box<dyn App>)
        });
        constructors.insert("plaits".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(PlaitsApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert(
            "analyzer".into(),
            {let bus=audio_bus.clone();Rc::new(move || Box::new(AnalyzerApp::with_audio_bus(bus.clone())) as Box<dyn App>)},
        );
        constructors.insert(
            "visualizer".into(),
            {let bus=audio_bus.clone();Rc::new(move || Box::new(VisualizerApp::with_audio_bus(bus.clone())) as Box<dyn App>)},
        );
        constructors.insert("sequencer".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(SequencerApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("pams".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            Rc::new(move || {
                Box::new(PamsApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus))) as Box<dyn App>
            })
        });
        constructors.insert("madness".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(MadnessApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("bloom".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(BloomApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("clouds".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(CloudsApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("nebula".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(NebulaApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("cascade".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(CascadeApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("voltage".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(VoltageApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("singularity".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(SingularityApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("tape".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(TapeApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("natural_gate".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(NaturalGateApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus)))
                    as Box<dyn App>
            })
        });
        constructors.insert("turing_machine".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            Rc::new(move || {
                Box::new(TuringMachineApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus))) as Box<dyn App>
            })
        });
        constructors.insert("warps".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(WarpsApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("beads".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(BeadsApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("starlab".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(StarlabApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("rainmaker".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(RainmakerApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("nautilus".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(NautilusApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("black_hole".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(BlackHoleApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("queen_of_pentacles".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            Rc::new(move || {
                Box::new(QueenOfPentaclesApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus))) as Box<dyn App>
            })
        });
        constructors.insert("magnito".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(MagnitoApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("retro".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(crate::apps::retro::RetroApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("sample_drum".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(SampleDrumApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("tonestack".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(TonestackApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("cv_out".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            Rc::new(move || {
                Box::new(CvOutApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus))) as Box<dyn App>
            })
        });
        constructors.insert("prism".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            let prism_cc = Arc::clone(&prism_cc);
            Rc::new(move || {
                Box::new(PrismApp::new_with_cc(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                    &prism_cc,
                )) as Box<dyn App>
            })
        });

        constructors.insert("morph".into(), {
            let bus=Arc::clone(&audio_bus);let mods=Arc::clone(&modbus);let mixer=Arc::clone(&mixer_bus);let nav=Arc::clone(&nav_speed);
            Rc::new(move || Box::new(crate::apps::morph::MorphApp::new(bus.clone(),mods.clone(),mixer.clone(),nav.clone())) as Box<dyn App>)
        });
        constructors.insert("forge".into(), {
            let bus=audio_bus.clone();let mods=modbus.clone();let mixer=mixer_bus.clone();let nav=nav_speed.clone();
            Rc::new(move || Box::new(crate::apps::forge::ForgeApp::new(bus.clone(),mods.clone(),mixer.clone(),nav.clone())) as Box<dyn App>)
        });
        constructors.insert("vector_filter".into(), {
            let bus=audio_bus.clone();let mods=modbus.clone();let mixer=mixer_bus.clone();let nav=nav_speed.clone();
            Rc::new(move || Box::new(crate::apps::vector_filter::VectorFilterApp::new(bus.clone(),mods.clone(),mixer.clone(),nav.clone())) as Box<dyn App>)
        });
        for &(kind,id,_) in crate::apps::collection::APPS {
            let bus=audio_bus.clone(); let mods=modbus.clone(); let mixer=mixer_bus.clone(); let nav=nav_speed.clone();
            constructors.insert(id.into(), Rc::new(move || if kind==crate::apps::collection::Kind::Portal {Box::new(crate::apps::collection::portal::PortalApp::new(bus.clone(),mods.clone(),mixer.clone(),nav.clone())) as Box<dyn App>} else {Box::new(crate::apps::collection::CollectionApp::new(kind,bus.clone(),mods.clone(),mixer.clone(),nav.clone())) as Box<dyn App>}));
        }
        constructors.insert("oracle".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(OracleApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("pulsar".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(PulsarApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("norns".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(NornsApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        constructors.insert("tinkertone".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Rc::new(move || {
                Box::new(TinkertoneApp::new(
                    Arc::clone(&sensitivity),
                    Arc::clone(&nav_speed),
                    Arc::clone(&modbus),
                    Arc::clone(&audio_bus),
                    Arc::clone(&mixer_bus),
                )) as Box<dyn App>
            })
        });
        Self { constructors, audio_bus, modbus }
    }

    /// Builds the launcher's app list from discovered manifests, in the
    /// order given. A manifest with no matching implementation is skipped
    /// with a warning rather than panicking the whole OS.
    pub fn build(&self, manifests: &[AppManifest]) -> Vec<(String, Box<dyn App>)> {
        for m in manifests {if self.constructors.contains_key(&m.id) {for name in audio_outputs(&m.id) {self.audio_bus.declare(&m.id,&name);}}}
        // Every installed app's modulation inputs, before any app exists,
        // so sources can list and patch them (see modbus.rs).
        for m in manifests {if self.constructors.contains_key(&m.id) {for name in &m.mod_inputs {self.modbus.declare(&m.id, name);}}}
        let mut seen = HashSet::new();
        manifests
            .iter()
            .filter(|m| {
                if seen.insert(m.id.clone()) { true } else {
                    eprintln!("apps: duplicate id '{}' skipped", m.id); false
                }
            })
            .filter_map(|m| match self.constructors.get(&m.id) {
                Some(make) => Some((m.name.clone(), Box::new(runtime::LazyApp::new(m.id.clone(), Rc::clone(make), Arc::clone(&self.audio_bus)).with_modbus(Arc::clone(&self.modbus))) as Box<dyn App>)),
                None => {
                    eprintln!("apps: no implementation registered for id '{}'", m.id);
                    None
                }
            })
            .collect()
    }
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
        let mut constructors: HashMap<String, Constructor> = HashMap::new();
        constructors.insert("mixer".into(), Rc::new(|| Box::new(OptionalApp)));
        let registry = Registry { constructors, audio_bus: Arc::new(AudioBus::new()), modbus: Arc::new(ModBus::new()) };
        let manifests = vec![
            AppManifest { id: "removed".into(), name: "Unavailable".into(), mod_inputs: Vec::new() },
            AppManifest { id: "mixer".into(), name: "Renamed service".into(), mod_inputs: Vec::new() },
            AppManifest { id: "mixer".into(), name: "Duplicate".into(), mod_inputs: Vec::new() },
        ];
        let mut apps = registry.build(&manifests);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].0, "Renamed service");
        assert_eq!(apps[0].1.system_role(), Some(SystemRole::Mixer));
        apps[0].1.tick(&Input::default());
        assert!(registry.build(&[]).is_empty());
        assert!(registry.build(&manifests[..1]).is_empty());
    }
}

/// Output metadata belongs to the installation contract, not running instances.
fn audio_outputs(id:&str)->Vec<String>{
    if id=="natural_gate" {return (1..=2).map(|i|format!("Natural Gate: Ch{i}")).collect();}
    if id=="warps" {return vec!["Warps".into(),"Warps (Aux)".into()];}
    // Pulsar also publishes its kick lane alone, for sidechain ducking.
    if id=="pulsar" {return vec!["Pulsar".into(),"Pulsar Kick".into()];}
    if id=="portal" {return std::iter::once("Portal".into()).chain((1..=4).map(|i|format!("Portal Aux {i}"))).collect();}
    if let Some((_,_,name))=crate::apps::collection::APPS.iter().find(|(_,app,_)|*app==id){return vec![(*name).into()];}
    let name=match id {
        "forge"=>"Forge","synth"=>"Synth","analyzer"=>"Analyzer","visualizer"=>"Visualizer","plaits"=>"Plaits","beads"=>"Beads","black_hole"=>"Black Hole","bloom"=>"Bloom","cascade"=>"Cascade","clouds"=>"Clouds","madness"=>"Madness","magnito"=>"Magnito","morph"=>"Morph","nautilus"=>"Nautilus","nebula"=>"Nebula","prism"=>"Prism","rainmaker"=>"Rainmaker","sample_drum"=>"Sample Drum","sequencer"=>"Sequencer","singularity"=>"Singularity","starlab"=>"Starlab","tape"=>"Tape","tonestack"=>"Tonestack","voltage"=>"Voltage","retro"=>"Retro","vector_filter"=>"Vector Filter","oracle"=>"Oracle","pulsar"=>"Pulsar","tinkertone"=>"Tinkertone","norns"=>"Norns",_=>return Vec::new(),
    };vec![name.into()]
}

/// The manifest is the patching contract: each app's `mod_inputs` must
/// list exactly what the app registers on the ModBus when built, so its
/// inputs can be offered before it's ever opened. Run with
/// `PORTAMAX_WRITE_MANIFESTS=1` to rewrite the manifests from the code.
#[cfg(test)]
mod manifest_contract_tests {
    use super::*;
    use crate::theme;

    fn fresh_registry(modbus: Arc<ModBus>) -> Registry {
        Registry::new(
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
        )
    }

    fn manifest_text(m: &AppManifest, inputs: &[String]) -> String {
        let mut s = format!("id = {:?}\nname = {:?}\n", m.id, m.name);
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
            let Some(make) = registry.constructors.get(&m.id) else { continue };
            let _app = make();
            let registered = modbus.names();
            if registered != m.mod_inputs {
                if write {
                    let path = if dir.join(&m.id).is_dir() { dir.join(&m.id).join("manifest.toml") } else { dir.join(format!("{}.toml", m.id)) };
                    std::fs::write(path, manifest_text(&m, &registered)).unwrap();
                }
                wrong.push(format!("{}: manifest {:?} vs code {:?}", m.id, m.mod_inputs, registered));
            }
        }
        assert!(write || wrong.is_empty(), "manifests out of date (rerun with PORTAMAX_WRITE_MANIFESTS=1):\n{}", wrong.join("\n"));
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
}
