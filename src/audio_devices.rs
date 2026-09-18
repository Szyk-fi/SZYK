//! Bridges the Settings app's device picker to the live cpal output
//! stream. Settings can only *request* a switch (from whatever thread
//! `tick()` runs on); the switch itself happens on `Os::run`'s loop,
//! since a cpal `Stream` isn't guaranteed `Send`/`Sync` across threads on
//! every backend -- keeping stream ownership on the one thread that
//! created it sidesteps that entirely.

use crate::audio::ActiveProcessor;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Stream};
use std::sync::{Arc, Mutex};

/// Shared between the Settings app (writes `requested_output`, reads the
/// `current_*` names) and `AudioHost` (performs the switch, writes the
/// result back).
pub struct AudioDeviceState {
    current_output: Mutex<String>,
    current_input: Mutex<String>,
    requested_output: Mutex<Option<String>>,
}

impl AudioDeviceState {
    fn new(initial_output: String) -> Self {
        Self {
            current_output: Mutex::new(initial_output),
            current_input: Mutex::new(String::new()),
            requested_output: Mutex::new(None),
        }
    }

    pub fn current_output(&self) -> String {
        self.current_output.lock().unwrap().clone()
    }

    pub fn current_input(&self) -> String {
        self.current_input.lock().unwrap().clone()
    }

    pub fn request_output(&self, name: String) {
        *self.requested_output.lock().unwrap() = Some(name);
    }

    /// Not wired to any audio processing yet -- this build has no
    /// microphone pipeline (see main.rs) -- but Settings can still record
    /// the preference for whenever an app needs it.
    pub fn set_input(&self, name: String) {
        *self.current_input.lock().unwrap() = name;
    }

    fn take_output_request(&self) -> Option<String> {
        self.requested_output.lock().unwrap().take()
    }

    fn set_current_output(&self, name: String) {
        *self.current_output.lock().unwrap() = name;
    }
}

/// Owns the live output `Stream`. Lives on, and is only ever touched by,
/// the main thread -- see the module doc for why.
pub struct AudioHost {
    host: Host,
    stream: Stream,
    engine: ActiveProcessor,
}

impl AudioHost {
    pub fn open_default(
        engine: ActiveProcessor,
    ) -> Result<(Self, AudioDeviceState), Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no output device found")?;
        let name = device.name()?;
        println!("Output device: {name}");

        let stream = build_stream(&device, Arc::clone(&engine))?;
        stream.play()?;

        let state = AudioDeviceState::new(name);
        Ok((Self { host, stream, engine }, state))
    }

    /// Call once per frame from the main thread. If Settings has
    /// requested a different output device, tears down the old stream
    /// and builds a new one in its place.
    pub fn poll(&mut self, state: &AudioDeviceState) {
        let Some(name) = state.take_output_request() else {
            return;
        };
        match self.switch_to(&name) {
            Ok(()) => state.set_current_output(name),
            Err(e) => eprintln!("audio: couldn't switch to '{name}': {e}"),
        }
    }

    fn switch_to(&mut self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let device = self
            .host
            .output_devices()?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            .ok_or("device not found")?;
        let stream = build_stream(&device, Arc::clone(&self.engine))?;
        stream.play()?;
        self.stream = stream; // dropping the old stream stops it
        Ok(())
    }
}

fn build_stream(device: &Device, engine: ActiveProcessor) -> Result<Stream, Box<dyn std::error::Error>> {
    let config = device.default_output_config()?;
    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;

    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _| {
            engine.process(data, channels, sample_rate);
            crate::run_inference(data);
        },
        |err| eprintln!("output stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

pub fn output_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
