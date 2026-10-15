use super::Config;
use anyhow::Result;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct SaveQueue {
    due: Option<Instant>,
}
impl SaveQueue {
    pub fn changed(&mut self) {
        self.due = Some(Instant::now() + Duration::from_millis(350));
    }
    pub fn flush(&mut self, config: &Config, path: &Path, force: bool) -> Result<()> {
        if self.due.is_some_and(|d| force || Instant::now() >= d) {
            self.changed();
            config.save(path)?;
            self.due = None;
        }
        Ok(())
    }
}
