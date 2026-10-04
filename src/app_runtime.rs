//! Lazy application instance and demand-controlled DSP. Never constructs an
//! installed app merely to list its name or register its dormant audio slot.
use crate::app::{App, Input, SlintExtra, SystemRole};
use crate::{audio::AudioProcessor, audio_bus::AudioBus, display::FrameBuffer, led_output::PadColor};
use std::{rc::Rc, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

pub struct LazyApp {
    id: String,
    make: Rc<dyn Fn() -> Box<dyn App>>,
    instance: Option<Box<dyn App>>,
    processor: Arc<Mutex<Option<Box<dyn AudioProcessor>>>>,
    enabled: Arc<AtomicBool>,
    screen_open: bool,
    last_input: Option<Instant>,
    bus: Arc<AudioBus>,
    /// Writing into one of this app's declared modulation inputs wakes it.
    modbus: Option<Arc<crate::modbus::ModBus>>,
    /// Notes other apps play on this one (instruments only), delivered to
    /// the app as MIDI keys -- see note_bus.rs.
    notes: Option<Arc<crate::note_bus::NoteBus>>,
    inbox: Option<crate::note_bus::NoteInboxRef>,
    view: crate::note_bus::NoteView,
    /// Bus notes were held when last delivered off screen (so their
    /// release gets delivered too).
    delivering: bool,
    outputs: Vec<usize>,
}
impl LazyApp {
    pub fn new(id: String, make: Rc<dyn Fn() -> Box<dyn App>>, bus: Arc<AudioBus>) -> Self {
        Self { id, make, instance: None, processor: Arc::new(Mutex::new(None)), enabled: Arc::new(AtomicBool::new(false)), screen_open: false, last_input: None, bus, modbus: None, notes: None, inbox: None, view: Default::default(), delivering: false, outputs: Vec::new() }
    }
    pub fn with_modbus(mut self, modbus: Arc<crate::modbus::ModBus>) -> Self {
        self.modbus = Some(modbus);
        self
    }
    pub fn with_notes(mut self, notes: Arc<crate::note_bus::NoteBus>, inbox: Option<crate::note_bus::NoteInboxRef>) -> Self {
        self.notes = Some(notes);
        self.inbox = inbox;
        self
    }
    /// Bus notes merged into a frame's input, as if played on a keyboard.
    fn with_bus_notes(&mut self, input: &Input) -> Input {
        let mut merged = input.clone();
        if let Some(inbox) = self.inbox.as_ref() {
            inbox.poll(&mut self.view);
            for (k, v) in merged.midi_keys.0.iter_mut().zip(self.view.keys.iter()) {
                *k = (*k).max(*v);
            }
        }
        merged
    }
    fn ensure(&mut self) -> &mut dyn App {
        if self.instance.is_none() {
            let first = self.bus.len();
            let mut app = (self.make)();
            self.outputs=self.bus.owned_indices(&self.id);
            for i in first..self.bus.len(){if !self.outputs.contains(&i){self.outputs.push(i);}}
            let publications=self.outputs.iter().filter(|i|!self.bus.is_claimed(**i)).map(|i|self.bus.register(self.bus.source_name(*i))).collect::<Vec<_>>();
            *self.processor.lock().unwrap() = app.audio_processor().map(|inner| if publications.is_empty(){inner}else{Box::new(PublishingProcessor{inner,publications}) as Box<dyn AudioProcessor>});
            self.instance = Some(app);
        }
        self.instance.as_mut().unwrap().as_mut()
    }
    fn wake(&mut self) { self.last_input = Some(Instant::now()); self.enabled.store(true, Ordering::Release); }
    fn update_activity(&mut self) {
        let requested=self.bus.requested(&self.id) || self.modbus.as_ref().is_some_and(|m| m.requested(&self.id)) || self.notes.as_ref().is_some_and(|n| n.requested(&self.id));
        if requested {self.ensure();}
        let needed = self.instance.as_ref().is_some_and(|app| requested || self.screen_open || app.running() == Some(true) || app.needs_background_audio()
            || self.last_input.is_some_and(|t| t.elapsed() < Duration::from_secs(5)));
        if self.enabled.swap(needed, Ordering::AcqRel) && !needed {
            for index in self.outputs.clone() { if let Some(buffer) = self.bus.peek(index) { buffer.lock().unwrap().fill(0.0); } }
        }
    }
}
struct PublishingProcessor {inner:Box<dyn AudioProcessor>,publications:Vec<Arc<Mutex<Vec<f32>>>>}
impl AudioProcessor for PublishingProcessor {
    fn process(&mut self,out:&mut[f32],channels:usize,rate:f32){self.inner.process(out,channels,rate);if channels==0{return;}for buffer in &self.publications {if let Ok(mut b)=buffer.try_lock(){b.clear();b.extend(out.chunks(channels).map(|frame|frame.iter().sum::<f32>()/frame.len() as f32));}}}
}
struct DeferredProcessor { processor: Arc<Mutex<Option<Box<dyn AudioProcessor>>>>, enabled: Arc<AtomicBool> }
impl AudioProcessor for DeferredProcessor {
    fn is_active(&self) -> bool { self.enabled.load(Ordering::Acquire) }
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        if !self.is_active() { out.fill(0.0); return; }
        if let Some(processor) = self.processor.lock().unwrap().as_mut() { processor.process(out, channels, rate); }
    }
}
impl App for LazyApp {
    fn system_role(&self) -> Option<SystemRole> { match self.id.as_str() { "mixer" => Some(SystemRole::Mixer), "settings" => Some(SystemRole::Settings), _ => self.instance.as_ref().and_then(|a| a.system_role()) } }
    fn supports_pad_lock(&self) -> bool { self.instance.as_ref().is_some_and(|a| a.supports_pad_lock()) }
    fn play_surface(&self) -> bool { self.instance.as_ref().is_some_and(|a| a.play_surface()) }
    fn play_column(&self) -> Option<crate::app::PlayColumn> { self.instance.as_ref().and_then(|a| a.play_column()) }
    fn transport_action(&self) -> Option<&'static str> { self.instance.as_ref().and_then(|a| a.transport_action()) }
    fn running(&self) -> Option<bool> { self.instance.as_ref().and_then(|a| a.running()) }
    fn wants_fullscreen(&self) -> bool { self.instance.as_ref().is_some_and(|a| a.wants_fullscreen()) }
    fn grid_mode_label(&self) -> Option<&'static str> { self.instance.as_ref().and_then(|a| a.grid_mode_label()) }
    fn on_enter(&mut self) { self.screen_open = true; self.ensure().on_enter(); self.wake(); }
    fn on_exit(&mut self) { if let Some(app) = self.instance.as_mut() { app.on_exit(); } self.screen_open = false; self.update_activity(); }
    fn tick(&mut self, input: &Input) { let input = self.with_bus_notes(input); self.delivering = false; self.ensure().tick(&input); self.wake(); }
    fn draw(&mut self, fb: &mut FrameBuffer) { self.ensure().draw(fb); }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> { Some(Box::new(DeferredProcessor { processor: Arc::clone(&self.processor), enabled: Arc::clone(&self.enabled) })) }
    fn toggle_running(&mut self) { self.ensure().toggle_running(); self.wake(); }
    fn toggle_grid_mode(&mut self) { self.ensure().toggle_grid_mode(); self.wake(); }
    fn background_tick(&mut self) {
        self.update_activity();
        // An instrument off screen still plays what other apps send it:
        // its notes arrive as keys on an otherwise empty frame.
        if !self.screen_open && self.inbox.is_some() {
            let input = self.with_bus_notes(&Input::default());
            let any = self.view.any();
            if any || self.delivering {
                self.ensure().tick(&input);
            }
            self.delivering = any;
        }
        if self.enabled.load(Ordering::Acquire) { if let Some(app) = self.instance.as_mut() { app.background_tick(); } }
    }
    fn slint_pointer_pick(&mut self, x:f32, y:f32) { self.ensure().slint_pointer_pick(x,y); self.wake(); }
    fn grid_led_overlay(&self) -> [PadColor;16] { self.instance.as_ref().map_or([PadColor::Off;16], |a| a.grid_led_overlay()) }
    fn slint_rows(&self) -> Vec<(String,String,bool)> { self.instance.as_ref().map_or_else(Vec::new, |a| a.slint_rows()) }
    fn slint_selected(&self) -> usize { self.instance.as_ref().map_or(0, |a| a.slint_selected()) }
    fn slint_windowed_rows(&mut self, n:usize) -> (Vec<(String,String,bool)>,usize,bool,bool) { self.ensure().slint_windowed_rows(n) }
    fn slint_levels(&mut self, n:usize) -> Vec<Option<f32>> { self.ensure().slint_levels(n) }
    fn slint_scale_info(&self) -> Option<crate::app::music_scales::ScaleInfo> { self.instance.as_ref().and_then(|a| a.slint_scale_info()) }
    fn slint_extra(&mut self) -> SlintExtra { self.ensure().slint_extra() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::AtomicUsize};
    struct CounterApp(Arc<AtomicUsize>);
    struct CounterDsp(Arc<AtomicUsize>);
    impl AudioProcessor for CounterDsp { fn process(&mut self, out:&mut [f32], _:usize, _:f32) { self.0.fetch_add(1,Ordering::Relaxed); out.fill(0.1); } }
    impl App for CounterApp {
        fn tick(&mut self,_:&Input) {}
        fn draw(&mut self,_:&mut FrameBuffer) {}
        fn audio_processor(&mut self)->Option<Box<dyn AudioProcessor>> { Some(Box::new(CounterDsp(self.0.clone()))) }
    }
    #[test]
    fn hundred_installed_apps_do_no_dsp_until_used_and_sleep_after_use() {
        let constructions = Rc::new(Cell::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = crate::audio::new_engine(Arc::new(crate::util::AtomicF32::new(1.0)));
        let bus = Arc::new(AudioBus::new());
        let mut apps:Vec<_> = (0..100).map(|i| {
            let count = constructions.clone(); let calls = calls.clone();
            LazyApp::new(format!("app-{i}"), Rc::new(move || { count.set(count.get()+1); Box::new(CounterApp(calls.clone())) }), bus.clone())
        }).collect();
        for app in &mut apps { engine.add(app.audio_processor().unwrap()); app.background_tick(); }
        let mut output = [1.0;64]; engine.process(&mut output,2,48000.0);
        assert_eq!(constructions.get(),0); assert_eq!(calls.load(Ordering::Relaxed),0); assert!(output.iter().all(|v| *v==0.0));
        apps[47].on_enter(); engine.process(&mut output,2,48000.0);
        assert_eq!(constructions.get(),1); assert_eq!(calls.load(Ordering::Relaxed),1);
        apps[47].on_exit(); apps[47].last_input = Some(Instant::now()-Duration::from_secs(6)); apps[47].background_tick();
        engine.process(&mut output,2,48000.0); assert_eq!(calls.load(Ordering::Relaxed),1);
        apps[47].on_enter(); engine.process(&mut output,2,48000.0);
        assert_eq!(constructions.get(),1); assert_eq!(calls.load(Ordering::Relaxed),2);
    }
}

#[cfg(test)] mod demand_tests {
 use super::*;use std::cell::Cell;
 struct Silent;impl App for Silent{fn tick(&mut self,_:&Input){}fn draw(&mut self,_:&mut FrameBuffer){}}
 #[test] fn catalog_read_wakes_only_the_requested_owner(){let bus=Arc::new(AudioBus::new());bus.declare("bloom","Bloom");bus.declare("plaits","Plaits");let count=Rc::new(Cell::new(0));let make={let c=count.clone();Rc::new(move||{c.set(c.get()+1);Box::new(Silent) as Box<dyn App>})};let mut bloom=LazyApp::new("bloom".into(),make.clone(),bus.clone());let mut plaits=LazyApp::new("plaits".into(),make,bus.clone());bus.names();bus.peek(0);bloom.background_tick();plaits.background_tick();assert_eq!(count.get(),0);bus.get(0);bloom.background_tick();plaits.background_tick();assert_eq!(count.get(),1);assert!(bloom.instance.is_some());assert!(plaits.instance.is_none());}
}

#[cfg(test)]
mod routing_lifecycle_tests {
    use super::*;
    use std::{cell::Cell,sync::atomic::AtomicUsize};
    struct TapApp{bus:Arc<AudioBus>,reads:Arc<AtomicUsize>}
    struct TapDsp{bus:Arc<AudioBus>,reads:Arc<AtomicUsize>}
    impl App for TapApp {
        fn tick(&mut self,_:&Input){}fn draw(&mut self,_:&mut FrameBuffer){}
        fn audio_processor(&mut self)->Option<Box<dyn AudioProcessor>>{Some(Box::new(TapDsp{bus:self.bus.clone(),reads:self.reads.clone()}))}
    }
    impl AudioProcessor for TapDsp{
        fn process(&mut self,out:&mut[f32],_:usize,_:f32){out.fill(0.);if let Some(source)=self.bus.get(0){if source.lock().unwrap().iter().any(|s|*s==0.25){self.reads.fetch_add(1,Ordering::Relaxed);}}}
    }
    struct Source;struct Tone;
    impl App for Source{fn tick(&mut self,_:&Input){}fn draw(&mut self,_:&mut FrameBuffer){}fn audio_processor(&mut self)->Option<Box<dyn AudioProcessor>>{Some(Box::new(Tone))}}
    impl AudioProcessor for Tone{fn process(&mut self,out:&mut[f32],_:usize,_:f32){out.fill(0.25);}}
    #[test]
    fn source_is_published_on_demand_and_sleeps_after_the_last_reader_closes(){
        let bus=Arc::new(AudioBus::new());bus.declare("source","Source");
        let builds=Rc::new(Cell::new(0));let count=builds.clone();
        let mut source=LazyApp::new("source".into(),Rc::new(move||{count.set(count.get()+1);Box::new(Source)}),bus.clone());
        let reads=Arc::new(AtomicUsize::new(0));let b=bus.clone();let r=reads.clone();
        let mut sink=LazyApp::new("scope".into(),Rc::new(move||Box::new(TapApp{bus:b.clone(),reads:r.clone()})),bus.clone());
        let engine=crate::audio::new_engine(Arc::new(crate::util::AtomicF32::new(1.)));
        engine.add(source.audio_processor().unwrap());engine.add(sink.audio_processor().unwrap());let mut out=[0.;64];
        sink.on_enter();engine.process(&mut out,2,48000.);assert_eq!(builds.get(),0);
        source.background_tick();engine.process(&mut out,2,48000.);assert_eq!(builds.get(),1);assert_eq!(reads.load(Ordering::Relaxed),1);
        sink.on_exit();sink.last_input=Some(Instant::now()-Duration::from_secs(6));sink.background_tick();
        std::thread::sleep(Duration::from_millis(550));source.background_tick();
        assert!(!source.enabled.load(Ordering::Acquire));engine.process(&mut out,2,48000.);assert!(out.iter().all(|s|*s==0.));
        assert!(bus.peek(0).unwrap().lock().unwrap().iter().all(|s|*s==0.));
        sink.on_enter();engine.process(&mut out,2,48000.);source.background_tick();engine.process(&mut out,2,48000.);
        assert_eq!(builds.get(),1,"reopening reuses source state");assert_eq!(reads.load(Ordering::Relaxed),2);
    }
}
