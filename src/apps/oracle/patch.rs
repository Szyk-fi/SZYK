//! The Oracle patch format: plain JSON the AI writes, people can edit and
//! share, and the engine compiles. See `llm::system_prompt` for the
//! AI-facing description and `starters/` for examples.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MAX_PARAMS: usize = 64;
pub const MAX_MACROS: usize = 8;
pub const MAX_NODES: usize = 32;
pub const MAX_VOICES: usize = 16;
pub const PAGES: usize = 4;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Instrument,
    Effect,
    Generator,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Instrument => "instrument",
            Kind::Effect => "effect",
            Kind::Generator => "generator",
        }
    }
}

/// An expression written either as a string or a bare number.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum ExprSrc {
    Num(f64),
    Text(String),
}

impl ExprSrc {
    pub fn text(&self) -> String {
        match self {
            ExprSrc::Num(n) => format!("{n}"),
            ExprSrc::Text(s) => s.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum OutSpec {
    Mono(ExprSrc),
    Stereo { left: ExprSrc, right: ExprSrc },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub min: f32,
    #[serde(default = "one")]
    pub max: f32,
    #[serde(default)]
    pub default: f32,
    /// lin | exp | int | toggle | choice
    #[serde(default = "lin", skip_serializing_if = "is_lin")]
    pub curve: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// 1..4; 0 = assign automatically.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub page: u8,
    /// Names for `choice` params (value = index).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MacroTarget {
    pub param: String,
    /// -1..1 in the target's normalized range, at macro = 1.
    pub amount: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MacroSpec {
    pub name: String,
    #[serde(default)]
    pub targets: Vec<MacroTarget>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NodeSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "in", skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, ExprSrc>,
    /// Block options (`wave`, `mode`, `values`, `max`, ...).
    #[serde(flatten)]
    pub opts: BTreeMap<String, Value>,
}

/// Saved control positions, in parameter units.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct State {
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
    #[serde(default)]
    pub macros: Vec<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Patch {
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "eight")]
    pub voices: u32,
    #[serde(default = "yes")]
    pub amp_env: bool,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    pub macros: Vec<MacroSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice: Vec<NodeSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global: Vec<NodeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_out: Option<ExprSrc>,
    pub out: OutSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
}

fn one() -> f32 {
    1.0
}
fn eight() -> u32 {
    8
}
fn yes() -> bool {
    true
}
fn lin() -> String {
    "lin".into()
}
fn is_lin(s: &String) -> bool {
    s == "lin"
}
fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

impl Patch {
    /// Parse the first JSON object found in `text` (tolerates code fences
    /// and chatter around it, which language models add).
    pub fn from_llm_text(text: &str) -> Result<Patch, String> {
        let json = extract_json(text).ok_or("no JSON object found in the reply")?;
        serde_json::from_str::<Patch>(json).map_err(|e| format!("JSON does not match the patch format: {e}"))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Params in page order, pages auto-assigned (16 per page) when unset.
    pub fn page_of(&self, idx: usize) -> usize {
        match self.params.get(idx).map(|p| p.page) {
            Some(p) if (1..=PAGES as u8).contains(&p) => p as usize - 1,
            _ => (idx / 16).min(PAGES - 1),
        }
    }
}

/// Find the outermost `{...}` in text, skipping strings.
pub fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------- param maps

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Lin,
    Exp,
    Int,
    Toggle,
    Choice(usize),
}

#[derive(Clone, Debug)]
pub struct ParamMap {
    pub min: f32,
    pub max: f32,
    pub curve: Curve,
}

impl ParamMap {
    pub fn from_spec(p: &ParamSpec) -> Self {
        let curve = match p.curve.as_str() {
            "exp" if p.min > 0.0 && p.max > 0.0 => Curve::Exp,
            "int" => Curve::Int,
            "toggle" | "bool" => Curve::Toggle,
            "choice" if !p.options.is_empty() => Curve::Choice(p.options.len()),
            _ => Curve::Lin,
        };
        Self { min: p.min, max: p.max, curve }
    }

    pub fn value(&self, norm: f32) -> f32 {
        let n = norm.clamp(0.0, 1.0);
        match self.curve {
            Curve::Lin => self.min + (self.max - self.min) * n,
            Curve::Exp => self.min * (self.max / self.min).powf(n),
            Curve::Int => (self.min + (self.max - self.min) * n).round(),
            Curve::Toggle => {
                if n >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Curve::Choice(k) => (n * (k.saturating_sub(1)) as f32).round(),
        }
    }

    pub fn norm(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        let n = match self.curve {
            Curve::Exp => (value.max(1e-9) / self.min).ln() / (self.max / self.min).ln(),
            Curve::Toggle => {
                if value >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Curve::Choice(k) => {
                if k <= 1 {
                    0.0
                } else {
                    value / (k - 1) as f32
                }
            }
            _ => {
                if span.abs() < 1e-12 {
                    0.0
                } else {
                    (value - self.min) / span
                }
            }
        };
        if n.is_finite() {
            n.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// How many discrete positions a knob should step through, if discrete.
    pub fn steps(&self) -> Option<usize> {
        match self.curve {
            Curve::Int => Some(((self.max - self.min).abs().round() as usize + 1).max(2)),
            Curve::Toggle => Some(2),
            Curve::Choice(k) => Some(k.max(1)),
            _ => None,
        }
    }
}

pub fn format_value(spec: &ParamSpec, map: &ParamMap, v: f32) -> String {
    match map.curve {
        Curve::Toggle => return if v >= 0.5 { "on".into() } else { "off".into() },
        Curve::Choice(_) => return spec.options.get(v as usize).cloned().unwrap_or_else(|| format!("{v}")),
        Curve::Int => return format!("{v:.0}{}", unit_suffix(&spec.unit)),
        _ => {}
    }
    let a = v.abs();
    let num = if spec.unit == "Hz" && a >= 1000.0 {
        return format!("{:.2} kHz", v / 1000.0);
    } else if spec.unit == "s" && a < 1.0 {
        return format!("{:.0} ms", v * 1000.0);
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    format!("{num}{}", unit_suffix(&spec.unit))
}

fn unit_suffix(u: &str) -> String {
    if u.is_empty() {
        String::new()
    } else if u == "%" {
        "%".into()
    } else {
        format!(" {u}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_llm_reply_with_fences_and_chatter() {
        let reply = "Here you go!\n```json\n{\"name\":\"T\",\"out\":\"voices\",\"voice_out\":\"0\"}\n```\nEnjoy {not json}";
        let p = Patch::from_llm_text(reply).unwrap();
        assert_eq!(p.name, "T");
        assert_eq!(p.voices, 8);
        assert!(p.amp_env);
    }

    #[test]
    fn stereo_out_and_numeric_expressions_parse() {
        let p: Patch = serde_json::from_str(
            r#"{"name":"S","kind":"effect","out":{"left":"in","right":0.5},
                "global":[{"id":"d","type":"delay","max":2,"in":{"in":"in","time":0.25}}]}"#,
        )
        .unwrap();
        assert!(matches!(p.out, OutSpec::Stereo { .. }));
        assert_eq!(p.global[0].inputs["time"], ExprSrc::Num(0.25));
        assert_eq!(p.global[0].opts["max"], serde_json::json!(2));
    }

    #[test]
    fn param_maps_round_trip() {
        let spec = |curve: &str, min, max| ParamSpec {
            id: "x".into(),
            name: "X".into(),
            min,
            max,
            default: min,
            curve: curve.into(),
            unit: String::new(),
            page: 0,
            options: vec!["a".into(), "b".into(), "c".into()],
        };
        for (curve, min, max, val) in [("lin", -1.0, 1.0, 0.25), ("exp", 20.0, 20000.0, 440.0), ("int", 0.0, 7.0, 3.0), ("choice", 0.0, 2.0, 2.0)] {
            let m = ParamMap::from_spec(&spec(curve, min, max));
            let back = m.value(m.norm(val));
            assert!((back - val).abs() < val.abs() * 1e-3 + 1e-3, "{curve}: {val} -> {back}");
        }
    }
}
