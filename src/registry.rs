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
            Rc::new(|| Box::new(AnalyzerApp::new()) as Box<dyn App>),
        );
        constructors.insert(
            "visualizer".into(),
            Rc::new(|| Box::new(VisualizerApp::new()) as Box<dyn App>),
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
        Self { constructors, audio_bus }
    }

    /// Builds the launcher's app list from discovered manifests, in the
    /// order given. A manifest with no matching implementation is skipped
    /// with a warning rather than panicking the whole OS.
    pub fn build(&self, manifests: &[AppManifest]) -> Vec<(String, Box<dyn App>)> {
        let mut seen = HashSet::new();
        manifests
            .iter()
            .filter(|m| {
                if seen.insert(m.id.clone()) { true } else {
                    eprintln!("apps: duplicate id '{}' skipped", m.id); false
                }
            })
            .filter_map(|m| match self.constructors.get(&m.id) {
                Some(make) => Some((m.name.clone(), Box::new(runtime::LazyApp::new(m.id.clone(), Rc::clone(make), Arc::clone(&self.audio_bus))) as Box<dyn App>)),
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
        let registry = Registry { constructors, audio_bus: Arc::new(AudioBus::new()) };
        let manifests = vec![
            AppManifest { id: "removed".into(), name: "Unavailable".into() },
            AppManifest { id: "mixer".into(), name: "Renamed service".into() },
            AppManifest { id: "mixer".into(), name: "Duplicate".into() },
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
