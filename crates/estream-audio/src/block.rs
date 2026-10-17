use crate::BLOCK;

/// One 10 ms block of planar stereo audio.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub l: [f32; BLOCK],
    pub r: [f32; BLOCK],
}

impl Default for Block {
    fn default() -> Self {
        Self::SILENT
    }
}

impl Block {
    pub const SILENT: Self = Self {
        l: [0.0; BLOCK],
        r: [0.0; BLOCK],
    };

    pub fn clear(&mut self) {
        self.l.fill(0.0);
        self.r.fill(0.0);
    }

    /// `self += other`.
    pub fn add(&mut self, other: &Block) {
        for (a, b) in self.l.iter_mut().zip(&other.l) {
            *a += b;
        }
        for (a, b) in self.r.iter_mut().zip(&other.r) {
            *a += b;
        }
    }

    pub fn scale(&mut self, gain_l: f32, gain_r: f32) {
        self.l.iter_mut().for_each(|s| *s *= gain_l);
        self.r.iter_mut().for_each(|s| *s *= gain_r);
    }

    /// Absolute peak per channel.
    pub fn peak(&self) -> [f32; 2] {
        let p = |c: &[f32]| c.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        [p(&self.l), p(&self.r)]
    }

    /// RMS per channel.
    pub fn rms(&self) -> [f32; 2] {
        let r = |c: &[f32]| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt();
        [r(&self.l), r(&self.r)]
    }

    pub fn is_silent(&self) -> bool {
        self.l.iter().chain(&self.r).all(|s| *s == 0.0)
    }
}
