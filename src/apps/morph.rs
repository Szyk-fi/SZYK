//! Dual-source spectral morph. Audio comes from explicit AudioBus inputs;
//! the display reports the processed signal, never a procedural demo waveform.
use crate::{app::{App,Input,SlintExtra,AnalyzerExtra,CurveSegments,polyline_segments,play_kit::{self as kit,KitConfig,Knob,Layer,PlayHost,PlayKit,Routes,Throw}},spleen_fonts::SPLEEN_6X12,audio::AudioProcessor,audio_bus::{AudioBus,NO_SOURCE,cycle_source},modbus::ModBus,mixer_bus::MixerBus,paramlist::ParamList,display::FrameBuffer,util::AtomicF32};
use std::sync::{Arc,Mutex,atomic::{AtomicUsize,AtomicBool,Ordering}};
use rustfft::{FftPlanner,Fft,num_complex::Complex};
use embedded_graphics::{pixelcolor::Rgb565,mono_font::MonoTextStyle,text::Text,prelude::*};
const N:usize=512;
const HOP:usize=128;
struct Params { a:AtomicUsize,b:AtomicUsize,blend:AtomicF32,smooth:AtomicF32,lock:AtomicBool,level:AtomicF32,mix:Arc<AtomicF32>,ext_mix:Arc<AtomicF32>,out:Arc<Mutex<Vec<f32>>> }
pub struct MorphApp { params:Arc<Params>,bus:Arc<AudioBus>,nav:Arc<AtomicF32>,list:ParamList,own:usize,kit:PlayKit }
// Morph's own palette: violet on ink, the colour a spectrum smears into.
const MORPH_BG:Rgb565=Rgb565::new(2,3,6);const MORPH_INK:Rgb565=Rgb565::new(27,54,30);const MORPH_ACCENT:Rgb565=Rgb565::new(24,26,31);const MORPH_DIM:Rgb565=Rgb565::new(13,24,19);const MORPH_FAINT:Rgb565=Rgb565::new(5,8,11);
/// Play-view controls as (menu row, label): the blend is the whole point,
/// smoothing is the second voice of the effect. Sources A/B stay menu-only.
const CONTROLS:[(usize,&str);4]=[(2,"Blend"),(3,"Smooth"),(5,"Level"),(4,"A Phase")];
fn kit_config()->KitConfig{KitConfig{app_id:"morph",
 // No pads before the kit, so the first layer is Throws: momentary jumps
 // to either source, a frozen-looking smear, phase lock and a cut.
 layers:vec![Layer::Throws,Layer::Controls,Layer::Moments],hero:vec![[0,1],[2,3]],
 // D-pad up/down = A-phase lock on/off: the one choice, and it changes the
 // timbre of every blend (A's phases keep A's transients).
 browse:Some(3),
 // Stick X and the left hand sweep A->B; Y and the right hand smear.
 routes:Routes{stick_x:Some(0),stick_y:Some(1),hand_l:Some(0),hand_r:Some(1)},
 throws:vec![Throw{control:0,to:0.0,label:"ALL A"},Throw{control:0,to:0.5,label:"HALF"},Throw{control:0,to:1.0,label:"ALL B"},Throw{control:1,to:1.0,label:"SMEAR"},Throw{control:3,to:1.0,label:"A PHASE"},Throw{control:2,to:0.0,label:"CUT"}],
 midi_to_pads:true,own_expression:false}}
impl MorphApp {
 pub fn new(bus:Arc<AudioBus>,modbus:Arc<ModBus>,mixer:Arc<MixerBus>,nav:Arc<AtomicF32>)->Self {
  let out=bus.register("Morph");let own=bus.index_of("Morph").unwrap();let(mix,ext_mix)=mixer.register("Morph",&modbus);
  Self{params:Arc::new(Params{a:AtomicUsize::new(NO_SOURCE),b:AtomicUsize::new(NO_SOURCE),blend:AtomicF32::new(0.5),smooth:AtomicF32::new(0.0),lock:AtomicBool::new(false),level:AtomicF32::new(0.8),mix,ext_mix,out}),bus,nav,list:ParamList::new(),own,kit:PlayKit::new(kit_config(),!cfg!(test))}
 }
 fn rows(&self)->Vec<(String,String,bool)> { vec![
  ("Source A".into(),self.bus.source_name(self.params.a.load(Ordering::Relaxed)),false),
  ("Source B".into(),self.bus.source_name(self.params.b.load(Ordering::Relaxed)),false),
  ("Blend A → B".into(),format!("{:.0}%",100.0*self.params.blend.get()),false),
  ("Spectral smoothing".into(),format!("{:.0}%",100.0*self.params.smooth.get()),false),
  ("Keep A phase".into(),if self.params.lock.load(Ordering::Relaxed){"on"}else{"off"}.into(),false),
  ("Output level".into(),format!("{:.0}%",100.0*self.params.level.get()),false)] }
 /// One menu row's edit -- shared by the menu (knob 2) and the play view.
 fn edit(&mut self,row:usize,d:i32){
  if d==0{return}
  match row {
   0|1=>{let p=if row==0{&self.params.a}else{&self.params.b};let mut i=cycle_source(p.load(Ordering::Relaxed),d.signum(),self.bus.len());if i==self.own{i=cycle_source(i,d.signum(),self.bus.len());}p.store(i,Ordering::Relaxed);},
   2=>self.params.blend.set((self.params.blend.get()+d as f32*0.02).clamp(0.0,1.0)),
   3=>self.params.smooth.set((self.params.smooth.get()+d as f32*0.02).clamp(0.0,1.0)),
   4=>self.params.lock.store(d>0,Ordering::Relaxed),
   _=>self.params.level.set((self.params.level.get()+d as f32*0.02).clamp(0.0,1.0)) }
 }
 fn knob(&self,i:usize)->Knob<'_>{let p=&self.params;match CONTROLS[i%CONTROLS.len()].0{2=>Knob::F(&p.blend,0.0,1.0),3=>Knob::F(&p.smooth,0.0,1.0),4=>Knob::B(&p.lock),5=>Knob::F(&p.level,0.0,1.0),_=>Knob::None}}
}
impl PlayHost for MorphApp {
 fn kit_control_count(&self)->usize{CONTROLS.len()}
 fn kit_label(&self,i:usize)->String{CONTROLS[i%CONTROLS.len()].1.into()}
 fn kit_value(&self,i:usize)->String{self.rows().swap_remove(CONTROLS[i%CONTROLS.len()].0).1}
 fn kit_norm(&self,i:usize)->Option<f32>{self.knob(i).norm()}
 fn kit_stepped(&self,i:usize)->bool{self.knob(i).stepped()}
 fn kit_edit(&mut self,i:usize,d:i32){self.edit(CONTROLS[i%CONTROLS.len()].0,d)}
 // Morph's menu has no reset gesture; these are the values it starts on.
 fn kit_reset(&mut self,i:usize){match CONTROLS[i%CONTROLS.len()].0{2=>self.params.blend.set(0.5),3=>self.params.smooth.set(0.0),4=>self.params.lock.store(false,Ordering::Relaxed),_=>self.params.level.set(0.8)}}
 fn kit_set_norm(&mut self,i:usize,v:f32){self.knob(i).set(v)}
 fn kit_line(&self)->String{format!("{} > {}",self.bus.source_name(self.params.a.load(Ordering::Relaxed)),self.bus.source_name(self.params.b.load(Ordering::Relaxed)))}
}
impl App for MorphApp {
 fn needs_background_audio(&self)->bool {self.params.a.load(Ordering::Relaxed)!=NO_SOURCE || self.params.b.load(Ordering::Relaxed)!=NO_SOURCE}
 fn tick(&mut self,input:&Input) {
  // The play view takes knobs and D-pad first; in the menu they pass through.
  let mut play=std::mem::take(&mut self.kit);let step=play.tick(self,input);self.kit=play;let input=&step.input;
  self.list.navigate_input(input,6,self.nav.get() as i32);self.edit(self.list.selected,input.knob2);
 }
 fn play_surface(&self)->bool{true}
 fn play_column(&self)->Option<crate::app::PlayColumn>{(!self.kit.menu).then(||self.kit.column(self))}
 fn grid_mode_label(&self)->Option<&'static str>{Some(self.kit.layer_label())}
 fn toggle_grid_mode(&mut self){self.kit.next_layer()}
 fn grid_led_overlay(&self)->[crate::led_output::PadColor;16]{self.kit.led_overlay(self)}
 fn draw(&mut self,fb:&mut FrameBuffer){
  if self.kit.menu{let rows=self.rows().into_iter().map(|(a,b,_)|(a,b)).collect::<Vec<_>>();self.list.draw(fb,16,44,24,rows.len(),&rows);return}
  if let Some(col)=self.play_column(){kit::draw::column(fb,&col,16,40,350,280,kit::draw::Palette{bg:MORPH_BG,ink:MORPH_INK,accent:MORPH_ACCENT,dim:MORPH_DIM,faint:MORPH_FAINT});}
  Text::new("L/R: blend/smooth   U/D: A phase   F2: pads   R1: menu (sources)",Point::new(16,345),MonoTextStyle::new(&SPLEEN_6X12,MORPH_DIM)).draw(fb).ok();
 }
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
 fn app()->MorphApp{MorphApp::new(Arc::new(AudioBus::new()),Arc::new(ModBus::new()),Arc::new(MixerBus::new()),Arc::new(AtomicF32::new(3.0)))}
 #[test]fn opens_playable_knob1_blends_throws_spring_back_and_r1_opens_the_menu(){
  let mut a=app();assert!(a.play_column().is_some(),"play view first");
  a.tick(&Input{knob1:5,..Default::default()});assert!((a.params.blend.get()-0.6).abs()<1e-5,"knob 1 is the blend");
  // Throws is the first layer: rank 2 = ALL B, held then released.
  let pad=kit::rank_pad(2);a.tick(&Input{grid:std::array::from_fn(|i|i==pad),..Default::default()});assert_eq!(a.params.blend.get(),1.0,"held throw");
  a.tick(&Input::default());assert!((a.params.blend.get()-0.6).abs()<1e-5,"springs back to the knob");
  a.tick(&Input{navigation_steps:-1,..Default::default()});assert!(a.params.lock.load(Ordering::Relaxed),"D-pad up locks A's phase");
  a.bus.register("Fixture");a.tick(&Input{shoulder_press:[false,true],..Default::default()});assert!(a.play_column().is_none(),"R1 opens the menu");
  a.tick(&Input{knob2:5,..Default::default()});assert_ne!(a.params.a.load(Ordering::Relaxed),NO_SOURCE,"the menu's knob 2 edits row 0, Source A, again");}
 #[test]fn missing_sources_are_silent_and_real_source_reaches_output(){let bus=Arc::new(AudioBus::new());let source=bus.register("Fixture");let mut app=MorphApp::new(bus.clone(),Arc::new(ModBus::new()),Arc::new(MixerBus::new()),Arc::new(AtomicF32::new(3.0)));let mut dsp=app.audio_processor().unwrap();let mut out=[0.0;1024];dsp.process(&mut out,2,48000.0);assert!(out.iter().all(|v|*v==0.0));app.params.a.store(0,Ordering::Relaxed);app.params.b.store(0,Ordering::Relaxed);*source.lock().unwrap()=(0..512).map(|i|(i as f32*std::f32::consts::TAU/32.0).sin()*0.2).collect();for _ in 0..8{dsp.process(&mut out,2,48000.0);}let rms=(out.iter().map(|v|v*v).sum::<f32>()/out.len()as f32).sqrt();assert!((rms-0.2*0.8/2.0f32.sqrt()).abs()<0.005,"OLA gain: {rms}");assert!(out.iter().all(|v|v.is_finite()));}
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(crate::apps::morph::MorphApp::new(ctx.get(),ctx.get(),ctx.get(),ctx.named("nav_speed")))
}
