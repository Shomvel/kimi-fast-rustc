//! Graph transforms; pipeline ordering and finalization live in `pipeline`.

pub(crate) mod control;
pub(crate) mod ipo;
pub(crate) mod memory;
pub(crate) mod scalar;
pub(crate) mod utils;
