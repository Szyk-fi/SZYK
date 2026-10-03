//! Launcher metadata and navigation never construct an app or start DSP.
pub const CATEGORIES:[&str;8]=["All apps","Instruments","Effects","Sequencing","Library","Utilities","Kids","Recent"];
/// The Recent section, which lists the apps opened last rather than a category.
pub const RECENT:usize=7;
/// What the installed manifests say about each app (name -> section,
/// description), so a new app files itself in the right section with its
/// own description. The matches below are only the fallback for an app
/// whose manifest says nothing.
fn catalog()->&'static std::sync::Mutex<std::collections::HashMap<String,(usize,String)>>{static C:std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String,(usize,String)>>>=std::sync::OnceLock::new();C.get_or_init(Default::default)}
pub fn install_catalog(entries:impl Iterator<Item=(String,String,String)>){let mut c=catalog().lock().unwrap();for (name,cat,desc) in entries{let idx=match cat.as_str(){"instrument"=>1,"effect"=>2,"sequencer"=>3,"library"=>4,"kids"=>6,""=>fallback_category(&name),_=>5};c.insert(name,(idx,desc));}}
pub fn category(name:&str)->usize {catalog().lock().unwrap().get(name).map(|e|e.0).unwrap_or_else(||fallback_category(name))}
fn fallback_category(name:&str)->usize {match name {
    "Forge"|"Orbit"|"Swarm"|"Mutant"|"Constellation"|"Dream"|"Plaits"|"Synth"|"Cascade"|"Sample Drum"|"Queen of Pentacles"|"Tinkertone"|"Oracle"=>1,
    "Fracture"|"Ghosts"|"Tape Machine"|"Beads"|"Black Hole"|"Clouds"|"Magnito"|"Natural Gate"|"Nautilus"|"Prism"|"Rainmaker"|"Singularity"|"Starlab"|"Tonestack"|"Warps"|"Morph"|"Master"|"Vector Filter"=>2,
    "Bloom"|"Madness"|"Nebula"|"Sequencer"|"Turing Machine"|"Pam's Workout"|"Pams Workout"|"Voltage"|"Pulsar"=>3,
    "Reference"|"Vinyl"|"Practice"|"Radio"|"Memories"|"Field"|"Sample Hunter"|"Studio"=>4,
    _=>5,
}}
pub fn description(name:&str)->String {if let Some((_,d))=catalog().lock().unwrap().get(name){if !d.is_empty(){return d.clone();}}fallback_description(name).into()}
fn fallback_description(name:&str)->&'static str {match name {
    "Forge"=>"Turn recorded sounds into playable, editable instruments.",
    "Vector Filter"=>"Sculpt audio in three dimensions: cutoff, resonance and drive.",
    "Portal"=>"Patch audio and modulation between your devices.","Retro"=>"Your cartridge library. Four systems, one console.","Bloom"=>"Create evolving melodies with intersecting orbits.","Reference"|"Vinyl"=>"Listen, browse and compare your audio collection.","Studio"=>"Record, layer and mix eight independent takes.","Settings"=>"Audio, controllers and system preferences.","Controller"=>"Map a PS5, Xbox or any gamepad onto Portamax.","Mixer"=>"Balance every active instrument and effect.","Scope"|"Analyzer"=>"See and measure the signals in your session.","Sample Hunter"=>"Capture a sound and turn it into an instrument.","Constellation"=>"Explore scales, chord voicings and harmonic motion.","Mutant"=>"Breed timbres and play evolving harmonic instruments.",_=>match category(name){1=>"Play an instrument. Shape its voice and musical motion.",2=>"Transform a source with an independent audio processor.",3=>"Generate patterns, triggers and musical movement.",4=>"Record, explore and play your sound library.",_=>"A dedicated tool for your Portamax session."},}}
#[derive(Default)] pub struct Launcher {pub category:usize,pub recent:Vec<usize>}
impl Launcher {
    pub fn cycle(&mut self,delta:i32){self.category=(self.category as i32+delta).rem_euclid(CATEGORIES.len() as i32) as usize;}
    pub fn visit(&mut self,index:usize){self.recent.retain(|i|*i!=index);self.recent.insert(0,index);self.recent.truncate(12);}
    pub fn indices(&self,names:&[String])->Vec<usize>{if self.category==RECENT {self.recent.iter().copied().filter(|i|*i<names.len()).collect()}else{names.iter().enumerate().filter(|(_,n)|self.category==0||category(n)==self.category).map(|(i,_)|i).collect()}}
}
#[cfg(test)] mod tests {use super::*;
    #[test]fn filters_keep_real_indices_and_recent_order(){let names=vec!["Orbit".into(),"Portal".into(),"Vinyl".into()];let mut l=Launcher::default();l.category=4;assert_eq!(l.indices(&names),vec![2]);l.visit(2);l.visit(0);l.visit(2);l.category=RECENT;assert_eq!(l.indices(&names),vec![2,0]);l.cycle(1);assert_eq!(l.category,0);}
}
