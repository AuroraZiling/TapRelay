use super::ActionAlert;
use crate::gui::action::Action;
use crate::gui::view::AppView;
use gpui_kit::Context;

impl AppView {
    pub(in crate::gui::view) fn admin_alert(&self, cx: &Context<Self>) -> ActionAlert {
        ActionAlert::error("admin-alert", self.tr("permissions.warning")).action(self.app_button(
            "elevate",
            "permissions.restart.short",
            Action::Elevate,
            false,
            cx,
        ))
    }
}
