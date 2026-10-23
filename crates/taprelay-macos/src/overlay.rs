//! Nonactivating AppKit status panel
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSScreen, NSStatusWindowLevel, NSTextAlignment,
    NSTextField, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::time::{Duration, Instant};

pub struct Overlay {
    main_thread: MainThreadMarker,
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
    until: Option<Instant>,
    visible: bool,
}
impl Overlay {
    pub fn new() -> std::io::Result<Self> {
        let main_thread = MainThreadMarker::new()
            .ok_or_else(|| std::io::Error::other("Overlay requires app main thread"))?;
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(600., 48.));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(main_thread),
            frame,
            NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setLevel(NSStatusWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        // Retained owns the panel
        unsafe {
            panel.setReleasedWhenClosed(false);
        }
        let label = NSTextField::labelWithString(&NSString::from_str(""), main_thread);
        label.setFrame(NSRect::new(NSPoint::new(16., 14.), NSSize::new(568., 24.)));
        label.setAlignment(NSTextAlignment::Center);
        panel
            .contentView()
            .ok_or_else(|| std::io::Error::other("Overlay has no content view"))?
            .addSubview(&label);
        Ok(Self {
            main_thread,
            panel,
            label,
            until: None,
            visible: false,
        })
    }
    pub fn update(
        &mut self,
        prefix: &str,
        device: &str,
        suffix: &str,
        background: [u8; 3],
        foreground: [u8; 3],
        duration: Option<Duration>,
    ) -> std::io::Result<()> {
        self.label
            .setStringValue(&NSString::from_str(&format!("{prefix}{device}{suffix}")));
        self.panel.setBackgroundColor(Some(&color(background)));
        self.label.setTextColor(Some(&color(foreground)));
        if let Some(screen) = NSScreen::mainScreen(self.main_thread) {
            let frame = screen.visibleFrame();
            self.panel.setFrameOrigin(NSPoint::new(
                frame.origin.x + (frame.size.width - 600.) / 2.,
                frame.origin.y + 36.,
            ));
        }
        self.panel.orderFrontRegardless();
        self.visible = true;
        if let Some(duration) = duration {
            self.until = Some(Instant::now() + duration);
        }
        Ok(())
    }
    pub fn hide(&mut self) {
        self.panel.orderOut(None);
        self.visible = false;
        self.until = None;
    }
    pub fn is_visible(&self) -> bool {
        self.visible
    }
    pub fn tick(&mut self) -> std::io::Result<()> {
        if self.until.is_some_and(|due| Instant::now() >= due) {
            self.hide();
        }
        Ok(())
    }
}
fn color(rgb: [u8; 3]) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        rgb[0] as f64 / 255.,
        rgb[1] as f64 / 255.,
        rgb[2] as f64 / 255.,
        1.,
    )
}
impl Drop for Overlay {
    fn drop(&mut self) {
        self.hide();
        self.panel.close();
    }
}
