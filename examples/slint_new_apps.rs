//! Interactive design studies, not installed audio apps.
//! cargo run --example slint_new_apps --offline
//! cargo run --example slint_new_apps --offline -- --render DIRECTORY
use std::{cell::RefCell, rc::Rc};
use slint::{ComponentHandle, ModelRc, VecModel, SharedString};
#[path = "slint_common/new_app_catalog.rs"] mod catalog;
use catalog::CONCEPTS;
slint::slint! { import { NewAppScreen } from "slint_common/new_app_screens.slint"; }
struct State { current: usize, selected: usize, values: Vec<[f32;6]>, enabled: Vec<[bool;16]>, generation: Vec<i32>, phase: f32, playing: bool, detail: bool }
impl State {
    fn new() -> Self { Self { current: 0, selected: 0, values: vec![[0.45,0.6,0.35,0.5,0.25,0.7]; CONCEPTS.len()], enabled: vec![std::array::from_fn(|i| [0,4,7,11].contains(&i));CONCEPTS.len()], generation: vec![0;CONCEPTS.len()], phase: 1.4, playing: false, detail: false } }
    fn nav(&mut self, direction:i32) {
        match direction { 0 => self.selected=(self.selected+5)%6, 2=>self.selected=(self.selected+1)%6,
            1|3 => { let value=&mut self.values[self.current][self.selected]; *value=(*value+if direction==1 {0.05} else {-0.05}).clamp(0.,1.); },
            4=>self.detail=!self.detail, _=>{} }
    }
    fn function(&mut self, index:i32) {
        match index {0=>{self.current=(self.current+CONCEPTS.len()-1)%CONCEPTS.len();self.selected=0;},
            3=>{self.current=(self.current+1)%CONCEPTS.len();self.selected=0;},
            1=>{self.generation[self.current]+=1;self.phase+=0.37;}, 2=>self.playing=!self.playing,_=>{} }
    }
    fn sync(&self, ui:&NewAppScreen) {
        let c=&CONCEPTS[self.current]; let values=&self.values[self.current];
        let color=|n:u32|slint::Color::from_rgb_u8((n>>16)as u8,(n>>8)as u8,n as u8);
        ui.set_app_count(CONCEPTS.len() as i32);ui.set_name(c.name.into());ui.set_kind(c.kind);ui.set_category(c.category.into());ui.set_headline(c.title.into());ui.set_subtitle(c.subtitle.into());ui.set_hint(c.hint.into());
        ui.set_paper(color(c.bg));ui.set_ink(color(c.ink));ui.set_highlight(color(c.accent));ui.set_secondary(color(c.secondary));
        ui.set_labels(model(c.labels.iter().map(|s|SharedString::from(*s)).collect()));
        ui.set_values(model(c.labels.iter().zip(values).map(|(label,v)|value_label(label,*v).into()).collect()));
        ui.set_controls(model(values.to_vec()));ui.set_enabled(model(self.enabled[self.current].to_vec()));
        ui.set_selected(self.selected as i32);ui.set_generation(self.generation[self.current]);ui.set_phase(self.phase);ui.set_playing(self.playing);ui.set_detail(self.detail);ui.set_app_index(self.current as i32);
        ui.set_pad_lights(model(self.enabled[self.current].iter().map(|&on| if on {color(c.accent)} else {color(0x232323)}).collect()));
    }
}
fn model<T:Clone+'static>(values:Vec<T>)->ModelRc<T> { Rc::new(VecModel::from(values)).into() }
fn value_label(label:&str,v:f32)->String { match label {
    "Tempo"=>format!("{} BPM",(60.+120.*v).round()), "Voices"=>format!("{}",(8.+56.*v).round()),
    "Track"|"Head offset"|"Station"|"Album"|"Entry"|"Reference"=>format!("{:02}",(1.+7.*v).round()),
    "Pre-roll"=>format!("{} s",(30.*v).round()), "Pitch"=>format!("{:+} st",((v-0.5)*24.).round()),
    "Speed"|"Orbit speed"=>format!("{:.2}×",0.5+v*1.5), "Root"=>["C","C♯","D","E♭","E","F","F♯","G","A♭","A","B♭","B"][(v*11.).round()as usize].into(),
    _=>format!("{}%",(v*100.).round()) } }
fn connect(ui:&NewAppScreen,state:Rc<RefCell<State>>) {
    let weak=ui.as_weak(); let s=state.clone(); ui.on_navigate(move|d|{let mut s=s.borrow_mut();s.nav(d);if let Some(u)=weak.upgrade(){s.sync(&u);}});
    let weak=ui.as_weak(); let s=state.clone(); ui.on_function(move|i|{let mut s=s.borrow_mut();s.function(i);if let Some(u)=weak.upgrade(){s.sync(&u);}});
    let weak=ui.as_weak(); let s=state.clone(); ui.on_choose(move|i|{let mut s=s.borrow_mut();s.selected=(i as usize).min(5);if let Some(u)=weak.upgrade(){s.sync(&u);}});
    let weak=ui.as_weak(); let s=state.clone(); ui.on_xy(move|x,y|{let mut s=s.borrow_mut();let c=s.current;let pair=if c==4 {(2,3)}else{(0,1)};s.values[c][pair.0]=x.clamp(0.,1.);s.values[c][pair.1]=y.clamp(0.,1.);if let Some(u)=weak.upgrade(){s.sync(&u);}});
    let weak=ui.as_weak(); ui.on_pad(move|i,down|{if down && (0..16).contains(&i) {let mut s=state.borrow_mut();let c=s.current;s.enabled[c][i as usize]=!s.enabled[c][i as usize];if let Some(u)=weak.upgrade(){s.sync(&u);}}});
}
fn main(){
    let args:Vec<String>=std::env::args().collect();
    if args.get(1).map(String::as_str)==Some("--render"){render(args.get(2).expect("output directory"));return;}
    let ui=NewAppScreen::new().unwrap();let state=Rc::new(RefCell::new(State::new()));connect(&ui,state.clone());state.borrow().sync(&ui);
    let timer=slint::Timer::default();let weak=ui.as_weak();timer.start(slint::TimerMode::Repeated,std::time::Duration::from_millis(33),move||{
        let Some(ui)=weak.upgrade()else{return};let mut s=state.borrow_mut();
        if ui.get_live_stick_y().abs()>0.35 || ui.get_live_stick_x().abs()>0.35 { let c=s.current;s.values[c][0]=(s.values[c][0]+ui.get_live_stick_x()*0.015).clamp(0.,1.);s.values[c][1]=(s.values[c][1]+ui.get_live_stick_y()*0.015).clamp(0.,1.);s.sync(&ui); }
        if s.playing {s.phase+=0.033;ui.set_phase(s.phase);}
    });ui.run().unwrap();
}
fn render(directory:&str){
    use slint::platform::{Platform,PlatformError,WindowAdapter,software_renderer::{MinimalSoftwareWindow,RepaintBufferType},WindowEvent,PointerEventButton};
    struct Headless(Rc<MinimalSoftwareWindow>);impl Platform for Headless {fn create_window_adapter(&self)->Result<Rc<dyn WindowAdapter>,PlatformError>{Ok(self.0.clone())}}
    let window=MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();window.set_size(slint::PhysicalSize::new(1240,560));
    std::fs::create_dir_all(directory).unwrap();let ui=NewAppScreen::new().unwrap();let state=Rc::new(RefCell::new(State::new()));connect(&ui,state.clone());ui.show().unwrap();
    let click=|x,y|{let position=slint::LogicalPosition::new(x,y);ui.window().dispatch_event(WindowEvent::PointerPressed{position,button:PointerEventButton::Left});ui.window().dispatch_event(WindowEvent::PointerReleased{position,button:PointerEventButton::Left});};
    for (index,c) in CONCEPTS.iter().enumerate(){
        {let mut s=state.borrow_mut();s.current=index;s.selected=0;s.sync(&ui);}
        click(113.,249.);assert_eq!(state.borrow().selected,1,"{} navigation",c.name);
        let old=state.borrow().values[index][1];click(157.,205.);assert!(state.borrow().values[index][1]>old,"{} edit",c.name);
        let old=state.borrow().enabled[index][0];click(913.,149.);assert_ne!(state.borrow().enabled[index][0],old,"{} pad",c.name);
        let before=state.borrow().playing;ui.invoke_function(2);assert_ne!(state.borrow().playing,before);ui.invoke_function(2);
        let before=state.borrow().generation[index];ui.invoke_function(1);assert_eq!(state.borrow().generation[index],before+1);
        state.borrow_mut().selected=0;state.borrow().sync(&ui);window.request_redraw();
        let mut pixels=slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1240,560);window.draw_if_needed(|r|{r.render(pixels.make_mut_slice(),1240);});
        let slug=c.name.to_lowercase().replace(' ',"-");
        for full in [false,true]{let (w,h,bytes)=if full{(1240,560,pixels.as_bytes().to_vec())}else{let mut data=Vec::new();for y in 92..452{let start=(y*1240+208)*3;data.extend_from_slice(&pixels.as_bytes()[start..start+640*3]);}(640,360,data)};
            let path=std::path::Path::new(directory).join(format!("{slug}{}.png",if full{"-console"}else{""}));let mut encoder=png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()),w,h);encoder.set_color(png::ColorType::Rgb);encoder.set_depth(png::BitDepth::Eight);encoder.write_header().unwrap().write_image_data(&bytes).unwrap();}
        println!("{}: rendered; navigation, value editing, pads, motion and variation passed",c.name);
    }
    // Switching concepts preserves their independent parameter and pad states.
    state.borrow_mut().current=0;let saved=state.borrow().values[0];ui.invoke_function(3);ui.invoke_function(0);assert_eq!(state.borrow().values[0],saved);
}
