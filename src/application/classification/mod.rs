//! Genre classification workflows shared independently of MCP transport types.

mod calibrate;
mod classify;
#[cfg(test)]
pub(crate) mod evaluate;
pub(crate) mod evidence;
#[cfg(test)]
mod profile_evaluation;

pub(crate) use calibrate::{CalibrationError, calibrate_audio_profiles, calibration_coverage};
pub(crate) use classify::classify_batch;
#[cfg(test)]
pub(crate) use classify::classify_batch_with_audio_identities;
