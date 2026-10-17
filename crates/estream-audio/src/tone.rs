//! Test tone source: a sine wave on both channels.

use crate::block::Block;
use crate::source::AudioSource;
use crate::{BLOCK, SAMPLE_RATE, db};
use estream_core::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneSettings {
    pub frequency: f64,
    pub level_db: f64,
}

impl Default for ToneSettings {
    fn default() -> Self {
        Self {
            frequency: 440.0,
            level_db: -18.0,
        }
    }
}

#[derive(Default)]
pub struct ToneSource {
    settings: ToneSettings,
    phase: f64,
}

impl AudioSource for ToneSource {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        self.settings = if settings.is_null() {
            ToneSettings::default()
        } else {
            serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?
        };
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        let amp = db::to_gain(self.settings.level_db as f32);
        let step = self.settings.frequency * std::f64::consts::TAU / SAMPLE_RATE as f64;
        for i in 0..BLOCK {
            let s = (self.phase.sin() as f32) * amp;
            block.l[i] = s;
            block.r[i] = s;
            self.phase = (self.phase + step) % std::f64::consts::TAU;
        }
        BLOCK
    }
}
