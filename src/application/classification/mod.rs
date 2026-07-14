//! Genre classification workflows shared independently of MCP transport types.

mod calibrate;
mod classify;
#[cfg(test)]
pub(crate) mod evaluate;
pub(crate) mod evidence;

pub(crate) use calibrate::{CalibrationError, calibrate_audio_profiles, calibration_coverage};
pub(crate) use classify::classify_batch;
#[cfg(test)]
pub(crate) use classify::classify_batch_rules_only;
