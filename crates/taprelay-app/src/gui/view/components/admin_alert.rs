use super::ActionAlert;
use crate::gui::action::Action;
use crate::gui::view::AppView;
use gpui_kit::Context;

impl AppView {
    pub(in crate::gui::view) fn admin_alert(&self, cx: &Context<Self>) -> ActionAlert {
        if self.ui.capabilities.privacy_permissions {
            return ActionAlert::error("permissions-alert", self.tr("permissions.mac.hint"))
                .action(self.app_button(
                    "input-permissions",
                    "common.open_settings",
                    Action::Elevate,
                    false,
                    cx,
                ));
        }
        ActionAlert::error("admin-alert", self.tr("permissions.warning")).action(self.app_button(
            "elevate",
            "permissions.restart.short",
            Action::Elevate,
            false,
            cx,
        ))
    }
}
