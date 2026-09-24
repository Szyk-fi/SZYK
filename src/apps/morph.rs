//! Dual-source spectral morph. Audio comes from explicit AudioBus inputs;
//! the display reports the processed signal, never a procedural demo waveform.
use crate::{app::{App,Input,SlintExtra,AnalyzerExtra,CurveSegments,polyline_segments},audio::AudioProcessor,audio_bus::{AudioBus,NO_SOURCE,cycle_source},modbus::ModBus,mixer_bus::MixerBus,paramlist::ParamList,display::FrameBuffer,util::AtomicF32};
use std::sync::{Arc,Mutex,atomic::{AtomicUsize,AtomicBool,Ordering}};
use rustfft::{FftPlanner,Fft,num_complex::Complex};
const N:usize=512;
const HOP:usize=128;
struct Params { a:AtomicUsize,b:AtomicUsize,blend:AtomicF32,smooth:AtomicF32,lock:AtomicBool,level:AtomicF32,mix:Arc<AtomicF32>,ext_mix:Arc<AtomicF32>,out:Arc<Mutex<Vec<f32>>> }
pub struct MorphApp { params:Arc<Params>,bus:Arc<AudioBus>,nav:Arc<AtomicF32>,list:ParamList,own:usize }
impl MorphApp {
 pub fn new(bus:Arc<AudioBus>,modbus:Arc<ModBus>,mixer:Arc<MixerBus>,nav:Arc<AtomicF32>)->Self {
  let own=bus.len();let out=bus.register("Morph");let(mix,ext_mix)=mixer.register("Morph",&modbus);
  Self{params:Arc::new(Params{a:AtomicUsize::new(NO_SOURCE),b:AtomicUsize::new(NO_SOURCE),blend:AtomicF32::new(0.5),smooth:AtomicF32::new(0.0),lock:AtomicBool::new(false),level:AtomicF32::new(0.8),mix,ext_mix,out}),bus,nav,list:ParamList::new(),own}
 }
 fn rows(&self)->Vec<(String,String,bool)> { vec![
  ("Source A".into(),self.bus.source_name(self.params.a.load(Ordering::Relaxed)),false),
  ("Source B".into(),self.bus.source_name(self.params.b.load(Ordering::Relaxed)),false),
  ("Blend A → B".into(),format!("{:.0}%",100.0*self.params.blend.get()),false),
  ("Spectral smoothing".into(),format!("{:.0}%",100.0*self.params.smooth.get()),false),
  ("Keep A phase".into(),if self.params.lock.load(Ordering::Relaxed){"on"}else{"off"}.into(),false),
  ("Output level".into(),format!("{:.0}%",100.0*self.params.level.get()),false)] }
}
impl App for MorphApp {
 fn needs_background_audio(&self)->bool {self.params.a.load(Ordering::Relaxed)!=NO_SOURCE || self.params.b.load(Ordering::Relaxed)!=NO_SOURCE}
 fn tick(&mut self,input:&Input) {
  self.list.navigate_input(input,6,self.nav.get() as i32);let d=input.knob2;
  if d==0{return}
  match self.list.selected {
   0|1=>{let p=if self.list.selected==0{&self.params.a}else{&self.params.b};let mut i=cycle_source(p.load(Ordering::Relaxed),d.signum(),self.bus.len());if i==self.own{i=cycle_source(i,d.signum(),self.bus.len());}p.store(i,Ordering::Relaxed);},
   2=>self.params.blend.set((self.params.blend.get()+d as f32*0.02).clamp(0.0,1.0)),
   3=>self.params.smooth.set((self.params.smooth.get()+d as f32*0.02).clamp(0.0,1.0)),
   4=>self.params.lock.store(d>0,Ordering::Relaxed),
   _=>self.params.level.set((self.params.level.get()+d as f32*0.02).clamp(0.0,1.0)) }
 }
 fn draw(&mut self,fb:&mut FrameBuffer){let rows=self.rows().into_iter().map(|(a,b,_)|(a,b)).collect::<Vec<_>>();self.list.draw(fb,16,44,24,rows.len(),&rows);}
 fn slint_rows(&self)->Vec<(String,String,bool)>{self.rows()}
 fn slint_selected(&self)->usize{self.list.selected}
 fn audio_processor(&mut self)->Option<Box<dyn AudioProcessor>>{Some(Box::new(MorphProcessor::new(self.params.clone(),self.bus.clone())))}
 fn slint_extra(&mut self)->SlintExtra{
  let output=self.params.out.lock().unwrap();let wave:Vec<_>=output.iter().step_by((output.len()/100).max(1)).take(100).copied().collect();
  let(x,y,l,a)=polyline_segments(&wave,288.0,145.0,true);
  let peak=output.iter().fold(0.0f32,|a,v|a.max(v.abs()));let rms=(output.iter().map(|v|v*v).sum::<f32>()/output.len().max(1)as f32).sqrt();
  SlintExtra::Analyzer(AnalyzerExtra{analyzer_kind:1,analyzer_name:"MORPH / LIVE OUTPUT".into(),spectrum:Vec::new(),waveform:CurveSegments{mid_x:x,mid_y:y,length:l,angle_deg:a},peak_level:peak,rms_level:rms,pitch_name:if self.needs_background_audio(){"SPECTRAL BLEND"}else{"SELECT SOURCE A / B"}.into()})
 }
}
struct MorphProcessor {p:Arc<Params>,bus:Arc<AudioBus>,fft:Arc<dyn Fft<f32>>,ifft:Arc<dyn Fft<f32>>,a:[f32;N],b:[f32;N],ola:[f32;N],pos:usize,hop:usize,window:[f32;N],fa:Vec<Complex<f32>>,fb:Vec<Complex<f32>>,scratch:Vec<Complex<f32>>,magnitudes:[f32;N],mono:Vec<f32>,input_a:Vec<f32>,input_b:Vec<f32>}
impl MorphProcessor {
 fn new(p:Arc<Params>,bus:Arc<AudioBus>)->Self{let mut planner=FftPlanner::new();let fft=planner.plan_fft_forward(N);let ifft=planner.plan_fft_inverse(N);let scratch=vec![Complex::default();fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len())];Self{p,bus,fft,ifft,a:[0.0;N],b:[0.0;N],ola:[0.0;N],pos:0,hop:0,window:std::array::from_fn(|i|0.5-0.5*(std::f32::consts::TAU*i as f32/N as f32).cos()),fa:vec![Complex::default();N],fb:vec![Complex::default();N],scratch,magnitudes:[0.0;N],mono:Vec::new(),input_a:Vec::new(),input_b:Vec::new()}}
 fn frame(&mut self){
  for i in 0..N{let j=(self.pos+i)%N;self.fa[i]=Complex::new(self.a[j]*self.window[i],0.0);self.fb[i]=Complex::new(self.b[j]*self.window[i],0.0);}
  self.fft.process_with_scratch(&mut self.fa,&mut self.scratch);self.fft.process_with_scratch(&mut self.fb,&mut self.scratch);
  let blend=self.p.blend.get();let smooth=self.p.smooth.get()*0.95;let lock=self.p.lock.load(Ordering::Relaxed);
  for i in 0..N{let a=self.fa[i];let b=self.fb[i];let mag=a.norm()*(1.0-blend)+b.norm()*blend;self.magnitudes[i]=self.magnitudes[i]*smooth+mag*(1.0-smooth);
   let phase=if lock{a.arg()}else{let delta=(b.arg()-a.arg()+std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)-std::f32::consts::PI;a.arg()+delta*blend};self.fa[i]=Complex::from_polar(self.magnitudes[i],phase);}
  self.ifft.process_with_scratch(&mut self.fa,&mut self.scratch);
  for i in 0..N{self.ola[(self.pos+i)%N]+=self.fa[i].re*self.window[i]/(N as f32*1.5);}
 }
}
impl AudioProcessor for MorphProcessor {
 fn process(&mut self,out:&mut[f32],channels:usize,_rate:f32){
  out.fill(0.0);if channels==0{return}
  let source_a=self.bus.get(self.p.a.load(Ordering::Relaxed));let source_b=self.bus.get(self.p.b.load(Ordering::Relaxed));
  // Copy outside the processing loop; never acquire one source twice at once.
  self.input_a.clear();self.input_b.clear();
  if let Some(v)=source_a{self.input_a.extend_from_slice(&v.lock().unwrap());}
  if let Some(v)=source_b{self.input_b.extend_from_slice(&v.lock().unwrap());}
  self.mono.clear();let level=self.p.level.get();let mix=(self.p.mix.get()+self.p.ext_mix.get()).clamp(0.0,2.0);
  for (i,frame)in out.chunks_mut(channels).enumerate(){let sample=self.ola[self.pos]*level;self.ola[self.pos]=0.0;self.a[self.pos]=self.input_a.get(i).copied().unwrap_or(0.0);self.b[self.pos]=self.input_b.get(i).copied().unwrap_or(0.0);self.pos=(self.pos+1)%N;self.hop+=1;if self.hop==HOP{self.hop=0;self.frame();}self.mono.push(sample);frame.fill(sample*mix);}
  let mut published=self.p.out.lock().unwrap();published.clear();published.extend_from_slice(&self.mono);
 }
}
#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn missing_sources_are_silent_and_real_source_reaches_output(){let bus=Arc::new(AudioBus::new());let source=bus.register("Fixture");let mut app=MorphApp::new(bus.clone(),Arc::new(ModBus::new()),Arc::new(MixerBus::new()),Arc::new(AtomicF32::new(3.0)));let mut dsp=app.audio_processor().unwrap();let mut out=[0.0;1024];dsp.process(&mut out,2,48000.0);assert!(out.iter().all(|v|*v==0.0));app.params.a.store(0,Ordering::Relaxed);app.params.b.store(0,Ordering::Relaxed);*source.lock().unwrap()=(0..512).map(|i|(i as f32*std::f32::consts::TAU/32.0).sin()*0.2).collect();for _ in 0..8{dsp.process(&mut out,2,48000.0);}let rms=(out.iter().map(|v|v*v).sum::<f32>()/out.len()as f32).sqrt();assert!((rms-0.2*0.8/2.0f32.sqrt()).abs()<0.005,"OLA gain: {rms}");assert!(out.iter().all(|v|v.is_finite()));}
}
