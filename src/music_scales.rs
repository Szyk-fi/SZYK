//! Shared musical facts, independent of any installed instrument.
//! Families follow https://pulse.berklee.edu/scales/index.html.
//! Keep the original first eight IDs stable for existing app settings.
pub const ROOT_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
pub const SCALE_TYPES: [(&str, &[i32]); 16] = [
    ("Chromatic", &[0,1,2,3,4,5,6,7,8,9,10,11]),
    ("Major / Ionian", &[0,2,4,5,7,9,11]),
    ("Minor / Aeolian", &[0,2,3,5,7,8,10]),
    ("Dorian", &[0,2,3,5,7,9,10]),
    ("Mixolydian", &[0,2,4,5,7,9,10]),
    ("Harmonic minor", &[0,2,3,5,7,8,11]),
    ("Major pentatonic", &[0,2,4,7,9]),
    ("Minor pentatonic", &[0,3,5,7,10]),
    ("Phrygian", &[0,1,3,5,7,8,10]),
    ("Lydian", &[0,2,4,6,7,9,11]),
    ("Locrian", &[0,1,3,5,6,8,10]),
    ("Major blues", &[0,2,3,4,7,9]),
    ("Minor blues", &[0,3,5,6,7,10]),
    ("Whole tone", &[0,2,4,6,8,10]),
    ("Diminished W-H", &[0,2,3,5,6,8,9,11]),
    ("Diminished H-W", &[0,1,3,4,6,7,9,10]),
];
pub fn intervals(index: usize) -> &'static [i32] { SCALE_TYPES[index % SCALE_TYPES.len()].1 }
pub fn degree(index: usize, position: usize) -> i32 {
    let notes=intervals(index); notes[position % notes.len()] + 12*(position/notes.len()) as i32
}
#[derive(Clone, Debug)]
pub struct ScaleInfo { pub name: String, pub root: String, pub notes: String, pub keys: Vec<bool> }
impl ScaleInfo {
    pub fn new(index: usize, root: i32) -> Self {
        let (name,notes)=SCALE_TYPES[index % SCALE_TYPES.len()];
        Self::from_intervals(name,root,notes)
    }
    pub fn from_intervals(name: &str, root: i32, intervals: &[i32]) -> Self {
        let root=root.rem_euclid(12) as usize;
        let letters=["C","D","E","F","G","A","B"];
        let natural=[0i32,2,4,5,7,9,11];
        let first=[0,0,1,1,2,3,3,4,4,5,5,6][root];
        let mut keys=vec![false;12];
        let names:Vec<_>=intervals.iter().enumerate().map(|(i,n)| {
            let pitch=(root as i32+n).rem_euclid(12) as usize; keys[pitch]=true;
            if intervals.len()==7 && name!="Custom" {
                let letter=(first+i)%7;
                let accidental=(pitch as i32-natural[letter]+6).rem_euclid(12)-6;
                format!("{}{}",letters[letter],if accidental>=0 {"#".repeat(accidental as usize)} else {"b".repeat((-accidental) as usize)})
            } else { ROOT_NAMES[pitch].to_string() }
        }).collect();
        Self {name:name.into(),root:ROOT_NAMES[root].into(),notes:names.join(" · "),keys}
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn all_families_transpose_and_wrap_for_every_root() {
        for (s,(_,notes)) in SCALE_TYPES.iter().enumerate() {
            assert_eq!(notes[0],0); assert!(notes.windows(2).all(|w|w[0]<w[1]));
            for root in 0..12 {let info=ScaleInfo::new(s,root);assert_eq!(info.keys.iter().filter(|b|**b).count(),notes.len());for p in 0..48 {assert!(info.keys[((root+degree(s,p))%12) as usize]);}}
            assert_eq!(degree(s,notes.len()),12);
        }
    }
    #[test] fn notes_use_diatonic_spelling() {
        assert_eq!(ScaleInfo::new(1,5).notes,"F · G · A · Bb · C · D · E");
        assert_eq!(ScaleInfo::new(5,9).notes,"A · B · C · D · E · F · G#");
        assert_eq!(intervals(14),[0,2,3,5,6,8,9,11]);
        assert_eq!(intervals(15),[0,1,3,4,6,7,9,10]);
    }
}
