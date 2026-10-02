//! Forge: clips and recorded AudioBus input become independently playable instruments.
//! Decoding, analysis, persistence and large-buffer destruction stay off the audio thread.
#[path="forge_analysis.rs"]
mod analysis;
use analysis::{Instrument,Chunk,RATE,MAX_FRAMES};
use crate::{app::{App,Input,SlintExtra,ForgeExtra},audio::AudioProcessor,audio_bus::{AudioBus,NO_SOURCE,cycle_source},display::FrameBuffer,modbus::ModBus,mixer_bus::MixerBus,paramlist::ParamList,util::AtomicF32};
use std::{path::{Path,PathBuf},sync::{Arc,Mutex,mpsc::{self,SyncSender,Receiver,TrySendError},atomic::{AtomicBool,AtomicUsize,Ordering}},time::{SystemTime,UNIX_EPOCH}};
use serde::{Serialize,Deserialize};
use crate::app::play_kit::{self as kit,KitConfig,Knob,Layer,PlayHost,PlayKit,Routes};
const MODES:[&str;4]=["Chops","Chromatic","Mosaic","Frankenstein"];
const MEDIA:&str=concat!(env!("CARGO_MANIFEST_DIR"),"/media");
const CAPTURE:usize=384000;
#[derive(Clone,Serialize,Deserialize)]
struct Settings{mode:usize,root:usize,transpose:f32,attack:f32,release:f32,playback:usize,sensitivity:f32}
impl Default for Settings{fn default()->Self{Self{mode:0,root:60,transpose:0.,attack:0.005,release:0.15,playback:0,sensitivity:0.5}}}
#[derive(Serialize,Deserialize)]struct Saved{version:u32,name:String,chunks:Vec<Chunk>,settings:Settings}
enum Job{Scan,Load(PathBuf,f32),Analyze(Vec<f32>,f32,String,f32),Save(Arc<Instrument>,Settings)}
enum ResultMessage{Files(Vec<PathBuf>),Ready(Result<(Instrument,Option<Settings>),String>),Saved(Result<PathBuf,String>)}
enum Command{Record(Vec<f32>)}
struct Captured{samples:Vec<f32>,rate:f32}
struct Shared{
 instrument:Mutex<Option<Arc<Instrument>>>,source:AtomicUsize,mode:AtomicUsize,root:AtomicUsize,transpose:AtomicF32,attack:AtomicF32,release:AtomicF32,playback:AtomicUsize,
 input:AtomicUsize,touch:AtomicUsize,selected:AtomicUsize,audition:AtomicBool,recording:AtomicBool,record_frames:AtomicUsize,audible:AtomicBool,
 output:Arc<Mutex<Vec<f32>>>,mix:Arc<AtomicF32>,ext:Arc<AtomicF32>,wave:Mutex<Vec<f32>>,voice_levels:Mutex<[f32;16]>,
}
pub struct ForgeApp{p:Arc<Shared>,bus:Arc<AudioBus>,own:usize,list:ParamList,nav:Arc<AtomicF32>,files:Vec<PathBuf>,file:usize,status:String,busy:bool,sensitivity:f32,jobs:SyncSender<Job>,results:Receiver<ResultMessage>,commands:SyncSender<Command>,command_rx:Option<Receiver<Command>>,captured:Receiver<Captured>,capture_tx:Option<SyncSender<Captured>>,retired:Receiver<Arc<Instrument>>,retire_tx:Option<SyncSender<Arc<Instrument>>>,kit:PlayKit}
/// The play view's controls as (menu row, label), most important first: the envelope and the
/// pitch/level of what the pads play take the knobs, then how chunks land on the pads; chunk
/// trimming, rebuild, capture, save and load sit on the upper Controls pads, where knob 2's
/// press does the same action the menu row's press does.
const KIT:[(usize,&str);16]=[(11,"Attack"),(12,"Release"),(10,"Transpose"),(14,"Output"),(3,"Build Mode"),(13,"Playback"),(5,"Chunk"),(9,"Key Root"),(6,"Chunk Start"),(7,"Chunk End"),(8,"Sample Root"),(4,"Sensitivity"),(0,"Source"),(2,"Record"),(15,"Save"),(1,"File")];
const C_MODE:usize=4;
fn kit_config()->KitConfig{KitConfig{app_id:"forge",layers:vec![Layer::Native(0,"PLAY"),Layer::Controls,Layer::Moments],hero:vec![[0,1],[2,3],[4,5],[6,7]],
 // Build mode changes what every pad plays at once, so it is what a player flips through.
 browse:Some(C_MODE),
 // Stick: release on X, transpose on Y (a pitch-bend gesture; each hit takes the pitch it is
 // struck at). Hands: attack (swell the hits in) and output level.
 routes:Routes{stick_x:Some(1),stick_y:Some(2),hand_l:Some(0),hand_r:Some(3)},throws:Vec::new(),midi_to_pads:true,own_expression:false}}
// Forge's own palette for the play column: forge-fire orange on soot.
const FORGE_BG:embedded_graphics::pixelcolor::Rgb565=embedded_graphics::pixelcolor::Rgb565::new(3,4,3);
const FORGE_INK:embedded_graphics::pixelcolor::Rgb565=embedded_graphics::pixelcolor::Rgb565::new(30,56,42);
const FORGE_ACCENT:embedded_graphics::pixelcolor::Rgb565=embedded_graphics::pixelcolor::Rgb565::new(31,30,4);
const FORGE_DIM:embedded_graphics::pixelcolor::Rgb565=embedded_graphics::pixelcolor::Rgb565::new(17,27,14);
const FORGE_FAINT:embedded_graphics::pixelcolor::Rgb565=embedded_graphics::pixelcolor::Rgb565::new(7,9,6);
fn scan(root:&Path,depth:usize,out:&mut Vec<PathBuf>){if depth>5||out.len()>=256{return;}if let Ok(entries)=std::fs::read_dir(root){let mut entries:Vec<_>=entries.filter_map(Result::ok).map(|e|e.path()).collect();entries.sort();for path in entries{if out.len()>=256{break;}if path.file_name().is_some_and(|n|n.to_string_lossy().starts_with('.')){continue;}if path.is_dir(){scan(&path,depth+1,out);}else if super::collection::decode::supported(&path)||path.to_string_lossy().ends_with(".forge.toml"){out.push(path);}}}}
fn load(path:&Path,sensitivity:f32)->Result<(Instrument,Option<Settings>),String>{
 let saved=path.to_string_lossy().ends_with(".forge.toml");
 let metadata:Option<Saved>=if saved{let raw=std::fs::read_to_string(path).map_err(|e|e.to_string())?;if raw.len()>65536{return Err("Instrument metadata is too large".into());}Some(toml::from_str(&raw).map_err(|e|format!("Instrument metadata: {e}"))?)}else{None};
 let audio=if saved{path.parent().unwrap_or(Path::new(MEDIA)).join("audio.wav")}else{path.to_path_buf()};let clip=super::collection::decode::load(&audio)?;
 let trimmed=clip.samples.len() as f32/clip.rate>8.;let samples=analysis::resample(&clip.samples,clip.rate);
 if let Some(meta)=metadata{if meta.version!=1||meta.settings.mode>3||meta.settings.playback>2||meta.settings.root>127||!meta.settings.transpose.is_finite()||!(-24.0..=24.0).contains(&meta.settings.transpose)||!meta.settings.attack.is_finite()||!(0.001..=2.).contains(&meta.settings.attack)||!meta.settings.release.is_finite()||!(0.01..=3.).contains(&meta.settings.release)||!meta.settings.sensitivity.is_finite()||!(0.0..=1.0).contains(&meta.settings.sensitivity){return Err("Unsupported or invalid Forge settings".into());}let instrument=Instrument{samples:Arc::new(samples),chunks:meta.chunks,name:meta.name,trimmed};analysis::validate(&instrument)?;Ok((instrument,Some(meta.settings)))}else{Ok((analysis::analyze(samples,clip.name,sensitivity,trimmed)?,None))}
}
fn save(i:&Instrument,settings:Settings)->Result<PathBuf,String>{
 analysis::validate(i)?;let root=Path::new(MEDIA).join("forge");std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
 let id=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();let staging=root.join(format!(".saving-{id}"));let dest=root.join(format!("Instrument-{id}"));std::fs::create_dir(&staging).map_err(|e|e.to_string())?;
 let result=(||{let spec=hound::WavSpec{channels:1,sample_rate:RATE as u32,bits_per_sample:24,sample_format:hound::SampleFormat::Int};let mut writer=hound::WavWriter::create(staging.join("audio.wav"),spec).map_err(|e|e.to_string())?;for s in i.samples.iter(){writer.write_sample((s.clamp(-1.,1.)*8388607.)as i32).map_err(|e|e.to_string())?;}writer.finalize().map_err(|e|e.to_string())?;let raw=toml::to_string(&Saved{version:1,name:i.name.clone(),chunks:i.chunks.clone(),settings}).map_err(|e|e.to_string())?;std::fs::write(staging.join("instrument.forge.toml"),raw).map_err(|e|e.to_string())?;std::fs::rename(&staging,&dest).map_err(|e|e.to_string())?;Ok(dest.join("instrument.forge.toml"))})();
 if result.is_err(){let _=std::fs::remove_dir_all(&staging);}result
}
impl ForgeApp{
 pub fn new(bus:Arc<AudioBus>,mods:Arc<ModBus>,mixer:Arc<MixerBus>,nav:Arc<AtomicF32>)->Self{
  let output=bus.register("Forge");let own=bus.index_of("Forge").unwrap();let(mix,ext)=mixer.register("Forge",&mods);
  let(jobs,rx)=mpsc::sync_channel(2);let(tx,results)=mpsc::channel();std::thread::spawn(move||while let Ok(job)=rx.recv(){let result=match job{
   Job::Scan=>{let mut files=Vec::new();scan(Path::new(MEDIA),0,&mut files);scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples"),0,&mut files);ResultMessage::Files(files)},
   Job::Load(path,s)=>ResultMessage::Ready(load(&path,s)),
   Job::Analyze(samples,rate,name,s)=>{let data:Vec<_>=samples.iter().map(|v|[*v,*v]).collect();ResultMessage::Ready(analysis::analyze(analysis::resample(&data,rate),name,s,false).map(|i|(i,None)))},
   Job::Save(i,s)=>ResultMessage::Saved(save(&i,s)),};if tx.send(result).is_err(){break;}});
  let(commands,command_rx)=mpsc::sync_channel(1);let(capture_tx,captured)=mpsc::sync_channel(1);let(retire_tx,retired)=mpsc::sync_channel(4);
  let app=Self{p:Arc::new(Shared{instrument:Mutex::new(None),source:AtomicUsize::new(NO_SOURCE),mode:AtomicUsize::new(0),root:AtomicUsize::new(60),transpose:AtomicF32::new(0.),attack:AtomicF32::new(0.005),release:AtomicF32::new(0.15),playback:AtomicUsize::new(0),input:AtomicUsize::new(0),touch:AtomicUsize::new(0),selected:AtomicUsize::new(0),audition:AtomicBool::new(false),recording:AtomicBool::new(false),record_frames:AtomicUsize::new(0),audible:AtomicBool::new(false),output,mix,ext,wave:Mutex::new(vec![0.;128]),voice_levels:Mutex::new([0.;16])}),bus,own,list:ParamList::new(),nav,files:Vec::new(),file:0,status:"Import a clip or choose a source to record".into(),busy:false,sensitivity:0.5,jobs,results,commands,command_rx:Some(command_rx),captured,capture_tx:Some(capture_tx),retired,retire_tx:Some(retire_tx),kit:PlayKit::new(kit_config(),!cfg!(test))};let _=app.jobs.try_send(Job::Scan);app
 }
 pub fn load_file(&mut self,path:PathBuf){if !self.busy&&!self.p.recording.load(Ordering::Relaxed){self.submit(Job::Load(path,self.sensitivity),"Analyzing clip…");}}
 fn submit(&mut self,job:Job,status:&str){if self.jobs.try_send(job).is_ok(){self.busy=true;self.status=status.into();}else{self.status="Worker busy; try again".into();}}
 fn settings(&self)->Settings{Settings{mode:self.p.mode.load(Ordering::Relaxed),root:self.p.root.load(Ordering::Relaxed),transpose:self.p.transpose.get(),attack:self.p.attack.get(),release:self.p.release.get(),playback:self.p.playback.load(Ordering::Relaxed),sensitivity:self.sensitivity}}
 fn apply_settings(&mut self,s:Settings){self.p.mode.store(s.mode,Ordering::Relaxed);self.p.root.store(s.root,Ordering::Relaxed);self.p.transpose.set(s.transpose);self.p.attack.set(s.attack);self.p.release.set(s.release);self.p.playback.store(s.playback,Ordering::Relaxed);self.sensitivity=s.sensitivity;}
 fn poll(&mut self){while self.retired.try_recv().is_ok(){}while let Ok(c)=self.captured.try_recv(){self.submit(Job::Analyze(c.samples,c.rate,"Recorded source".into(),self.sensitivity),"Finding playable chunks…");}while let Ok(result)=self.results.try_recv(){match result{
  ResultMessage::Files(files)=>{self.files=files;self.file=self.file.min(self.files.len().saturating_sub(1));},
  ResultMessage::Ready(result)=>{self.busy=false;match result{Ok((i,settings))=>{self.status=format!("{} chunks ready{} • play the pads",i.chunks.len(),if i.trimmed{" / first 8 seconds"}else{""});self.p.selected.store(0,Ordering::Relaxed);self.p.audition.store(false,Ordering::Relaxed);*self.p.instrument.lock().unwrap()=Some(Arc::new(i));if let Some(s)=settings{self.apply_settings(s);}},Err(e)=>self.status=e}},
  ResultMessage::Saved(result)=>{self.busy=false;match result{Ok(path)=>{self.status="Instrument saved • select it in File to reload".into();self.files.push(path);self.file=self.files.len()-1;},Err(e)=>self.status=format!("Save failed: {e}")}}
 }}}
 fn record(&mut self){if self.p.recording.swap(false,Ordering::Relaxed){self.status="Finishing capture…".into();return;}if self.busy{return;}if self.p.source.load(Ordering::Relaxed)==NO_SOURCE{self.status="Choose an audio source before recording".into();return;}if self.commands.try_send(Command::Record(Vec::with_capacity(CAPTURE))).is_ok(){self.p.record_frames.store(0,Ordering::Relaxed);self.p.recording.store(true,Ordering::Relaxed);self.status="Recording • press Record or F3 to finish (8 s max)".into();}}
 fn edit_chunk(&mut self,field:usize,d:f32){let old=self.p.instrument.lock().unwrap().clone();if let Some(old)=old{let mut next=(*old).clone();let idx=self.p.selected.load(Ordering::Relaxed).min(next.chunks.len()-1);let c=&mut next.chunks[idx];match field{6=>c.start=(c.start as f32+d*RATE*0.005).clamp(0.,(c.end-32) as f32)as usize,7=>c.end=(c.end as f32+d*RATE*0.005).clamp((c.start+32)as f32,next.samples.len()as f32)as usize,8=>{c.root=(c.root+d).clamp(0.,127.);c.confidence=1.;},_=>{}}*self.p.instrument.lock().unwrap()=Some(Arc::new(next));}}
 fn action(&mut self,row:usize){if self.busy{return;}match row{1=>{if let Some(path)=self.files.get(self.file){self.load_file(path.clone());}else{self.status="Put audio in media/ and choose Rescan".into();}},2=>self.record(),4=>{let current=self.p.instrument.lock().unwrap().clone();if let Some(i)=current{self.submit(Job::Analyze((*i.samples).clone(),RATE,i.name.clone(),self.sensitivity),"Rebuilding chunks…");}},15=>{let current=self.p.instrument.lock().unwrap().clone();if let Some(i)=current{self.submit(Job::Save(i,self.settings()),"Saving instrument…");}},16=>{let _=self.jobs.try_send(Job::Scan);self.status="Scanning media and samples folders…".into();},_=>{}}}

 /// One knob-2 turn on a menu row; the play view's knobs use the same edits.
 fn edit_row(&mut self,row:usize,d:i32){match row{
  0=>{let mut n=cycle_source(self.p.source.load(Ordering::Relaxed),d.signum(),self.bus.len());if n==self.own{n=cycle_source(n,d.signum(),self.bus.len());}self.p.source.store(n,Ordering::Relaxed);},
  1=>{if !self.files.is_empty(){self.file=(self.file as i32+d).rem_euclid(self.files.len()as i32)as usize;}},
  3=>self.p.mode.store((self.p.mode.load(Ordering::Relaxed)as i32+d).rem_euclid(4)as usize,Ordering::Relaxed),4=>self.sensitivity=(self.sensitivity+d as f32*0.05).clamp(0.,1.),
  5=>{if let Some(i)=self.p.instrument.lock().unwrap().as_ref(){self.p.selected.store((self.p.selected.load(Ordering::Relaxed)as i32+d).rem_euclid(i.chunks.len()as i32)as usize,Ordering::Relaxed);}},6..=8=>self.edit_chunk(row,d as f32),
  9=>self.p.root.store((self.p.root.load(Ordering::Relaxed)as i32+d).clamp(24,96)as usize,Ordering::Relaxed),10=>self.p.transpose.set((self.p.transpose.get()+d as f32).clamp(-24.,24.)),11=>self.p.attack.set((self.p.attack.get()+d as f32*0.005).clamp(0.001,2.)),12=>self.p.release.set((self.p.release.get()+d as f32*0.025).clamp(0.01,3.)),13=>self.p.playback.store((self.p.playback.load(Ordering::Relaxed)as i32+d).rem_euclid(3)as usize,Ordering::Relaxed),14=>self.p.mix.set((self.p.mix.get()+d as f32*0.05).clamp(0.,1.5)),_=>{}}}
 /// The settings' own defaults (`Settings::default`, and the mixer's unity output); press-only
 /// rows do their action, as a knob-2 press on that menu row does.
 fn reset_row(&mut self,row:usize){let d=Settings::default();match row{1|2|4|15|16=>self.action(row),3=>self.p.mode.store(d.mode,Ordering::Relaxed),5=>self.p.selected.store(0,Ordering::Relaxed),9=>self.p.root.store(d.root,Ordering::Relaxed),10=>self.p.transpose.set(d.transpose),11=>self.p.attack.set(d.attack),12=>self.p.release.set(d.release),13=>self.p.playback.store(d.playback,Ordering::Relaxed),14=>self.p.mix.set(1.),_=>{}}}
 fn chunk_count(&self)->usize{self.p.instrument.lock().unwrap().as_ref().map_or(0,|i|i.chunks.len())}
 fn knob(&self,i:usize)->Knob<'_>{match KIT[i%16].0{11=>Knob::F(&self.p.attack,0.001,2.),12=>Knob::F(&self.p.release,0.01,3.),10=>Knob::F(&self.p.transpose,-24.,24.),14=>Knob::F(&*self.p.mix,0.,1.5),_=>Knob::None}}
 fn rows(&self)->Vec<(String,String,bool)>{let instrument=self.p.instrument.lock().unwrap();let chunk=instrument.as_ref().and_then(|i|i.chunks.get(self.p.selected.load(Ordering::Relaxed)));let f=|name:&str,value:String|(name.into(),value,false);
  vec![f("Source",self.bus.source_name(self.p.source.load(Ordering::Relaxed))),f("File / R1 load",self.files.get(self.file).map(|p|p.strip_prefix(MEDIA).unwrap_or(p).to_string_lossy().into_owned()).unwrap_or("No files • Rescan below".into())),f("Record / R1",if self.p.recording.load(Ordering::Relaxed){"STOP"}else{"Start capture"}.into()),f("Build mode",MODES[self.p.mode.load(Ordering::Relaxed)].into()),f("Rebuild / R1",format!("Sensitivity {:.0}%",self.sensitivity*100.)),f("Selected chunk",chunk.map(|c|format!("{} / {}",self.p.selected.load(Ordering::Relaxed)+1,c.class)).unwrap_or("—".into())),f("Chunk start",chunk.map(|c|format!("{:.3} s",c.start as f32/RATE)).unwrap_or("—".into())),f("Chunk end",chunk.map(|c|format!("{:.3} s",c.end as f32/RATE)).unwrap_or("—".into())),f("Sample root",chunk.map(|c|format!("{} / {:.0}%",crate::util::note_name(c.root.round() as i32),c.confidence*100.)).unwrap_or("Unknown".into())),f("Keyboard root",crate::util::note_name(self.p.root.load(Ordering::Relaxed)as i32)),f("Transpose",format!("{:+.0} st",self.p.transpose.get())),f("Attack",format!("{:.0} ms",self.p.attack.get()*1000.)),f("Release",format!("{:.0} ms",self.p.release.get()*1000.)),f("Playback",["One-shot","Gate","Loop"][self.p.playback.load(Ordering::Relaxed)].into()),f("Output",format!("{:.0}%",self.p.mix.get()*100.)),f("Save instrument / R1","Audio + mappings".into()),f("Rescan / R1",format!("{} files",self.files.len()))]
 }
}
impl PlayHost for ForgeApp{
 fn kit_control_count(&self)->usize{KIT.len()}
 fn kit_label(&self,i:usize)->String{KIT[i%16].1.into()}
 fn kit_value(&self,i:usize)->String{self.rows().swap_remove(KIT[i%16].0).1}
 /// Mode, playback, chunk and key root are AtomicUsize (no Knob variant), so by hand. Chunk
 /// trims rebuild the instrument (which silences every voice), so they get no position and are
 /// never pushed by expression or recalled by moments; nor is the source, which only matters
 /// for the next capture.
 fn kit_norm(&self,i:usize)->Option<f32>{let r=Ordering::Relaxed;match KIT[i%16].0{
  3=>Some(self.p.mode.load(r).min(3)as f32/3.),13=>Some(self.p.playback.load(r).min(2)as f32/2.),
  5=>{let n=self.chunk_count();(n>0).then(||if n>1{self.p.selected.load(r).min(n-1)as f32/(n-1)as f32}else{0.})},
  9=>Some((self.p.root.load(r).clamp(24,96)-24)as f32/72.),4=>Some(self.sensitivity),_=>self.knob(i).norm()}}
 fn kit_stepped(&self,i:usize)->bool{!matches!(KIT[i%16].0,4|10|11|12|14)}
 fn kit_edit(&mut self,i:usize,d:i32){self.edit_row(KIT[i%16].0,d);}
 fn kit_reset(&mut self,i:usize){self.reset_row(KIT[i%16].0);}
 fn kit_set_norm(&mut self,i:usize,v:f32){let(v,r)=(v.clamp(0.,1.),Ordering::Relaxed);match KIT[i%16].0{
  3=>self.p.mode.store((v*3.).round()as usize,r),13=>self.p.playback.store((v*2.).round()as usize,r),
  5=>{let n=self.chunk_count();if n>0{self.p.selected.store((v*(n-1)as f32).round()as usize,r);}},
  9=>self.p.root.store(24+(v*72.).round()as usize,r),4=>self.sensitivity=v,_=>self.knob(i).set(v)}}
 /// Chromatic mode pitches pad n at key root + n, so a key plays the pad with its pitch (keys
 /// outside the 16-pad window fold in by octaves). The other modes aren't pitched: the shell's
 /// legacy note % 16 above the controllers' encoder-touch notes, as the kit's default does.
 fn kit_midi_pad(&self,note:u8)->Option<usize>{if self.p.mode.load(Ordering::Relaxed)==1{let d=note as i32-self.p.root.load(Ordering::Relaxed)as i32;Some((if(0..16).contains(&d){d}else{d.rem_euclid(12)})as usize)}else{(note>=21).then_some(note as usize%16)}}
 fn kit_pad_label(&self,_layer:u8,pad:usize)->String{let mode=self.p.mode.load(Ordering::Relaxed);if mode==1{return crate::util::note_name((self.p.root.load(Ordering::Relaxed)+pad)as i32);}let g=self.p.instrument.lock().unwrap();match g.as_ref(){Some(i)=>{let n=mapping(i,mode,pad);format!("{} {}",n+1,i.chunks[n].class.chars().take(4).collect::<String>())},None=>String::new()}}
 /// Held green; a voice still ringing after release yellow; pads with a chunk behind them blue.
 fn kit_pad_color(&self,_layer:u8,pad:usize,held:bool)->crate::led_output::PadColor{use crate::led_output::PadColor;if held{return PadColor::Green;}if self.p.voice_levels.try_lock().is_ok_and(|l|l[pad]>0.01){return PadColor::Yellow;}if self.chunk_count()>0{PadColor::Blue}else{PadColor::Off}}
 fn kit_line(&self)->String{if self.p.recording.load(Ordering::Relaxed){return "RECORDING".into();}if self.busy{return "analyzing...".into();}match self.chunk_count(){0=>"no instrument".into(),n=>format!("{} / {} chunks",MODES[self.p.mode.load(Ordering::Relaxed).min(3)],n)}}
}
impl App for ForgeApp{
 fn play_surface(&self)->bool{true}
 fn play_column(&self)->Option<crate::app::PlayColumn>{(!self.kit.menu).then(||self.kit.column(self))}
 fn grid_mode_label(&self)->Option<&'static str>{Some(self.kit.layer_label())}
 fn toggle_grid_mode(&mut self){self.kit.next_layer();}
 fn grid_led_overlay(&self)->[crate::led_output::PadColor;16]{self.kit.led_overlay(self)}
 fn tick(&mut self,input:&Input){
  // The play view takes the knobs and D-pad first; in the menu they pass straight through.
  // Pads reach the voices only on PLAY (kit layers clear them, so held voices release).
  let mut play=std::mem::take(&mut self.kit);let step=play.tick(self,input);self.kit=play;let input=&step.input;
  self.poll();self.list.navigate_input(input,17,self.nav.get()as i32);self.p.input.store(input.grid.iter().enumerate().fold(0,|m,(i,v)|m|if *v{1<<i}else{0}),Ordering::Relaxed);let d=input.knob2;if d!=0{self.edit_row(self.list.selected,d);}if input.knob1_press||input.knob2_press{self.action(self.list.selected);}}
 fn background_tick(&mut self){self.poll();}
 fn needs_background_audio(&self)->bool{self.busy||self.p.recording.load(Ordering::Relaxed)||self.p.audible.load(Ordering::Relaxed)}
 fn supports_pad_lock(&self)->bool{true}
 fn running(&self)->Option<bool>{Some(self.p.audition.load(Ordering::Relaxed)||self.p.recording.load(Ordering::Relaxed))}
 fn transport_action(&self)->Option<&'static str>{Some(if self.p.recording.load(Ordering::Relaxed){"STOP REC"}else if self.p.audition.load(Ordering::Relaxed){"STOP"}else{"AUDITION"})}
 fn toggle_running(&mut self){if self.p.recording.load(Ordering::Relaxed){self.record();}else if self.p.instrument.lock().unwrap().is_some(){self.p.audition.fetch_xor(true,Ordering::Relaxed);}else{self.status="Import or record sound first".into();}}
 fn slint_rows(&self)->Vec<(String,String,bool)>{self.rows()}
 fn slint_selected(&self)->usize{self.list.selected}
 fn slint_windowed_rows(&mut self,n:usize)->(Vec<(String,String,bool)>,usize,bool,bool){let rows=self.rows();let start=self.list.selected.saturating_sub(n.saturating_sub(1));let end=(start+n).min(rows.len());(rows[start..end].to_vec(),self.list.selected-start,start>0,end<rows.len())}
 fn draw(&mut self,fb:&mut FrameBuffer){if self.kit.menu{let rows=self.rows().into_iter().map(|(a,b,_)|(a,b)).collect::<Vec<_>>();self.list.draw(fb,16,44,20,10,&rows);}else if let Some(col)=self.play_column(){
  use embedded_graphics::{prelude::*,text::Text,mono_font::MonoTextStyle};
  kit::draw::column(fb,&col,16,40,350,280,kit::draw::Palette{bg:FORGE_BG,ink:FORGE_INK,accent:FORGE_ACCENT,dim:FORGE_DIM,faint:FORGE_FAINT});
  Text::new("knobs: envelope/pitch   D-pad: build mode   F2: pads   F3: audition   R1: menu",Point::new(16,337),MonoTextStyle::new(&crate::spleen_fonts::SPLEEN_6X12,FORGE_DIM)).draw(fb).ok();}}
 fn slint_pointer_pick(&mut self,x:f32,y:f32){if x>=0.&&x<16.{let pad=x as usize;self.p.selected.store(pad.min(self.p.instrument.lock().unwrap().as_ref().map_or(0,|i|i.chunks.len()-1)),Ordering::Relaxed);if y>0.{self.p.touch.fetch_or(1<<pad,Ordering::Relaxed);}else{self.p.touch.fetch_and(!(1<<pad),Ordering::Relaxed);}}else if x==-1.{self.record();}else if x==-2.{self.action(1);}else if x==-3.{self.action(4);}else if x==-4.{self.action(15);}}
 fn slint_extra(&mut self)->SlintExtra{self.poll();let current=self.p.instrument.lock().unwrap().clone();let mut starts=Vec::new();let mut ends=Vec::new();let mut labels=Vec::new();let mut waveform=vec![0.;128];let mut name="NO INSTRUMENT".into();let mut duration=0.;if let Some(i)=current{name=i.name.clone();duration=i.samples.len()as f32/RATE;for n in 0..128{let a=n*i.samples.len()/128;let b=((n+1)*i.samples.len()/128).max(a+1).min(i.samples.len());waveform[n]=i.samples[a..b].iter().copied().max_by(|a,b|a.abs().total_cmp(&b.abs())).unwrap_or(0.);}for c in &i.chunks{starts.push(c.start as f32/i.samples.len()as f32);ends.push(c.end as f32/i.samples.len()as f32);labels.push(c.class.clone());}}if self.p.recording.load(Ordering::Relaxed){waveform=self.p.wave.lock().unwrap().clone();}SlintExtra::Forge(ForgeExtra{wave:waveform,starts,ends,labels,name,status:self.status.clone(),mode:MODES[self.p.mode.load(Ordering::Relaxed)].into(),source:self.bus.source_name(self.p.source.load(Ordering::Relaxed)),selected:self.p.selected.load(Ordering::Relaxed)as i32,levels:self.p.voice_levels.lock().unwrap().to_vec(),busy:self.busy,recording:self.p.recording.load(Ordering::Relaxed),duration})}
 fn audio_processor(&mut self)->Option<Box<dyn AudioProcessor>>{Some(Box::new(Processor{p:self.p.clone(),bus:self.bus.clone(),commands:self.command_rx.take()?,captured:self.capture_tx.take()?,retired:self.retire_tx.take()?,retiring:None,current:None,capture:None,pending_capture:None,voices:std::array::from_fn(|_|Voice::default()),previous:0,previous_audition:false,mono:Vec::new()}))}
}
#[derive(Default)]struct Voice{active:bool,position:f32,increment:f32,chunk:usize,env:f32,age:usize,releasing:bool,parts:[usize;3],frankenstein:bool}
struct Processor{p:Arc<Shared>,bus:Arc<AudioBus>,commands:Receiver<Command>,captured:SyncSender<Captured>,retired:SyncSender<Arc<Instrument>>,retiring:Option<Arc<Instrument>>,current:Option<Arc<Instrument>>,capture:Option<Vec<f32>>,pending_capture:Option<Captured>,voices:[Voice;16],previous:usize,previous_audition:bool,mono:Vec<f32>}
fn mapping(i:&Instrument,mode:usize,pad:usize)->usize{match mode{1=>i.chunks.iter().enumerate().max_by(|a,b|(a.1.confidence*(a.1.end-a.1.start)as f32).total_cmp(&(b.1.confidence*(b.1.end-b.1.start)as f32))).map(|(n,_)|n).unwrap_or(0),2=>{let mut order:Vec<_>=(0..i.chunks.len()).collect();order.sort_by(|a,b|i.chunks[*a].brightness.total_cmp(&i.chunks[*b].brightness));order[pad%i.chunks.len()]},_=>pad%i.chunks.len()}}
impl AudioProcessor for Processor{
 fn process(&mut self,out:&mut[f32],channels:usize,rate:f32){out.fill(0.);if channels==0||rate<=0.||!rate.is_finite(){return;}
  if let Some(old)=self.retiring.take(){if let Err(TrySendError::Full(old))=self.retired.try_send(old){self.retiring=Some(old);}}
  if self.retiring.is_none(){if let Ok(next)=self.p.instrument.try_lock(){if let Some(next)=next.as_ref(){if self.current.as_ref().is_none_or(|old|!Arc::ptr_eq(old,next)){self.retiring=self.current.replace(next.clone());for v in &mut self.voices{v.active=false;}}}}}
  if let Some(c)=self.pending_capture.take(){if let Err(TrySendError::Full(c))=self.captured.try_send(c){self.pending_capture=Some(c);}}
  if let Ok(Command::Record(buffer))=self.commands.try_recv(){self.capture=Some(buffer);}
  let frames=out.len()/channels;self.mono.clear();self.mono.resize(frames,0.);
  if let Some(capture)=self.capture.as_mut(){if self.p.recording.load(Ordering::Relaxed){if let Some(input)=self.bus.get(self.p.source.load(Ordering::Relaxed)){if let Ok(input)=input.try_lock(){let limit=(rate*8.)as usize;let room=limit.min(CAPTURE).saturating_sub(capture.len());for n in 0..frames.min(room){let x=input.get(n).copied().unwrap_or(0.);capture.push(if x.is_finite(){x.clamp(-1.,1.)}else{0.});}if let Ok(mut wave)=self.p.wave.try_lock(){for n in 0..128{wave[n]=input.get(n*input.len().max(1)/128).copied().unwrap_or(0.);}}}}self.p.record_frames.store(capture.len(),Ordering::Relaxed);if capture.len()>=(rate*8.)as usize||capture.len()>=CAPTURE{self.p.recording.store(false,Ordering::Relaxed);}}if !self.p.recording.load(Ordering::Relaxed){self.pending_capture=Some(Captured{samples:self.capture.take().unwrap(),rate});}}
  let mask=self.p.input.load(Ordering::Relaxed)|self.p.touch.load(Ordering::Relaxed);let audition=self.p.audition.load(Ordering::Relaxed);let mode=self.p.mode.load(Ordering::Relaxed);let playback=self.p.playback.load(Ordering::Relaxed);let transpose=self.p.transpose.get();let root=self.p.root.load(Ordering::Relaxed)as f32;let attack=self.p.attack.get();let release=self.p.release.get();
  if let Some(instrument)=self.current.as_ref(){for pad in 0..16{let held=mask&(1<<pad)!=0;let hit=held&&self.previous&(1<<pad)==0;let audition_hit=pad==0&&audition&&!self.previous_audition;if hit||audition_hit{let index=if audition_hit{self.p.selected.load(Ordering::Relaxed).min(instrument.chunks.len()-1)}else{mapping(instrument,mode,pad)};let c=&instrument.chunks[index];let semitones=transpose+if mode==1{root+pad as f32-c.root}else{0.};self.voices[pad]=Voice{active:true,position:0.,increment:RATE/rate*2f32.powf(semitones/12.).clamp(0.125,8.),chunk:index,env:0.,age:0,releasing:false,parts:[index,(index+1)%instrument.chunks.len(),(index+2)%instrument.chunks.len()],frankenstein:mode==3};}
    let v=&mut self.voices[pad];if !v.active{continue;}if playback>0&&!held&&!(pad==0&&audition){v.releasing=true;}if pad==0&&!audition&&self.previous_audition{v.releasing=true;}
    for sample in &mut self.mono{if !v.active{break;}let c=&instrument.chunks[v.chunk];let length=(c.end-c.start)as f32;
      if v.position>=length{if playback==2&&!v.releasing{v.position%=length;}else{v.active=false;break;}}
      let phase=v.position/length;let read=|chunk:usize,fraction:f32|{let c=&instrument.chunks[chunk];let pos=c.start as f32+fraction.clamp(0.,0.99999)*(c.end-c.start-1)as f32;let n=pos as usize;let a=instrument.samples[n];let b=instrument.samples[(n+1).min(c.end-1)];a+(b-a)*pos.fract()};
      let value=if v.frankenstein{let part=if phase<0.2{0}else if phase<0.8{1}else{2};let widths=[0.2,0.6,0.2];let starts=[0.,0.2,0.8];let local=(phase-starts[part])/widths[part];let base=read(v.parts[part],local);if part>0&&local<0.08{let t=local/0.08;read(v.parts[part-1],0.92+local)*(1.-t)+base*t}else{base}}else{read(v.chunk,phase)};
      if v.releasing{v.env=(v.env-1./(release*rate).max(1.)).max(0.);if v.env==0.{v.active=false;break;}}else{v.env=(v.env+1./(attack*rate).max(1.)).min(1.);}
      let edge=(v.position/120.).min((length-v.position)/120.).clamp(0.,1.);*sample+=value*v.env*edge*0.25;v.position+=v.increment;v.age+=1;
    }
  }}
  if audition&&!self.voices[0].active{self.p.audition.store(false,Ordering::Relaxed);}
  self.previous=mask;self.previous_audition=self.p.audition.load(Ordering::Relaxed);self.p.audible.store(self.voices.iter().any(|v|v.active),Ordering::Relaxed);
  if let Ok(mut levels)=self.p.voice_levels.try_lock(){for (n,v) in self.voices.iter().enumerate(){levels[n]=if v.active{v.env}else{0.};}}
  for s in &mut self.mono{*s=s.tanh();}if let Ok(mut bus)=self.p.output.try_lock(){bus.clear();bus.extend_from_slice(&self.mono);}let gain=(self.p.mix.get()+self.p.ext.get()).clamp(0.,1.5);for(frame,s)in out.chunks_mut(channels).zip(&self.mono){frame.fill(*s*gain);}
 }
}
#[cfg(test)]mod tests{use super::*;
 fn app()->ForgeApp{let mods=Arc::new(ModBus::new());ForgeApp::new(Arc::new(AudioBus::new()),mods,Arc::new(MixerBus::new()),Arc::new(AtomicF32::new(1.)))}
 #[test]fn opens_playable_knob_1_shapes_the_attack_and_r1_opens_the_menu(){let mut a=app();assert!(a.play_column().is_some(),"play view first");let attack=a.p.attack.get();a.tick(&Input{knob1:3,..Default::default()});assert!(a.p.attack.get()>attack,"knob 1 is Attack on the play view");a.tick(&Input{navigation_steps:-1,..Default::default()});assert_eq!(a.p.mode.load(Ordering::Relaxed),1,"D-pad up = next build mode");a.tick(&Input{shoulder_press:[false,true],..Default::default()});assert!(a.play_column().is_none(),"R1 opens the full menu");}
 /// The pads' held mask is what the audio thread turns into voices.
 #[test]fn play_layer_pads_still_reach_the_voices_and_kit_layers_keep_them(){let mut a=app();assert_eq!(a.kit.layer_label(),"PLAY");a.tick(&Input{grid:std::array::from_fn(|i|i==5),..Default::default()});assert_eq!(a.p.input.load(Ordering::Relaxed),1<<5,"pad 5 held");a.toggle_grid_mode();a.tick(&Input{grid:std::array::from_fn(|i|i==5),..Default::default()});assert_eq!(a.p.input.load(Ordering::Relaxed),0,"Controls pads don't play");}
 #[test]fn chromatic_keys_press_the_pad_with_their_pitch(){let a=app();a.p.mode.store(1,Ordering::Relaxed);a.p.root.store(60,Ordering::Relaxed);assert_eq!(a.kit_midi_pad(64),Some(4));assert_eq!(a.kit_midi_pad(48),Some(0));}
 /// The menu still edits through knob 2 (the shared edit path the play view now uses too).
 #[test]fn the_menu_still_edits_rows(){let mut a=app();a.kit.menu=true;for _ in 0..12{a.tick(&Input{navigation_steps:1,..Default::default()});}assert_eq!(a.list.selected,12,"D-pad still walks the menu");let release=a.p.release.get();a.tick(&Input{knob2:1,..Default::default()});assert!((a.p.release.get()-release-0.025).abs()<1e-5,"knob 2 edits Release in the menu");}
}
