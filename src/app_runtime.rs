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
    outputs: std::ops::Range<usize>,
}
impl LazyApp {
    pub fn new(id: String, make: Rc<dyn Fn() -> Box<dyn App>>, bus: Arc<AudioBus>) -> Self {
        Self { id, make, instance: None, processor: Arc::new(Mutex::new(None)), enabled: Arc::new(AtomicBool::new(false)), screen_open: false, last_input: None, bus, outputs: 0..0 }
    }
    fn ensure(&mut self) -> &mut dyn App {
        if self.instance.is_none() {
            let first = self.bus.len();
            let mut app = (self.make)();
            self.outputs = first..self.bus.len();
            *self.processor.lock().unwrap() = app.audio_processor();
            self.instance = Some(app);
        }
        self.instance.as_mut().unwrap().as_mut()
    }
    fn wake(&mut self) { self.last_input = Some(Instant::now()); self.enabled.store(true, Ordering::Release); }
    fn update_activity(&mut self) {
        let needed = self.instance.as_ref().is_some_and(|app| self.screen_open || app.running() == Some(true) || app.needs_background_audio()
            || self.last_input.is_some_and(|t| t.elapsed() < Duration::from_secs(5)));
        if self.enabled.swap(needed, Ordering::AcqRel) && !needed {
            for index in self.outputs.clone() { if let Some(buffer) = self.bus.get(index) { buffer.lock().unwrap().fill(0.0); } }
        }
    }
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
    fn transport_action(&self) -> Option<&'static str> { self.instance.as_ref().and_then(|a| a.transport_action()) }
    fn running(&self) -> Option<bool> { self.instance.as_ref().and_then(|a| a.running()) }
    fn grid_mode_label(&self) -> Option<&'static str> { self.instance.as_ref().and_then(|a| a.grid_mode_label()) }
    fn on_enter(&mut self) { self.screen_open = true; self.ensure().on_enter(); self.wake(); }
    fn on_exit(&mut self) { if let Some(app) = self.instance.as_mut() { app.on_exit(); } self.screen_open = false; self.update_activity(); }
    fn tick(&mut self, input: &Input) { self.ensure().tick(input); self.wake(); }
    fn draw(&mut self, fb: &mut FrameBuffer) { self.ensure().draw(fb); }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> { Some(Box::new(DeferredProcessor { processor: Arc::clone(&self.processor), enabled: Arc::clone(&self.enabled) })) }
    fn toggle_running(&mut self) { self.ensure().toggle_running(); self.wake(); }
    fn toggle_grid_mode(&mut self) { self.ensure().toggle_grid_mode(); self.wake(); }
    fn background_tick(&mut self) { self.update_activity(); if self.enabled.load(Ordering::Acquire) { if let Some(app) = self.instance.as_mut() { app.background_tick(); } } }
    fn slint_pointer_pick(&mut self, x:f32, y:f32) { self.ensure().slint_pointer_pick(x,y); self.wake(); }
    fn grid_led_overlay(&self) -> [PadColor;16] { self.instance.as_ref().map_or([PadColor::Off;16], |a| a.grid_led_overlay()) }
    fn slint_rows(&self) -> Vec<(String,String,bool)> { self.instance.as_ref().map_or_else(Vec::new, |a| a.slint_rows()) }
    fn slint_selected(&self) -> usize { self.instance.as_ref().map_or(0, |a| a.slint_selected()) }
    fn slint_windowed_rows(&mut self, n:usize) -> (Vec<(String,String,bool)>,usize,bool,bool) { self.ensure().slint_windowed_rows(n) }
    fn slint_levels(&mut self, n:usize) -> Vec<Option<f32>> { self.ensure().slint_levels(n) }
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
