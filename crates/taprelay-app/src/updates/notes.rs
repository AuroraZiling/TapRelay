use anyhow::{Result, ensure};
use serde::Deserialize;

use super::{Target, text};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Notes {
    pub(crate) en: LocalizedNotes,
    pub(crate) zh: LocalizedNotes,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct LocalizedNotes {
    pub(crate) fallback: bool,
    pub(crate) blocks: Vec<NoteBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct NoteBlock {
    pub(crate) kind: NoteKind,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NoteKind {
    Added,
    Fixed,
    Improved,
    Heading,
    Paragraph,
    Bullet,
}

pub(crate) fn notes(target: &Target) -> Option<Notes> {
    at("https://taprelay.ziling.moe", &target.tag())
}

pub(super) fn at(site: &str, tag: &str) -> Option<Notes> {
    #[derive(Deserialize)]
    struct Response {
        tag: String,
        #[serde(flatten)]
        notes: Notes,
    }
    let result: Result<Notes> = (|| {
        let response: Response =
            serde_json::from_str(&text(&format!("{site}/api/changelog/{tag}/"))?)?;
        ensure!(response.tag == tag, "Release notes returned another tag.");
        Ok(response.notes)
    })();
    result
        .inspect_err(|error| tracing::warn!(%tag, %error, "Release notes unavailable"))
        .ok()
}
