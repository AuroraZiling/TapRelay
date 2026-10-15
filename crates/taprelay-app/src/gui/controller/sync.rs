use super::*;

impl Controller {
    pub(in crate::gui) fn sync(&mut self, ui: &mut Presentation) {
        if self.last_system_poll.elapsed() >= Duration::from_secs(1) {
            self.system_theme = desktop::system_dark();
            self.system_language = desktop::system_locale_id();
            self.last_system_poll = Instant::now();
        }
        let locale = self.locale();
        ui.locale = locale.to_owned();
        self.sync_group_operation(ui);
        self.sync_mapping_operation(ui);
        let icons_changed = self.foreground_app_icons.borrow_mut().poll();
        if icons_changed {
            ui.groups.icon_revision = ui.groups.icon_revision.wrapping_add(1);
        }
        ui.groups.busy = self.runtime.busy() || self.runtime.stopped() || self.recording();
        let o = &self.runtime.config.options;
        let dark = match o.theme {
            ThemeSetting::System => self.system_theme,
            ThemeSetting::Dark => true,
            ThemeSetting::Light => false,
        };
        ui.dark = dark;
        ui.theme_choice = match o.theme {
            ThemeSetting::System => 0,
            ThemeSetting::Light => 1,
            ThemeSetting::Dark => 2,
        };
        ui.language_options = i18n::language_options(locale);
        ui.language_choice = match o.language {
            Language::System => 0,
            ref selected => {
                1 + i18n::locale::all()
                    .iter()
                    .position(|compiled| compiled == selected.locale())
                    .unwrap_or_default() as i32
            }
        };
        ui.auto_start = o.autostart;
        ui.auto_listen = o.auto_listen;
        ui.passthrough_overlay = o.passthrough_overlay;
        ui.passthrough_reverse_scroll = o.passthrough_reverse_scroll;
        ui.passthrough_mouse_report_rate = i32::from(o.passthrough_mouse_report_rate);
        ui.start_hidden = o.start_hidden;
        ui.close_to_tray = o.close_to_tray;
        ui.always_admin = o.always_admin;
        ui.notifications = o.notifications;
        ui.connection_wait_warning = o.connection_wait_warning;
        ui.elevated = self.status.elevated;
        ui.app_version = crate::constants::VERSION.into();
        ui.data_directory = self
            .path
            .parent()
            .unwrap()
            .to_string_lossy()
            .as_ref()
            .into();
        let s = &self.runtime.state;
        let flags = [s.adapter, s.peripheral, s.service, s.broadcasting];
        ui.bluetooth_available = s.adapter_state == taprelay_core::devices::AdapterState::Available;
        ui.scanning = s.discovery == taprelay_core::devices::DiscoveryState::Scanning;
        ui.adapter_label = self.tr(crate::gui::presentation::receiver::adapter_label_key(
            s.adapter_state,
        ));
        ui.receiver_next_allowed = taprelay_core::devices::receiver_next_allowed(s);
        ui.device_selected = s.selected.is_some();
        ui.discovery_status = self.tr(crate::gui::presentation::receiver::page_status_key(s));
        let scope = if self.fatal.is_some() {
            crate::gui::feedback::ErrorScope::Startup
        } else if self.runtime.stopped() {
            crate::gui::feedback::ErrorScope::Runtime
        } else {
            crate::gui::feedback::ErrorScope::Connection
        };
        let failure = self.failure();
        if self.fatal.is_some() {
            ui.mode = 3;
        }
        ui.problem = if failure.is_some() {
            self.tr(scope.summary_key(s))
        } else {
            "".into()
        };
        ui.problem_retry_allowed = scope.can_retry() && !self.runtime.stopped();
        ui.problem_busy = self.runtime.busy() || self.recovering;
        ui.busy = (!self.passed || self.recovering) && failure.is_none();
        let checks_key = (flags, self.fatal.is_some(), failure.is_some(), locale);
        if self.previous_checks != Some(checks_key) {
            self.previous_checks = Some(checks_key);
            ui.checks = crate::gui::presentation::startup::rows(
                locale,
                self.fatal.is_some(),
                flags,
                failure.is_some(),
            );
        }
        let target = s.selected_target();
        let (key, stage) = if !self.passed {
            let key = if failure.is_some() {
                keys::STARTUP_ERROR
            } else {
                keys::STARTUP_PREPARING
            };
            (key, self.tr(key))
        } else if s.profile_switching && failure.is_none() {
            ("switching", self.tr(keys::RECEIVER_SWITCHING))
        } else if self.recovering && failure.is_none() {
            ("prepare", self.tr(keys::RECEIVER_PREPARING))
        } else if self.fatal.is_some()
            || self.runtime.stopped()
            || (self.passed
                && (!s.adapter
                    || (!s.service_paused
                        && !s.profile_switching
                        && (!s.service || !s.broadcasting))))
        {
            ("fault", self.tr(keys::RECEIVER_FAULT))
        } else if s.activity == TransportActivity::ResolvingDevice {
            ("identity", self.tr(keys::RECEIVER_RESOLVING))
        } else if s.selected.is_none() {
            ("unselected", self.tr(keys::RECEIVER_CHOOSE))
        } else {
            let status = self.session_presentation.key(s, Instant::now());
            (status, self.tr(status))
        };
        let has_bindings = self
            .runtime
            .config
            .functions
            .values()
            .any(|function| function.enabled && !function.shortcuts.is_empty())
            || self
                .runtime
                .config
                .custom_mappings
                .iter()
                .any(|mapping| mapping.enabled);
        let (display_key, signal) = crate::gui::presentation::receiver::relay_presentation(
            key,
            self.runtime.listening,
            has_bindings,
            self.runtime.passthrough,
        );
        let stage = if display_key != key {
            self.tr(display_key)
        } else {
            stage
        }
        .to_owned();
        let key = display_key;
        if self.stage_key != key {
            tracing::info!(previous=%self.stage_key,elapsed_ms=self.stage_since.elapsed().as_millis(),next=key,"Connection stage changed");
            self.stage_key = key.into();
            self.stage_since = Instant::now();
        }
        ui.stage = stage;
        ui.ready = s.ready && !s.profile_switching && key != keys::RECEIVER_SUSPENDED;
        ui.listening = self.runtime.listening;
        ui.state_color = if (!self.passed && failure.is_some()) || key == "fault" {
            3
        } else {
            signal
        };
        let receiver_none = self.tr(keys::RECEIVER_NONE);
        ui.device_name = s
            .selected_target()
            .map(|t| {
                if t.name.trim().is_empty() {
                    t.id.as_str()
                } else {
                    t.name.as_str()
                }
            })
            .unwrap_or(&receiver_none)
            .into();
        let diagnostics = format!(
            "Adapter={} · Peripheral={} · Service={} · Advertising={}\nLink={:?} · Subscription={:?} · Input={} · HID={:?}\n{}: {}",
            s.adapter,
            s.peripheral,
            s.service,
            s.broadcasting,
            target.map(|t| t.link),
            target.map(|t| t.subscribed),
            s.input,
            s.hid_profile,
            self.tr(keys::DIAGNOSTICS_INPUTS),
            self.runtime.matched
        );
        ui.diagnostics = diagnostics;
        let keyboard_layout = crate::platform::keyboard_layout();
        if self.previous_bindings != Some((self.runtime.bindings_revision, locale, keyboard_layout))
        {
            self.previous_bindings =
                Some((self.runtime.bindings_revision, locale, keyboard_layout));
            self.sync_foreground_app_groups(ui);
            self.sync_mapping_rows(ui);
            let mut rows = Vec::new();
            let mut summary = Vec::new();
            for definition in taprelay_core::function::FUNCTION_CATALOG {
                let Some(config) = self.runtime.config.functions.get(&definition.id) else {
                    continue;
                };
                // The catalog states a function as up to two gestures, so the
                // row shows exactly what each one does instead of naming the
                // pair and leaving the reader to guess which half is the hold.
                let mut gestures = vec![GestureAction {
                    gesture: self.tr(keys::BINDINGS_GESTURE_PRESS),
                    action: self.tr(definition.tap_action.name_key()),
                }];
                if let Some(hold) = definition.hold_action {
                    gestures.push(GestureAction {
                        gesture: self.tr(keys::BINDINGS_GESTURE_HOLD),
                        action: self.tr(hold.name_key()),
                    });
                }
                if definition.repeats() {
                    gestures.push(GestureAction {
                        gesture: self.tr(keys::BINDINGS_GESTURE_HOLD),
                        action: self.tr(keys::BINDINGS_GESTURE_REPEAT),
                    });
                }
                let shortcuts = config
                    .shortcuts
                    .iter()
                    .enumerate()
                    .map(|(slot, shortcut)| {
                        let keys = i18n::shortcut_labels(self.locale(), shortcut);
                        ShortcutItem {
                            text: keys.join("+"),
                            keys: keys.into_iter().collect::<Vec<String>>(),
                            slot: slot as i32,
                            enabled: config.enabled,
                        }
                    })
                    .collect::<Vec<_>>();
                if config.enabled {
                    for shortcut in &config.shortcuts {
                        let keys = i18n::shortcut_labels(self.locale(), shortcut);
                        summary.push(BindingRow {
                            text: keys.join("+"),
                            function_name: self.tr(definition.name_key),
                            keys: keys.into_iter().collect::<Vec<String>>(),
                            enabled: true,
                        });
                    }
                }
                let row = FunctionBindingRow {
                    scope_label: self
                        .runtime
                        .config
                        .foreground_app_rules
                        .assignments
                        .get(&definition.id)
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|id| {
                                    self.runtime.config.foreground_app_rules.groups.get(id)
                                })
                                .map(|group| group.name.as_str())
                                .collect::<Vec<_>>()
                                .join(" / ")
                        })
                        .unwrap_or_else(|| self.tr(keys::COMMON_ALL_APPS)),
                    scope_editable: definition.category == function::CategoryId::Media,
                    id: definition.id.stable_id().into(),
                    label: self.tr(definition.name_key),
                    gestures,
                    enabled: config.enabled,
                    shortcuts,
                };
                rows.push(row);
            }
            ui.binding.rows = rows;
            for mapping in &self.runtime.config.custom_mappings {
                if mapping.enabled {
                    for shortcut in &mapping.shortcuts {
                        let keys = i18n::shortcut_labels(self.locale(), shortcut);
                        summary.push(BindingRow {
                            text: keys.join("+"),
                            function_name: mapping.name.clone(),
                            keys: keys.into_iter().collect::<Vec<String>>(),
                            enabled: true,
                        });
                    }
                }
            }
            ui.bindings = summary;
        }
        if self
            .previous_devices
            .as_ref()
            .is_none_or(|(targets, selected, previous, adapter)| {
                targets != &s.targets
                    || selected != &s.selected
                    || *previous != locale
                    || *adapter != s.adapter_state
            })
        {
            self.previous_devices = Some((
                s.targets.clone(),
                s.selected.clone(),
                locale,
                s.adapter_state,
            ));
            let rows: Vec<_> = s
                .targets
                .iter()
                .map(|d| crate::gui::presentation::receiver::row(d, s, |key| self.tr(key)))
                .collect();
            ui.paired_devices = rows
                .iter()
                .filter(|r| r.paired)
                .cloned()
                .collect::<Vec<_>>();
        }
        let capture = self
            .runtime
            .capture()
            .filter(|capture| matches!(capture.kind, CaptureKind::Function(_)));
        let target = capture.and_then(|session| match session.kind {
            CaptureKind::Function(target) => Some(target),
            _ => None,
        });
        ui.binding.capture = BindingCaptureState {
            function_id: target
                .map(|target| target.id.stable_id())
                .unwrap_or("")
                .into(),
            slot: target.map_or(-1, |target| target.slot as i32),
            text: match capture {
                Some(session) if session.phase == CapturePhase::WaitingForRelease => {
                    self.tr(keys::CAPTURE_RELEASE)
                }
                Some(session) if !session.preview.is_empty() => {
                    i18n::capture_preview(self.locale(), session)
                }
                _ => self.tr(keys::CAPTURE_RECORDING),
            },
            error: match capture.and_then(|session| session.error.as_ref()) {
                Some(CaptureError::Invalid) => self.tr(keys::CAPTURE_INVALID),
                Some(CaptureError::Duplicate(id)) => format!(
                    "{}: {}",
                    self.tr(keys::CAPTURE_DUPLICATE),
                    self.tr(function::function_definition(*id).name_key)
                ),
                Some(CaptureError::Commit(error)) => error.clone(),
                Some(CaptureError::DuplicateMapping(name)) => {
                    format!("{}: {}", self.tr(keys::CAPTURE_DUPLICATE), name)
                }
                None => String::new(),
            },
        };
        ui.input_count = self.runtime.matched;
        ui.progress_message = if self.runtime.slow() {
            self.tr(keys::RUNTIME_SLOW)
        } else if self.exit.is_some() {
            self.tr(keys::RUNTIME_FINISHING)
        } else if self.runtime.progress_pending() {
            self.tr(keys::RUNTIME_BUSY)
        } else if matches!(self.runtime.test, TestStatus::Pending(_)) {
            self.tr(keys::TEST_SENDING)
        } else {
            "".into()
        };
    }
}
