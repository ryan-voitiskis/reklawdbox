//! Compatibility façade for Rekordbox metadata color rules.
//!
//! The canonical implementation lives in [`crate::domain::metadata::color`].

#[allow(unused_imports)]
pub(crate) use crate::domain::metadata::{
    COLORS, canonical_color_name, color_name_to_code, is_valid_color,
};
