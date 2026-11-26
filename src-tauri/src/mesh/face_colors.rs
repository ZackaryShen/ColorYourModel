/// Face color management utilities

/// Mix two RGBA colors with a given strength (0.0 = all original, 1.0 = all target)
pub fn mix_color(original: &[u8; 4], target: &[u8; 4], strength: f32) -> [u8; 4] {
    let s = strength.clamp(0.0, 1.0);
    let inv = 1.0 - s;
    [
        (original[0] as f32 * inv + target[0] as f32 * s) as u8,
        (original[1] as f32 * inv + target[1] as f32 * s) as u8,
        (original[2] as f32 * inv + target[2] as f32 * s) as u8,
        255,
    ]
}

pub(crate) fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
