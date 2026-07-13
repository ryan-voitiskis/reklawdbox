//! Genre classification workflows shared independently of MCP transport types.

mod calibrate;
mod classify;
pub(crate) mod evidence;

pub(crate) use calibrate::{CalibrationError, calibrate_audio_profiles, calibration_coverage};
pub(crate) use classify::classify_batch;
