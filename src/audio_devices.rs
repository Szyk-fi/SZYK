//! Bridges the Settings app's device picker to the live cpal output
//! stream. Settings can only *request* a switch (from whatever thread
//! `tick()` runs on); the switch itself happens on `Os::run`'s loop,
//! since a cpal `Stream` isn't guaranteed `Send`/`Sync` across threads on
//! every backend -- keeping stream ownership on the one thread that
//! created it sidesteps that entirely.

use crate::audio::ActiveProcessor;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Stream};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::collections::VecDeque;

/// Shared between the Settings app (writes `requested_output`, reads the
/// `current_*` names) and `AudioHost` (performs the switch, writes the
/// result back).
pub struct AudioDeviceState {
    current_output: Mutex<String>,
    current_input: Mutex<String>,
    requested_output: Mutex<Option<String>>,
    requested_input: Mutex<Option<String>>,
}

impl AudioDeviceState {
    pub(crate) fn new(initial_output: String) -> Self {
        Self {
            current_output: Mutex::new(initial_output),
            current_input: Mutex::new("Off".into()),
            requested_input: Mutex::new(None),
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

    /// Explicit user selection starts capture; startup never opens a microphone.
    pub fn set_input(&self, name: String) {
        *self.requested_input.lock().unwrap() = Some(name);
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
    stream: Option<Stream>,
    engine: ActiveProcessor,
    input_stream: Option<Stream>,
    input_bridge: Option<Arc<InputBridge>>,
}

impl AudioHost {
    pub fn open_default(
        engine: ActiveProcessor,
    ) -> Result<(Self, AudioDeviceState), Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no output device found")?;
        let name = device.name().unwrap_or_else(|_| "Default output".into());
        println!("Output device: {name}");

        let stream = build_stream(&device, Arc::clone(&engine))?;
        stream.play()?;

        let state = AudioDeviceState::new(name);
        Ok((Self { host, stream: Some(stream), engine, input_stream:None, input_bridge:None }, state))
    }

    /// Keep device failure separate from app/UI startup. The same host can
    /// later acquire a real stream through Settings without restarting apps.
    pub fn open_resilient(engine: ActiveProcessor) -> (Self, AudioDeviceState) {
        Self::open_or_offline(engine, Self::open_default)
    }

    fn open_or_offline(
        engine: ActiveProcessor,
        open: impl FnOnce(ActiveProcessor) -> Result<(Self, AudioDeviceState), Box<dyn std::error::Error>>,
    ) -> (Self, AudioDeviceState) {
        match open(Arc::clone(&engine)) {
            Ok(opened) => opened,
            Err(error) => {
                eprintln!("audio: output unavailable; continuing without audio: {error}");
                (Self { host: cpal::default_host(), stream: None, engine, input_stream:None, input_bridge:None },
                 AudioDeviceState::new("Unavailable — select output".into()))
            }
        }
    }

    pub fn is_connected(&self) -> bool { self.stream.is_some() }

    /// Call once per frame from the main thread. If Settings has
    /// requested a different output device, tears down the old stream
    /// and builds a new one in its place.
    pub fn poll(&mut self, state: &AudioDeviceState) {
        let input_request=state.requested_input.lock().unwrap().take();
        if let Some(name)=input_request {
            match self.switch_input(&name) {
                Ok(())=>*state.current_input.lock().unwrap()=name,
                Err(e)=>{*state.current_input.lock().unwrap()=format!("Unavailable: {e}");eprintln!("audio input: {e}");}
            }
        }
        let Some(name) = state.take_output_request() else {
            return;
        };
        match self.switch_to(&name) {
            Ok(()) => state.set_current_output(name),
            Err(e) => eprintln!("audio: couldn't switch to '{name}': {e}"),
        }
    }

    /// The bridge is dormant until Settings selects an input device. Its output
    /// port is metadata only at startup; the stream and queue are created on demand.
    pub fn attach_input(&mut self, output: Arc<Mutex<Vec<f32>>>) {
        let bridge=Arc::new(InputBridge { queue:Mutex::new(VecDeque::new()), rate:crate::util::AtomicF32::new(48000.), enabled:AtomicBool::new(false), output });
        self.engine.add(Box::new(InputReader { bridge:bridge.clone(), phase:0. }));
        self.input_bridge=Some(bridge);
    }
    fn switch_input(&mut self,name:&str)->Result<(),Box<dyn std::error::Error>> {
        let bridge=self.input_bridge.as_ref().ok_or("Input capture requires the main UI")?.clone();
        if name=="Off" { self.input_stream=None;bridge.enabled.store(false,Ordering::Release);bridge.queue.lock().unwrap().clear();bridge.output.lock().unwrap().fill(0.);return Ok(()) }
        let device=self.host.input_devices()?.find(|d|d.name().map(|n|n==name).unwrap_or(false)).ok_or("input device not found")?;
        let config=device.default_input_config()?;
        bridge.queue.lock().unwrap().reserve(48000);
        let input_rate=config.sample_rate().0 as f32;
        let stream=match config.sample_format() {
            cpal::SampleFormat::F32=>build_input::<f32>(&device,&config.into(),bridge.clone())?,
            cpal::SampleFormat::I16=>build_input::<i16>(&device,&config.into(),bridge.clone())?,
            cpal::SampleFormat::U16=>build_input::<u16>(&device,&config.into(),bridge.clone())?,
            cpal::SampleFormat::I32=>build_input::<i32>(&device,&config.into(),bridge.clone())?,
            _=>return Err("Unsupported capture sample format".into()),
        };
        stream.play()?;
        self.input_stream=Some(stream);bridge.queue.lock().unwrap().clear();bridge.rate.set(input_rate);bridge.enabled.store(true,Ordering::Release);Ok(())
    }

    fn switch_to(&mut self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let device = if name == DEFAULT_OUTPUT {
            self.host.default_output_device().ok_or("no default output device found")?
        } else {
            self.host.output_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .ok_or("device not found")?
        };
        let stream = build_stream(&device, Arc::clone(&self.engine))?;
        stream.play()?;
        self.stream = Some(stream); // dropping the old stream stops it
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

pub const DEFAULT_OUTPUT: &str = "System default (retry)";

pub fn output_device_names() -> Vec<String> {
    let host = cpal::default_host();
    let mut names = vec![DEFAULT_OUTPUT.to_string()];
    if let Ok(devices) = host.output_devices() {
        names.extend(devices.filter_map(|d| d.name().ok()));
    }
    names
}

pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    let mut names=vec!["Off".into()];
    if let Ok(devices)=host.input_devices(){names.extend(devices.filter_map(|d|d.name().ok()));}
    names
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn unavailable_audio_preserves_engine_and_accepts_reconnection_request() {
        let engine = crate::audio::new_engine(Arc::new(crate::util::AtomicF32::new(1.0)));
        let (mut host, state) = AudioHost::open_or_offline(Arc::clone(&engine), |_| Err("test unavailable device".into()));
        assert!(!host.is_connected());
        assert!(Arc::ptr_eq(&host.engine, &engine));
        assert!(state.current_output().contains("Unavailable"));
        host.poll(&state); // no request: no driver access, no crash
        state.request_output(DEFAULT_OUTPUT.into());
        assert_eq!(state.take_output_request().as_deref(), Some(DEFAULT_OUTPUT));
        assert!(state.take_output_request().is_none());
    }
}

struct InputBridge { queue:Mutex<VecDeque<f32>>,rate:crate::util::AtomicF32,enabled:AtomicBool,output:Arc<Mutex<Vec<f32>>> }
struct InputReader { bridge:Arc<InputBridge>,phase:f32 }
impl crate::audio::AudioProcessor for InputReader {
    fn is_active(&self)->bool{self.bridge.enabled.load(Ordering::Acquire)}
    fn process(&mut self,out:&mut[f32],channels:usize,rate:f32){
        out.fill(0.);if channels==0{return}
        // Input is published to the patch bus only. Monitoring is app-controlled.
        if let (Ok(mut q),Ok(mut dest))=(self.bridge.queue.try_lock(),self.bridge.output.try_lock()){
            dest.clear();let step=self.bridge.rate.get()/rate.max(1.);
            for _ in 0..out.len()/channels {
                if q.len()<2{dest.push(0.);continue}
                dest.push(q[0]*(1.-self.phase)+q[1]*self.phase);
                self.phase+=step;while self.phase>=1. {q.pop_front();self.phase-=1.;}
            }
        }
    }
}
fn build_input<T>(device:&Device,config:&cpal::StreamConfig,bridge:Arc<InputBridge>)->Result<Stream,cpal::BuildStreamError>
where T:cpal::SizedSample, f32:cpal::FromSample<T> {
    let channels=config.channels as usize;
    device.build_input_stream(config,move|samples:&[T],_|{
        if let Ok(mut queue)=bridge.queue.try_lock(){
            for frame in samples.chunks(channels){let mono=frame.iter().map(|s|<f32 as cpal::FromSample<T>>::from_sample_(*s)).sum::<f32>()/channels as f32;
                if queue.len()>=48000{queue.pop_front();}queue.push_back(if mono.is_finite(){mono}else{0.});}
        }
    },|e|eprintln!("capture stream: {e}"),None)
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::audio::AudioProcessor;
    #[test]
    fn input_bridge_is_dormant_and_resamples_without_direct_monitoring() {
        let output=Arc::new(Mutex::new(Vec::new()));
        let bridge=Arc::new(InputBridge{queue:Mutex::new(VecDeque::from(vec![0.,0.5,1.,0.5,0.,-0.5,-1.,-0.5])),rate:crate::util::AtomicF32::new(24000.),enabled:AtomicBool::new(false),output:output.clone()});
        let mut reader=InputReader{bridge:bridge.clone(),phase:0.};assert!(!reader.is_active());
        bridge.enabled.store(true,Ordering::Relaxed);let mut audio=[1.;8];reader.process(&mut audio,2,48000.);
        assert!(audio.iter().all(|v|*v==0.));assert_eq!(*output.lock().unwrap(),vec![0.,0.25,0.5,0.75]);
        let state=AudioDeviceState::new("test".into());state.set_input("microphone".into());assert_eq!(state.current_input(),"Off");assert_eq!(state.requested_input.lock().unwrap().as_deref(),Some("microphone"));
    }
}
