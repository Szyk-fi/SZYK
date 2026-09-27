//! Deterministic, bounded audio analysis. This is DSP, not a neural classifier.
use serde::{Serialize,Deserialize};
use std::sync::Arc;
pub const RATE:f32=24000.;
pub const MAX_FRAMES:usize=192000;
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Chunk {pub start:usize,pub end:usize,pub root:f32,pub confidence:f32,pub brightness:f32,pub class:String}
#[derive(Clone)]
pub struct Instrument {pub samples:Arc<Vec<f32>>,pub chunks:Vec<Chunk>,pub name:String,pub trimmed:bool}
pub fn resample(input:&[[f32;2]],rate:f32)->Vec<f32>{
 let n=((input.len() as f32*RATE/rate) as usize).min(MAX_FRAMES);(0..n).map(|i|{let at=i as f32*rate/RATE;let j=at as usize;let a=input.get(j).copied().unwrap_or([0.;2]);let b=input.get(j+1).copied().unwrap_or(a);let t=at.fract();(((a[0]+a[1])*(1.-t)+(b[0]+b[1])*t)*0.5).clamp(-1.,1.)}).collect()
}
fn root_pitch(samples:&[f32])->(f32,f32){
 // Decimate to 8 kHz after a three-sample box low-pass; correlate the energetic window.
 let peak_at=samples.chunks(1024).enumerate().max_by(|a,b|energy(a.1).total_cmp(&energy(b.1))).map(|(i,_)|i*1024).unwrap_or(0);
 let start=peak_at.min(samples.len().saturating_sub(6144));
 let x:Vec<f32>=samples[start..samples.len().min(start+6144)].chunks_exact(3).map(|v|v.iter().sum::<f32>()/3.).collect();
 if x.len()<200{return(60.,0.);}
 let mut best=(0usize,0f32);let maxlag=160.min(x.len()/2);
 for lag in 8..=maxlag {let(mut xy,mut xx,mut yy)=(0.,0.,0.);for i in 0..x.len()-lag {xy+=x[i]*x[i+lag];xx+=x[i]*x[i];yy+=x[i+lag]*x[i+lag];}let c=xy/(xx*yy).sqrt().max(1e-12);if c>best.1{best=(lag,c);}}
 // Prefer the shortest similarly strong period to avoid octave-down multiples.
 if best.1>0.7 {for lag in 8..best.0 {let(mut xy,mut xx,mut yy)=(0.,0.,0.);for i in 0..x.len()-lag{xy+=x[i]*x[i+lag];xx+=x[i]*x[i];yy+=x[i+lag]*x[i+lag];}let c=xy/(xx*yy).sqrt().max(1e-12);if c>best.1-0.025 {let left=if lag>8{let mut p=0.;for i in 0..x.len()-lag{p+=x[i]*x[i+lag-1];}p}else{xy};let right=(0..x.len()-lag-1).map(|i|x[i]*x[i+lag+1]).sum::<f32>();if xy>=left&&xy>=right{best=(lag,c);break;}}}}
 if best.1<0.65{(60.,best.1.max(0.))}else{(69.+12.*(8000./best.0 as f32/440.).log2(),best.1.clamp(0.,1.))}
}
fn energy(s:&[f32])->f32{s.iter().map(|x|x*x).sum::<f32>()/s.len().max(1) as f32}
pub fn analyze(mut samples:Vec<f32>,name:String,sensitivity:f32,trimmed:bool)->Result<Instrument,String>{
 samples.truncate(MAX_FRAMES);for x in &mut samples{*x=if x.is_finite(){x.clamp(-1.,1.)}else{0.};}
 if samples.len()<1200{return Err("Use at least 50 ms of sound".into());}
 let frames:Vec<f32>=samples.chunks(240).map(|v|energy(v).sqrt()).collect();let peak=frames.iter().copied().fold(0.,f32::max);
 if peak<0.0001{return Err("No usable sound; choose a source or louder clip".into());}
 let floor=(peak*0.025).max(0.0001);let threshold=1.4+(1.-sensitivity.clamp(0.,1.))*3.;
 let mut starts=Vec::new();let mut quiet=true;let mut previous=0f32;
 for (i,&rms) in frames.iter().enumerate(){if rms>floor && (quiet||rms>previous.max(floor)*threshold) && starts.last().is_none_or(|last|i*240-last>=1920) {if starts.len()<16{starts.push(i*240);}}quiet=rms<floor;previous=previous*0.5+rms*0.5;}
 if starts.is_empty(){starts.push(0);}
 let mut chunks=Vec::new();
 for (i,&start) in starts.iter().enumerate(){let mut end=starts.get(i+1).copied().unwrap_or(samples.len());while end>start+1200 && energy(&samples[end.saturating_sub(240)..end]).sqrt()<floor {end-=240;}
  if end<=start+240{continue;}let signal=&samples[start..end];let(root,confidence)=root_pitch(signal);
  let zcr=signal.windows(2).filter(|v|v[0].is_sign_positive()!=v[1].is_sign_positive()).count() as f32/signal.len() as f32;
  let early=energy(&signal[..signal.len().min(1200)]);let late=energy(&signal[signal.len()/2..]);
  let class=if confidence>0.75{"Tonal"}else if early>late*2.5{"Percussive"}else if zcr>0.15{"Noisy"}else{"Texture"};
  chunks.push(Chunk{start,end,root,confidence,brightness:(zcr*4.).min(1.),class:class.into()});
 }
 if chunks.is_empty(){return Err("No playable chunks detected".into());}
 Ok(Instrument{samples:Arc::new(samples),chunks,name,trimmed})
}
pub fn validate(i:&Instrument)->Result<(),String>{if i.samples.is_empty()||i.samples.len()>MAX_FRAMES||i.chunks.is_empty()||i.chunks.len()>16{return Err("Invalid instrument size".into());}for c in &i.chunks{if c.start>=c.end||c.end>i.samples.len()||c.end-c.start<32||!c.root.is_finite()||!(0.0..=127.0).contains(&c.root)||!c.confidence.is_finite()||!c.brightness.is_finite(){return Err("Invalid chunk mapping".into());}}Ok(())}
#[cfg(test)]mod tests{use super::*;
 #[test]fn finds_separate_attacks_and_pitch(){let mut x=vec![0.;24000];for (a,b,hz) in [(1200,8400,220.),(12000,20400,440.)]{for n in a..b{x[n]=(std::f32::consts::TAU*hz*n as f32/RATE).sin()*0.5;}}let i=analyze(x,"test".into(),0.5,false).unwrap();assert_eq!(i.chunks.len(),2);assert!((i.chunks[0].root-57.).abs()<0.8,"{}",i.chunks[0].root);assert!((i.chunks[1].root-69.).abs()<0.8,"{}",i.chunks[1].root);validate(&i).unwrap();}
 #[test]fn silence_and_invalid_mappings_are_rejected(){assert!(analyze(vec![0.;24000],"silent".into(),0.5,false).is_err());assert!(analyze(vec![f32::NAN;24000],"invalid".into(),0.5,false).is_err());}
 #[test]fn segmentation_is_bounded(){let x=(0..MAX_FRAMES).map(|i|if i%2400<1200{(i as f32*0.2).sin()*0.4}else{0.}).collect();let i=analyze(x,"many".into(),1.,false).unwrap();assert!(i.chunks.len()<=16);}
}
