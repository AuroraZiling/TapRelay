//! Slint's software backend skips presentation when logical damage is empty.
//! Windows can request repainting even when the retained pixels have not changed.
//! Conservatively invalidate the whole client area on each requested frame, so
//! both rasterization and softbuffer presentation cover the exposed surface.
//!
//! https://github.com/slint-ui/slint/issues/9977
//! https://github.com/slint-ui/slint/pull/11150

use i_slint_core::{
    lengths::{LogicalRect, LogicalSize},
    window::WindowInner,
};
use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};

pub fn install(window: &slint::Window) {
    window.on_winit_window_event(|window, event| {
        if matches!(event, WindowEvent::RedrawRequested) {
            invalidate_surface(window);
        }
        EventResult::Propagate
    });
}

pub(crate) fn invalidate_surface(window: &slint::Window) {
    let size = window.size().to_logical(window.scale_factor());
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }
    WindowInner::from_pub(window)
        .window_adapter()
        .renderer()
        .mark_dirty_region(
            LogicalRect::from_size(LogicalSize::new(size.width, size.height)).into(),
        );
}
