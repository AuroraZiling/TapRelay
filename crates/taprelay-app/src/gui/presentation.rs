use crate::config::Geometry;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ShortcutItem {
    pub text: String,
    pub keys: Vec<String>,
    pub slot: i32,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BindingCaptureState {
    pub function_id: String,
    pub slot: i32,
    pub text: String,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GestureAction {
    pub gesture: String,
    pub action: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FunctionBindingRow {
    pub id: String,
    pub label: String,
    pub gestures: Vec<GestureAction>,
    pub enabled: bool,
    pub shortcuts: Vec<ShortcutItem>,
    pub scope_label: String,
    pub scope_editable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ForegroundAppGroupRow {
    pub id: String,
    pub name: String,
    pub count: i32,
    pub count_label: String,
    pub summary: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ForegroundAppRow {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ForegroundAppScopeOption {
    pub id: String,
    pub name: String,
    pub checked: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BindingRow {
    pub text: String,
    pub keys: Vec<String>,
    pub enabled: bool,
    pub function_name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DeviceRow {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub selected: bool,
    pub action: String,
    pub action_label: String,
    pub enabled: bool,
    pub paired: bool,
    pub device_kind: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CheckRow {
    pub title: String,
    pub state: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MappingRow {
    pub id: String,
    pub name: String,
    pub input: String,
    pub output: String,
    pub scope: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MappingOutputOption {
    pub label: String,
    pub value: String,
    pub is_mouse: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Bindings {
    pub rows: Vec<FunctionBindingRow>,
    pub capture: BindingCaptureState,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Groups {
    pub scope_function: String,
    pub scope_options: Vec<ForegroundAppScopeOption>,
    pub icon_revision: i32,
    pub groups: Vec<ForegroundAppGroupRow>,
    pub foreground_apps: Vec<ForegroundAppRow>,
    pub running: Vec<ForegroundAppRow>,
    pub busy: bool,
    pub saving: bool,
    pub editor_open: bool,
    pub selected_id: String,
    pub selected_name: String,
    pub error: String,
    pub form_error: String,
    pub deletion_warning: String,
    pub confirming_delete: bool,
    pub choosing_running: bool,
    pub focus_request: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Mapping {
    pub rows: Vec<MappingRow>,
    pub open: bool,
    pub editor: bool,
    pub new_mapping: bool,
    pub focus_request: i32,
    pub leave_focus_request: i32,
    pub busy: bool,
    pub saving: bool,
    pub name: String,
    pub inputs: Vec<String>,
    pub output_options: Vec<MappingOutputOption>,
    pub selected_output: MappingOutputOption,
    pub output_filter: String,
    pub entry_mode: i32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub scopes: Vec<ForegroundAppScopeOption>,
    pub scope_label: String,
    pub input_error: String,
    pub capture_error: String,
    pub error: String,
    pub conflict_target: String,
    pub capture_text: String,
    pub capture_slot: i32,
    pub can_undo: bool,
    pub leave_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChoiceOption {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum WindowCommand {
    Show,
    Hide,
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Presentation {
    pub mode: i32,
    pub page: i32,
    pub wizard_page: i32,
    pub bindings: Vec<BindingRow>,
    pub checks: Vec<CheckRow>,
    pub listening: bool,
    pub ready: bool,
    pub receiver_next_allowed: bool,
    pub discovery_status: String,
    pub adapter_label: String,
    pub scanning: bool,
    pub paired_devices: Vec<DeviceRow>,
    pub bluetooth_available: bool,
    pub busy: bool,
    pub elevated: bool,
    pub device_name: String,
    pub device_selected: bool,
    pub stage: String,
    pub stage_detail: String,
    pub diagnostics: String,
    pub problem: String,
    pub problem_retry_allowed: bool,
    pub problem_busy: bool,
    pub input_count: u64,
    pub progress_message: String,
    pub state_color: i32,
    pub data_directory: String,
    pub app_version: String,
    pub auto_listen: bool,
    pub passthrough_overlay: bool,
    pub passthrough_reverse_scroll: bool,
    pub passthrough_mouse_report_rate: i32,
    pub auto_start: bool,
    pub start_hidden: bool,
    pub close_to_tray: bool,
    pub always_admin: bool,
    pub notifications: bool,
    pub connection_wait_warning: bool,
    pub theme_choice: i32,
    pub language_choice: i32,
    pub language_options: Vec<ChoiceOption>,
    pub binding: Bindings,
    pub groups: Groups,
    pub mapping: Mapping,
    pub locale: String,
    pub dark: bool,
    pub geometry: Geometry,
    pub window_command: Option<WindowCommand>,
    pub overlay_background: [u8; 3],
    pub overlay_foreground: [u8; 3],
}
