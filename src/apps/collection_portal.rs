//! Eight independent cables: audio buses, internal CV, and real app parameters.
//! Parameter destinations run at the host block rate; audio sends retain samples.
use crate::{app::{App,Input,SlintExtra,PortalExtra},audio::AudioProcessor,audio_bus::AudioBus,display::FrameBuffer,mixer_bus::MixerBus,modbus::ModBus,paramlist::ParamList,util::{AtomicF32,AtomicContribution}};
use std::sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}};
use std::f32::consts::TAU;
const N:usize=8;
const TRANSFORMS:[&str;4]=["Signal","Envelope","Gate","Sample & hold"];
#[derive(Clone)] struct Cable {
    source:usize,target:usize,amount:f32,offset:f32,slew:f32,mode:usize,enabled:bool,
    cv:Option<Arc<AtomicContribution>>,
}
impl Default for Cable {fn default()->Self {Self{source:0,target:0,amount:0.5,offset:0.,slew:0.,mode:0,enabled:true,cv:None}}}
#[derive(Clone)] struct Patch {cables:[Cable;N],revision:u64}
struct Shared {patch:Mutex<Patch>,active:AtomicBool,monitor:AtomicBool,rates:[AtomicF32;2],rate_cv:[Arc<AtomicF32>;2],meters:[AtomicF32;N],mix:Arc<AtomicF32>,ext:Arc<AtomicF32>,outputs:[Arc<Mutex<Vec<f32>>>;5]}
pub struct PortalApp {p:Arc<Shared>,bus:Arc<AudioBus>,mods:Arc<ModBus>,own:usize,list:ParamList,nav:Arc<AtomicF32>,selected:usize,held:[bool;16],status:String,taken:bool,
    /// The note bus: Portal's Notes rows show and set every app's
    /// App -> Instrument route in one place (see note_bus.rs).
    notes:Option<Arc<crate::note_bus::NoteBus>>}
/// Cable rows; the Notes rows follow.
const CABLE_ROWS:usize=14;
impl PortalApp {
    pub fn new(bus:Arc<AudioBus>,mods:Arc<ModBus>,mixer:Arc<MixerBus>,nav:Arc<AtomicF32>)->Self {
        let outputs=std::array::from_fn(|i|bus.register(if i==0{"Portal".into()}else{format!("Portal Aux {i}")}));
        let own=bus.index_of("Portal").unwrap();
        let (mix,ext)=mixer.register("Portal",&mods);
        let rate_cv=[mods.register("Portal: LFO 1 rate"),mods.register("Portal: LFO 2 rate")];
        Self{p:Arc::new(Shared{patch:Mutex::new(Patch{cables:std::array::from_fn(|_|Cable::default()),revision:0}),active:AtomicBool::new(true),monitor:AtomicBool::new(false),rates:[AtomicF32::new(0.5),AtomicF32::new(0.13)],rate_cv,meters:std::array::from_fn(|_|AtomicF32::new(0.)),mix,ext,outputs}),bus,mods,own,list:ParamList::new(),nav,selected:0,held:[false;16],status:"Choose a cable, source and destination. Pads 1–8 select; 9–16 mute.".into(),taken:false,notes:None}
    }
    pub fn with_notes(mut self,notes:Option<Arc<crate::note_bus::NoteBus>>)->Self{self.notes=notes;self}
    /// The Notes section: a header, then one row per note source
    /// (App -> Instrument), in the order apps declared them.
    fn note_rows(&self)->Vec<(String,String,bool)> {
        let Some(bus)=&self.notes else {return Vec::new()};
        let sources=bus.sources();
        if sources.is_empty(){return Vec::new();}
        let mut r=vec![("NOTES  app → instrument".to_string(),String::new(),true)];
        r.extend(sources.iter().map(|(name,_,route)|(format!("  {name}"),format!("→ {}",bus.route_name(route.load(Ordering::Relaxed))),false)));
        r
    }
    fn source_name(&self,i:usize)->String {match i {0=>"Unpatched".into(),1=>"LFO 1 / sine".into(),2=>"LFO 2 / triangle".into(),3=>"Random / LFO 1 clock".into(),_=>self.bus.source_name(i-4)}}
    fn target_name(&self,i:usize)->String {match i {0=>"Unpatched".into(),1=>"Portal out".into(),2..=5=>format!("Aux {}",i-1),_=>crate::modbus::Patch::label(&self.mods,i-5)}}
    /// Destination row: the six Portal outputs, then one stop per app (its
    /// first input); the Destination Input row then walks that app's inputs.
    fn step_dest(&self,cur:usize,d:i32)->usize {
        let apps=self.mods.apps();
        let pos=if cur<6 {cur} else {6+apps.iter().position(|(_,v)|v.contains(&(cur-6))).unwrap_or(0)};
        let next=(pos as i32+d.signum()).rem_euclid((apps.len()+6) as i32) as usize;
        if next<6 {next} else {apps[next-6].1[0]+6}
    }
    fn rows(&self)->Vec<(String,String,bool)> {
        let p=self.p.patch.lock().unwrap();let c=&p.cables[self.selected];
        vec![("Cable",format!("{} / 8",self.selected+1)),("Source",self.source_name(c.source)),("Destination",if c.target>=6 {crate::modbus::Patch::app_label(&self.mods,c.target-5)} else {self.target_name(c.target)}),("Destination input",if c.target>=6 {crate::modbus::Patch::input_label(&self.mods,c.target-5)} else {"--".into()}),("Amount / polarity",format!("{:+.0}%",c.amount*100.)),("Offset",format!("{:+.2}",c.offset)),("Slew",format!("{:.0} ms",c.slew)),("Transform",TRANSFORMS[c.mode].into()),("Cable enabled",if c.enabled{"on"}else{"muted"}.into()),("LFO 1 rate",format!("{:.2} Hz",self.p.rates[0].get())),("LFO 2 rate",format!("{:.2} Hz",self.p.rates[1].get())),("Direct monitor",if self.p.monitor.load(Ordering::Relaxed){"on"}else{"off"}.into()),("Clear cable / R1","Disconnect selected".into()),("Clear all / R1","Disconnect all".into())].into_iter().map(|(a,b)|(a.into(),b,false)).chain(self.note_rows()).collect()
    }
    fn edit(&mut self,d:i32) {
        let row=self.list.selected;
        if row>CABLE_ROWS {if let Some(bus)=&self.notes{bus.step_source(row-CABLE_ROWS-1,d);}return;}
        if row==CABLE_ROWS {return;}
        if row==0 {self.selected=(self.selected as i32+d.signum()).rem_euclid(N as i32) as usize;return;}
        if row==9||row==10 {let v=&self.p.rates[row-9];v.set((v.get()+d as f32*0.05).clamp(0.01,40.));return;}
        if row==11 {self.p.monitor.store(d>0,Ordering::Relaxed);return;}
        let mut patch=self.p.patch.lock().unwrap();let c=&mut patch.cables[self.selected];
        match row {
            1=>{let n=self.bus.len()+4;let mut next=(c.source as i32+d.signum()).rem_euclid(n as i32) as usize;
                // Self-feedback is never an accidental source-list wrap.
                while next>=4 && (self.own..self.own+5).contains(&(next-4)) {next=(next as i32+d.signum()).rem_euclid(n as i32) as usize;}
                c.source=next;},
            2|3=>{let next=if row==2 {self.step_dest(c.target,d)} else if c.target>=6 {crate::modbus::Patch::step_input(&self.mods,c.target-5,d)+5} else {c.target};
                if next==c.target {return;}
                let cv=if next>=6 {match self.mods.contribution(next-6){Some(h)=>Some(Arc::new(h)),None=>{self.status="Destination has no free cable slots".into();return;}}}else{None};
                if let Some(old)=&c.cv{old.set(0.);} c.target=next;c.cv=cv;},
            4=>c.amount=(c.amount+d as f32*0.05).clamp(-1.,1.),5=>c.offset=(c.offset+d as f32*0.05).clamp(-1.,1.),6=>c.slew=(c.slew+d as f32*10.).clamp(0.,2000.),7=>c.mode=(c.mode as i32+d.signum()).rem_euclid(4) as usize,8=>c.enabled=d>0,_=>{},
        }
        if !c.enabled {if let Some(h)=&c.cv{h.set(0.);}}
        patch.revision+=1;
    }
}
impl App for PortalApp {
    fn tick(&mut self,input:&Input) {
        let n=self.rows().len();self.list.navigate_input(input,n,self.nav.get() as i32);if input.knob2!=0{self.edit(input.knob2);}
        if (input.knob1_press||input.knob2_press)&& (12..CABLE_ROWS).contains(&self.list.selected) {let mut p=self.p.patch.lock().unwrap();for i in 0..N {if self.list.selected==13||i==self.selected{if let Some(h)=&p.cables[i].cv{h.set(0.);}p.cables[i]=Cable::default();}}p.revision+=1;self.status="Cables disconnected; other apps keep their settings".into();}
        for i in 0..16 {if input.grid[i]&&!self.held[i] {if i<8{self.selected=i;}else{let mut p=self.p.patch.lock().unwrap();let c=&mut p.cables[i-8];c.enabled=!c.enabled;if !c.enabled{if let Some(h)=&c.cv{h.set(0.);}}p.revision+=1;}}}self.held=input.grid;
    }

    fn transport_action(&self)->Option<&'static str>{Some(if self.p.active.load(Ordering::Relaxed){"BYPASS"}else{"ENABLE"})}
    fn toggle_running(&mut self){let active=!self.p.active.load(Ordering::Relaxed);self.p.active.store(active,Ordering::Relaxed);if !active{for c in &self.p.patch.lock().unwrap().cables{if let Some(h)=&c.cv{h.set(0.);}}}}
    fn needs_background_audio(&self)->bool{self.p.active.load(Ordering::Relaxed)&&self.p.patch.lock().unwrap().cables.iter().any(|c|c.enabled&&c.source!=0&&c.target!=0)}
    fn draw(&mut self,fb:&mut FrameBuffer){let rows=self.rows().into_iter().map(|(a,b,_)|(a,b)).collect::<Vec<_>>();self.list.draw(fb,16,44,24,rows.len(),&rows);}
    fn slint_rows(&self)->Vec<(String,String,bool)>{self.rows()}
    fn slint_selected(&self)->usize{self.list.selected}
    fn slint_windowed_rows(&mut self,n:usize)->(Vec<(String,String,bool)>,usize,bool,bool){let rows=self.rows();let(a,b)=self.list.centered_scroll_window(n.max(1),rows.len());(rows[a..b].to_vec(),self.list.selected-a,a>0,b<rows.len())}
    fn slint_pointer_pick(&mut self,x:f32,_:f32){if (0. ..8.).contains(&x){self.selected=x as usize;}else if (8. ..16.).contains(&x){let mut p=self.p.patch.lock().unwrap();let c=&mut p.cables[x as usize-8];c.enabled=!c.enabled;if !c.enabled{if let Some(h)=&c.cv{h.set(0.);}}p.revision+=1;}}
    fn slint_extra(&mut self)->SlintExtra {let p=self.p.patch.lock().unwrap();SlintExtra::Portal(PortalExtra{sources:p.cables.iter().map(|c|self.source_name(c.source)).collect(),targets:p.cables.iter().map(|c|self.target_name(c.target)).collect(),amounts:p.cables.iter().map(|c|c.amount).collect(),enabled:p.cables.iter().map(|c|c.enabled&&c.source!=0&&c.target!=0).collect(),levels:self.p.meters.iter().map(|v|v.get()).collect(),selected:self.selected as i32,active:self.p.active.load(Ordering::Relaxed),status:self.status.clone()})}
    fn audio_processor(&mut self)->Option<Box<dyn AudioProcessor>> {if self.taken{return None;}self.taken=true;Some(Box::new(PortalProcessor{bus:self.bus.clone(),p:self.p.clone(),patch:self.p.patch.lock().unwrap().clone(),inputs:std::array::from_fn(|_|Vec::with_capacity(4096)),outputs:std::array::from_fn(|_|Vec::with_capacity(4096)),phase:[0.;2],smooth:[0.;N],held:[0.;N],random:0.,rng:0x547aca,previous_active:true}))}
}
impl Drop for PortalApp {fn drop(&mut self){self.p.active.store(false,Ordering::Relaxed);for c in &self.p.patch.lock().unwrap().cables{if let Some(h)=&c.cv{h.set(0.);}}}}
struct PortalProcessor {bus:Arc<AudioBus>,p:Arc<Shared>,patch:Patch,inputs:[Vec<f32>;N],outputs:[Vec<f32>;5],phase:[f32;2],smooth:[f32;N],held:[f32;N],random:f32,rng:u32,previous_active:bool}
impl AudioProcessor for PortalProcessor {
    fn process(&mut self,out:&mut[f32],channels:usize,rate:f32){
        out.fill(0.);if channels==0||!rate.is_finite()||rate<=0.{return;}
        if let Ok(p)=self.p.patch.try_lock(){if p.revision!=self.patch.revision{self.patch=p.clone();}}
        let active=self.p.active.load(Ordering::Relaxed);let frames=out.len()/channels;
        for b in &mut self.outputs{b.clear();b.resize(frames,0.);}
        for (i,c) in self.patch.cables.iter().enumerate(){self.inputs[i].clear();if active&&c.enabled&&c.source>=4&&c.target!=0 {if let Some(input)=self.bus.get(c.source-4){if let Ok(b)=input.try_lock(){self.inputs[i].extend_from_slice(&b);}}}self.p.meters[i].set(0.);}
        let rates=std::array::from_fn::<_,2,_>(|i|(self.p.rates[i].get()*2f32.powf(self.p.rate_cv[i].get().clamp(-2.,2.)*4.)).clamp(0.001,100.));
        let mut peaks=[0f32;N];
        if !active {self.smooth.fill(0.);for c in &self.patch.cables{if let Some(h)=&c.cv{h.set(0.);}}}
        if active {for n in 0..frames {
            let old=self.phase[0];for i in 0..2{self.phase[i]=(self.phase[i]+rates[i]/rate).fract();}let clock=self.phase[0]<old;
            if clock {self.rng^=self.rng<<13;self.rng^=self.rng>>17;self.rng^=self.rng<<5;self.random=self.rng as f32/u32::MAX as f32*2.-1.;}
            for (i,c) in self.patch.cables.iter().enumerate(){
                if !c.enabled||c.source==0||c.target==0 {self.smooth[i]=0.;if let Some(h)=&c.cv{h.set(0.);}continue;}
                let signal=match c.source {1=>(self.phase[0]*TAU).sin(),2=>1.-4.*(self.phase[1]-0.5).abs(),3=>self.random,_=>self.inputs[i].get(n).copied().unwrap_or(0.)};
                let signal=if signal.is_finite(){signal.clamp(-4.,4.)}else{0.};
                if clock {self.held[i]=signal;}
                let signal=match c.mode {1=>signal.abs(),2=>if signal>0.05{1.}else{0.},3=>self.held[i],_=>signal};
                let desired=(signal*c.amount+c.offset).clamp(-2.,2.);
                let alpha=if c.slew<=0.{1.}else{(1./(c.slew*0.001*rate)).min(1.)};self.smooth[i]+=alpha*(desired-self.smooth[i]);
                peaks[i]=peaks[i].max(self.smooth[i].abs());
                if c.target<=5 {self.outputs[c.target-1][n]+=self.smooth[i];}
            }
        }}
        for (i,c) in self.patch.cables.iter().enumerate(){if let Some(h)=&c.cv{h.set(if active&&c.enabled{self.smooth[i]}else{0.});}self.p.meters[i].set(peaks[i]);}
        for (i,b) in self.outputs.iter_mut().enumerate(){for x in b.iter_mut(){*x=x.clamp(-1.,1.);}if let Ok(mut bus)=self.p.outputs[i].try_lock(){bus.clear();bus.extend_from_slice(b);}}
        if active&&self.p.monitor.load(Ordering::Relaxed){let gain=(self.p.mix.get()+self.p.ext.get()).clamp(0.,1.);for (i,frame) in out.chunks_mut(channels).enumerate(){frame.fill(self.outputs[0].get(i).copied().unwrap_or(0.)*gain);}}
        self.previous_active=active;
    }
}
#[cfg(test)] mod tests {
    use super::*;
    fn fixture()->(PortalApp,Arc<AtomicF32>,Arc<Mutex<Vec<f32>>>) {let bus=Arc::new(AudioBus::new());let input=bus.register("input");let mods=Arc::new(ModBus::new());let target=mods.register("filter");(PortalApp::new(bus,mods,Arc::new(MixerBus::new()),Arc::new(AtomicF32::new(3.))),target,input)}
    fn set(app:&mut PortalApp,row:usize,delta:i32){app.list.selected=row;app.edit(delta);}
    #[test] fn multiple_cables_sum_modulation_and_bypass_disconnects() {
        let(mut app,target,input)=fixture();*input.lock().unwrap()=vec![0.4;512];
        for cable in 0..2 {app.selected=cable;for _ in 0..4{set(&mut app,1,1);}for _ in 0..6{set(&mut app,2,1);}set(&mut app,6,-1);}
        let mut dsp=app.audio_processor().unwrap();let mut out=[0.;1024];dsp.process(&mut out,2,48000.);assert!((target.get()-0.4).abs()<0.001);assert!(out.iter().all(|x|*x==0.));
        app.toggle_running();dsp.process(&mut out,2,48000.);assert_eq!(target.get(),0.);
    }
    #[test] fn audio_sends_and_parameter_routes_remain_independent() {
        let(mut app,target,input)=fixture();*input.lock().unwrap()=vec![0.4;512];for _ in 0..4{set(&mut app,1,1);}set(&mut app,2,1);set(&mut app,6,-1);
        let mut dsp=app.audio_processor().unwrap();let mut out=[0.;1024];dsp.process(&mut out,2,48000.);assert!((app.p.outputs[0].lock().unwrap()[10]-0.2).abs()<0.001);assert_eq!(target.get(),0.);
        app.list.selected=12;app.tick(&Input{knob1_press:true,..Default::default()});dsp.process(&mut out,2,48000.);assert!(app.p.outputs[0].lock().unwrap().iter().all(|x|*x==0.));
    }
}
