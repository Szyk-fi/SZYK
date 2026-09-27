//! File decoding runs on a loader worker, never in an audio callback.
use super::{Clip,MEDIA_LIMIT};
use std::{path::Path,sync::Arc};
use symphonia::core::{audio::SampleBuffer,codecs::DecoderOptions,errors::Error,formats::FormatOptions,io::MediaSourceStream,meta::MetadataOptions,probe::Hint};
pub const EXTENSIONS:&[&str]=&["wav","wave","flac","mp3","ogg","oga","m4a","mp4","aac","aif","aiff","aifc"];
pub fn supported(path:&Path)->bool{path.extension().and_then(|s|s.to_str()).is_some_and(|s|EXTENSIONS.iter().any(|e|s.eq_ignore_ascii_case(e)))}
pub fn load(path:&Path)->Result<Clip,String>{
    let file=std::fs::File::open(path).map_err(|e|format!("Open audio: {e}"))?;
    let mut hint=Hint::new();if let Some(ext)=path.extension().and_then(|s|s.to_str()){hint.with_extension(&ext.to_ascii_lowercase());}
    let probed=symphonia::default::get_probe().format(&hint,MediaSourceStream::new(Box::new(file),Default::default()),&FormatOptions{enable_gapless:true,..Default::default()},&MetadataOptions::default()).map_err(|e|format!("Unsupported or damaged audio: {e}"))?;
    let mut format=probed.format;let track=format.default_track().ok_or("No audio track")?;
    if track.codec_params.n_frames.is_some_and(|n|n>MEDIA_LIMIT as u64){return Err("Audio exceeds 5.76M decoded frames (120s at 48k); use a shorter excerpt".into());}
    let id=track.id;let mut decoder=symphonia::default::get_codecs().make(&track.codec_params,&DecoderOptions::default()).map_err(|e|format!("Unsupported codec: {e}"))?;
    let mut samples=Vec::new();let mut rate=0u32;let mut peak=0f32;
    loop {
        let packet=match format.next_packet(){Ok(p)=>p,Err(Error::IoError(e)) if e.kind()==std::io::ErrorKind::UnexpectedEof=>break,Err(e)=>return Err(format!("Read audio: {e}"))};
        if packet.track_id()!=id{continue;}
        let decoded=decoder.decode(&packet).map_err(|e|format!("Decode audio: {e}"))?;let spec=*decoded.spec();let channels=spec.channels.count();
        if channels==0||channels>2||spec.rate==0{return Err("Use mono or stereo audio".into());}
        if rate!=0&&rate!=spec.rate{return Err("Changing sample rates within one file are unsupported".into());}rate=spec.rate;
        let mut buffer=SampleBuffer::<f32>::new(decoded.capacity() as u64,spec);buffer.copy_interleaved_ref(decoded);
        if samples.len()+buffer.samples().len()/channels>MEDIA_LIMIT{return Err("Audio exceeds bounded decoded-frame limit".into());}
        for frame in buffer.samples().chunks_exact(channels){let pair=[frame[0],*frame.get(1).unwrap_or(&frame[0])].map(|x|if x.is_finite(){x.clamp(-1.,1.)}else{0.});peak=peak.max(pair[0].abs()).max(pair[1].abs());samples.push(pair);}
    }
    if samples.is_empty(){return Err("Audio file contains no decodable samples".into());}
    Ok(Clip{samples:Arc::new(samples),rate:rate as f32,name:path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),peak})
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn extension_filter_accepts_real_audio_families_case_insensitively(){for ext in EXTENSIONS{assert!(supported(Path::new(&format!("file.{}",ext.to_uppercase()))));}assert!(!supported(Path::new("cover.png")));}
    #[test] fn real_flac_and_aiff_decode_with_duration_rate_and_stereo() {
        for name in ["stereo.flac","stereo.aiff"] {
            let clip=load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio").join(name)).unwrap_or_else(|e|panic!("{name}: {e}"));
            assert_eq!(clip.rate,44100.);assert_eq!(clip.samples.len(),4410);assert!(clip.peak>0.3);
            assert!((clip.samples[100][0]-clip.samples[100][1]).abs()>0.01);
        }
    }
    #[test] fn shared_decoder_preserves_stereo_pcm(){
        let path=std::env::temp_dir().join(format!("portamax-decoder-{}.wav",std::process::id()));let mut w=hound::WavWriter::create(&path,hound::WavSpec{channels:2,sample_rate:44100,bits_per_sample:16,sample_format:hound::SampleFormat::Int}).unwrap();
        for i in 0..512i16{w.write_sample(i*16).unwrap();w.write_sample(-i*8).unwrap();}w.finalize().unwrap();let clip=load(&path).unwrap();assert_eq!(clip.rate,44100.);assert_eq!(clip.samples.len(),512);assert!(clip.samples[100][0]>0.&&clip.samples[100][1]<0.);std::fs::remove_file(path).unwrap();
    }
}
