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
use crate::apps::{
    analyzer::AnalyzerApp, bloom::BloomApp, cascade::CascadeApp, clouds::CloudsApp, cv_out::CvOutApp, madness::MadnessApp,
    mixer::MixerApp, nebula::NebulaApp, pams::PamsApp, plaits::PlaitsApp, prism::PrismApp, sequencer::SequencerApp,
    settings::SettingsApp, singularity::SingularityApp, synth::SynthApp, tape::TapeApp, voltage::VoltageApp,
};
use crate::audio_bus::AudioBus;
use crate::audio_devices::AudioDeviceState;
use crate::manifest::AppManifest;
use crate::apps::prism::PrismCcTargets;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use std::collections::HashMap;
use std::sync::Arc;

type Constructor = Box<dyn Fn() -> Box<dyn App>>;

pub struct Registry {
    constructors: HashMap<String, Constructor>,
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
    ) -> Self {
        let mut constructors: HashMap<String, Constructor> = HashMap::new();

        constructors.insert("mixer".into(), {
            let master_volume = Arc::clone(&master_volume);
            let mixer_bus = Arc::clone(&mixer_bus);
            let nav_speed = Arc::clone(&nav_speed);
            Box::new(move || {
                Box::new(MixerApp::new(Arc::clone(&master_volume), Arc::clone(&mixer_bus), Arc::clone(&nav_speed))) as Box<dyn App>
            })
        });

        constructors.insert("synth".into(), {
            let cutoff = Arc::clone(&cutoff);
            let sensitivity = Arc::clone(&sensitivity);
            Box::new(move || Box::new(SynthApp::new(Arc::clone(&cutoff), Arc::clone(&sensitivity))) as Box<dyn App>)
        });
        constructors.insert("settings".into(), {
            let devices = Arc::clone(&devices);
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            Box::new(move || {
                Box::new(SettingsApp::new(Arc::clone(&devices), Arc::clone(&sensitivity), Arc::clone(&nav_speed)))
                    as Box<dyn App>
            })
        });
        constructors.insert("plaits".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Box::new(move || {
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
            Box::new(|| Box::new(AnalyzerApp::new()) as Box<dyn App>),
        );
        constructors.insert("sequencer".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Box::new(move || {
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
            Box::new(move || {
                Box::new(PamsApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus))) as Box<dyn App>
            })
        });
        constructors.insert("madness".into(), {
            let sensitivity = Arc::clone(&sensitivity);
            let nav_speed = Arc::clone(&nav_speed);
            let modbus = Arc::clone(&modbus);
            let audio_bus = Arc::clone(&audio_bus);
            let mixer_bus = Arc::clone(&mixer_bus);
            Box::new(move || {
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
            Box::new(move || {
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
            Box::new(move || {
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
            Box::new(move || {
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
            Box::new(move || {
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
            Box::new(move || {
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
            Box::new(move || {
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
            Box::new(move || {
                Box::new(TapeApp::new(
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
            Box::new(move || {
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
            Box::new(move || {
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

        Self { constructors }
    }

    /// Builds the launcher's app list from discovered manifests, in the
    /// order given. A manifest with no matching implementation is skipped
    /// with a warning rather than panicking the whole OS.
    pub fn build(&self, manifests: &[AppManifest]) -> Vec<(String, Box<dyn App>)> {
        manifests
            .iter()
            .filter_map(|m| match self.constructors.get(&m.id) {
                Some(make) => Some((m.name.clone(), make())),
                None => {
                    eprintln!("apps: no implementation registered for id '{}'", m.id);
                    None
                }
            })
            .collect()
    }
}
