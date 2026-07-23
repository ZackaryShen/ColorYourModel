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

/// Apply falloff to strength based on distance from center
pub fn apply_falloff(strength: f32, distance: f32, radius: f32, mode: &str) -> f32 {
    let t = (distance / radius).clamp(0.0, 1.0);
    match mode {
        "linear" => strength * (1.0 - t),
        "smooth" => strength * (1.0 - t * t * (3.0 - 2.0 * t)),
        "step" => {
            if t < 0.5 {
                strength
            } else {
                0.0
            }
        }
        _ => strength * (1.0 - t),
    }
}
