use crate::updates::Notes;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading(u8),
    Applying,
    CheckFailed,
    DownloadFailed,
    ApplyFailed,
    Unsupported,
}

impl UpdateStatus {
    pub(in crate::gui) fn committing(self) -> bool {
        matches!(self, Self::Downloading(_) | Self::Applying)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UpdateOffer {
    pub tag: String,
    pub portable: bool,
    pub show_notes: bool,
    pub notes_loading: bool,
    pub notes: Option<Arc<Notes>>,
}
