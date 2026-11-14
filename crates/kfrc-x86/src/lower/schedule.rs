//! The machine-independent schedule lives in `kfrc_son::schedule`; this
//! selector consumes it unchanged.
pub(super) use kfrc_son::schedule::Schedule;

impl From<kfrc_son::schedule::ScheduleError> for super::LowerError {
    fn from(error: kfrc_son::schedule::ScheduleError) -> Self {
        Self::Malformed(error.0)
    }
}
