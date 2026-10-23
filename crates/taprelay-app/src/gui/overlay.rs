use crate::runtime::passthrough::Notice;

#[derive(Default)]
pub struct Overlay {
    native: Option<crate::platform::desktop::Overlay>,
    notice: Option<Notice>,
    appearance: Option<(String, [u8; 3], [u8; 3])>,
}

impl Overlay {
    pub fn update(
        &mut self,
        notice: Option<Notice>,
        enabled: bool,
        locale: &str,
        background: [u8; 3],
        foreground: [u8; 3],
    ) {
        let native = &mut self.native;
        if !enabled {
            if let Some(native) = native.as_mut() {
                native.hide();
            }
            self.notice = None;
            return;
        }
        let changed = notice.is_some();
        if let Some(notice) = notice {
            self.notice = Some(notice);
        }
        let appearance = (locale.to_owned(), background, foreground);
        let restyle = self.appearance.as_ref() != Some(&appearance);
        self.appearance = Some(appearance);
        let result = (|| -> anyhow::Result<()> {
            if (changed || restyle)
                && let Some(notice) = &self.notice
            {
                if native.is_none() {
                    *native = Some(crate::platform::desktop::Overlay::new()?);
                }
                let (prefix, device, suffix) = notice.text_parts(locale);
                native.as_mut().unwrap().update(
                    &prefix,
                    &device,
                    &suffix,
                    background,
                    foreground,
                    changed.then_some(notice.duration()),
                )?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "Passthrough overlay unavailable");
            *native = None;
            self.notice = None;
        }
    }

    pub fn is_visible(&self) -> bool {
        self.native
            .as_ref()
            .is_some_and(|window| window.is_visible())
    }

    pub fn tick(&mut self) {
        if let Some(window) = self.native.as_mut()
            && let Err(error) = window.tick()
        {
            tracing::warn!(%error, "Passthrough overlay animation failed");
            self.native = None;
        }
    }
}
