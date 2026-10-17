//! Decibel helpers. Silence is `-inf` dB.

pub fn to_gain(db: f32) -> f32 {
    if db <= -96.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

pub fn from_gain(gain: f32) -> f32 {
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}

/// Converts milliseconds to a one-pole smoothing coefficient at 48 kHz.
pub fn time_coefficient(ms: f32) -> f32 {
    let samples = (ms.max(0.01) / 1000.0) * crate::SAMPLE_RATE as f32;
    (-1.0 / samples).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert!((to_gain(-6.0) - 0.501).abs() < 0.001);
        assert!((from_gain(0.5) + 6.02).abs() < 0.01);
        assert_eq!(to_gain(-120.0), 0.0);
        assert_eq!(from_gain(0.0), f32::NEG_INFINITY);
    }
}
