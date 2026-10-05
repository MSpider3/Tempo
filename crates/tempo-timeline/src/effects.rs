//! Filters on clips.
//!
//! A filter is a short recipe of simple steps (saturation, brightness, contrast,
//! blur, or a colour matrix). Both the viewer and the exporter know how to carry
//! out the two things every recipe boils down to: a colour transform and a blur.
//! The recipe and its settings are stored with the clip, so a project still
//! looks right if the plugin that supplied the filter is no longer installed.

use serde::{Deserialize, Serialize};

/// A number in a recipe: fixed, or taken from one of the filter's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Amount {
    Value(f32),
    /// The id of a parameter, written `"$name"` in a manifest.
    Param(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectOp {
    /// 0 is black and white, 1 leaves the colours alone, above 1 is more vivid.
    Saturation(Amount),
    /// Added to every colour: -1 is black, 0 no change, 1 white.
    Brightness(Amount),
    /// 1 is no change; contrast is stretched around mid grey.
    Contrast(Amount),
    /// Blur radius in pixels of a 1080-line picture.
    Blur(Amount),
    /// Twenty numbers: four rows of `r g b a offset` giving the new r, g, b and a.
    Matrix(Vec<f32>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectParam {
    pub id: String,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub value: f32,
    /// What Reset returns the setting to.
    #[serde(default)]
    pub default: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipEffect {
    /// `plugin-id/filter-id`, to recognise where it came from.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub params: Vec<EffectParam>,
    pub ops: Vec<EffectOp>,
}

/// A colour transform: `new = matrix × old + offset`, rows in r, g, b, a order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorTransform {
    pub matrix: [[f32; 4]; 4],
    pub offset: [f32; 4],
}

impl ColorTransform {
    pub const IDENTITY: Self = Self {
        matrix: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
        offset: [0.0; 4],
    };

    /// The transform that does `self` first and `next` second.
    pub fn then(&self, next: &Self) -> Self {
        let mut out = Self { matrix: [[0.0; 4]; 4], offset: next.offset };
        for r in 0..4 {
            for c in 0..4 {
                out.matrix[r][c] = (0..4).map(|k| next.matrix[r][k] * self.matrix[k][c]).sum();
            }
            out.offset[r] += (0..4).map(|k| next.matrix[r][k] * self.offset[k]).sum::<f32>();
        }
        out
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

/// What a list of filters comes to: one colour transform and one blur radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedEffects {
    pub color: ColorTransform,
    /// In pixels of a 1080-line picture; 0 is no blur.
    pub blur: f32,
}

impl ClipEffect {
    fn amount(&self, amount: &Amount) -> f32 {
        match amount {
            Amount::Value(v) => *v,
            Amount::Param(name) => {
                let id = name.trim_start_matches('$');
                self.params.iter().find(|p| p.id == id).map_or(0.0, |p| p.value.clamp(p.min, p.max))
            }
        }
    }
}

/// Combine every filter on a clip, in order.
pub fn resolve(effects: &[ClipEffect]) -> ResolvedEffects {
    // How bright each colour looks (Rec. 709).
    const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
    let mut out = ResolvedEffects { color: ColorTransform::IDENTITY, blur: 0.0 };
    for effect in effects {
        for op in &effect.ops {
            let mut step = ColorTransform::IDENTITY;
            match op {
                EffectOp::Saturation(a) => {
                    let s = effect.amount(a).max(0.0);
                    for (r, row) in step.matrix.iter_mut().take(3).enumerate() {
                        for (c, luma) in LUMA.iter().enumerate() {
                            row[c] = luma * (1.0 - s) + if r == c { s } else { 0.0 };
                        }
                    }
                }
                EffectOp::Brightness(a) => {
                    let b = effect.amount(a).clamp(-1.0, 1.0);
                    step.offset = [b, b, b, 0.0];
                }
                EffectOp::Contrast(a) => {
                    let k = effect.amount(a).max(0.0);
                    for i in 0..3 {
                        step.matrix[i][i] = k;
                        step.offset[i] = 0.5 * (1.0 - k);
                    }
                }
                EffectOp::Blur(a) => {
                    out.blur += effect.amount(a).max(0.0);
                    continue;
                }
                EffectOp::Matrix(values) => {
                    if values.len() != 20 {
                        continue;
                    }
                    for r in 0..4 {
                        step.matrix[r].copy_from_slice(&values[r * 5..r * 5 + 4]);
                        step.offset[r] = values[r * 5 + 4];
                    }
                }
            }
            out.color = out.color.then(&step);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(t: &ColorTransform, rgb: [f32; 3]) -> [f32; 3] {
        let v = [rgb[0], rgb[1], rgb[2], 1.0];
        let mut out = [0.0; 3];
        for (r, value) in out.iter_mut().enumerate() {
            *value = (0..4).map(|c| t.matrix[r][c] * v[c]).sum::<f32>() + t.offset[r];
        }
        out
    }

    fn effect(ops: Vec<EffectOp>, value: f32) -> ClipEffect {
        ClipEffect {
            id: "t/x".into(),
            name: "x".into(),
            params: vec![EffectParam { id: "amount".into(), name: "Amount".into(), min: 0.0, max: 2.0, value, default: 1.0 }],
            ops,
        }
    }

    #[test]
    fn no_effects_change_nothing() {
        let r = resolve(&[]);
        assert!(r.color.is_identity());
        assert_eq!(r.blur, 0.0);
    }

    #[test]
    fn zero_saturation_is_grey_at_the_right_brightness() {
        let r = resolve(&[effect(vec![EffectOp::Saturation(Amount::Param("$amount".into()))], 0.0)]);
        let blue = apply(&r.color, [0.0, 0.0, 1.0]);
        assert!((blue[0] - 0.0722).abs() < 1e-4 && blue[0] == blue[1] && blue[1] == blue[2]);
        let white = apply(&r.color, [1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn steps_apply_in_order_and_settings_are_clamped() {
        // Darken by 0.5, then double the contrast: mid grey 0.5 -> 0.0 -> -0.5.
        let r = resolve(&[effect(vec![EffectOp::Brightness(Amount::Value(-0.5)), EffectOp::Contrast(Amount::Value(2.0))], 1.0)]);
        assert!((apply(&r.color, [0.5, 0.5, 0.5])[0] - (-0.5)).abs() < 1e-4);
        // A setting outside its range is held to the range.
        let r = resolve(&[effect(vec![EffectOp::Blur(Amount::Param("$amount".into()))], 99.0)]);
        assert_eq!(r.blur, 2.0);
    }

    #[test]
    fn recipes_read_from_plain_data() {
        let ops: Vec<EffectOp> = serde_json::from_str(r#"[{"saturation": "$amount"}, {"blur": 4.0}, {"matrix": [1,0,0,0,0, 0,1,0,0,0, 0,0,1,0,0, 0,0,0,1,0]}]"#).unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0], EffectOp::Saturation(Amount::Param("$amount".into())));
    }
}
