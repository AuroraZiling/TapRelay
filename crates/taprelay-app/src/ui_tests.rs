//! Headless view validation only: no native window, permissions, input hooks or Bluetooth.
use crate::*;
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

fn set_function_bindings(ui: &AppWindow, rows: Vec<FunctionBindingRow>) {
    ui.global::<BindingUi>()
        .set_rows(ModelRc::new(VecModel::from(rows)));
}

fn set_binding_capture(ui: &AppWindow, function_id: &str, slot: i32, text: &str, error: &str) {
    ui.global::<BindingUi>().set_capture(BindingCaptureState {
        function_id: function_id.into(),
        slot,
        text: text.into(),
        error: error.into(),
    });
}

fn binding_element(ui: &AppWindow, accessible_id: &str) -> ElementHandle {
    let accessible_id = accessible_id.to_owned();
    let query_id = accessible_id.clone();
    ui.root_element()
        .query_descendants()
        .match_predicate(move |element| {
            element
                .accessible_id()
                .is_some_and(|candidate| candidate.as_str() == query_id)
        })
        .find_first()
        .unwrap_or_else(|| panic!("missing accessible binding element: {accessible_id}"))
}

#[derive(Debug, PartialEq, Eq)]
enum BindingUiAction {
    Begin(String, i32),
    Delete(String, i32),
    Toggle(String, bool),
}

struct SoftwareTestPlatform {
    window: std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
}

impl slint::platform::Platform for SoftwareTestPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> std::time::Duration {
        std::time::Duration::from_millis(i_slint_backend_testing::get_mocked_time())
    }
}

#[test]
fn render_all_views_without_hardware() {
    let render_window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
    );
    slint::platform::set_platform(Box::new(SoftwareTestPlatform {
        window: render_window.clone(),
    }))
    .unwrap();
    let ui = AppWindow::new().unwrap();
    render_pages(&ui);
    check_environment_failure(&ui);
    check_binding_capture(&ui);
    check_navigation_and_settings(&ui);
    check_surface_redraw(&ui, &render_window);
    check_binding_layouts_and_actions(&ui);
    check_volume_bindings(&ui);
    check_empty_foreground_app_groups(&ui);
    check_foreground_app_groups(&ui);
    check_foreground_app_icons(&ui);
    check_foreground_app_list_virtualization(&ui);
    check_about_links(&ui);
    check_runtime_feedback(&ui);
}

fn check_foreground_app_icons(ui: &AppWindow) {
    #[cfg(windows)]
    {
        let path = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("explorer.exe")
            .to_string_lossy()
            .into_owned();
        let pixels =
            crate::platform::foreground_apps::executable_icon(&path).expect("Explorer icon");
        let mut buffer =
            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(pixels.size, pixels.size);
        buffer.make_mut_bytes().copy_from_slice(&pixels.rgba);
        let icon = slint::Image::from_rgba8(buffer);
        let icon_path = path.clone();
        let groups = ui.global::<ForegroundAppUi>();
        groups.on_app_icon(move |path, _| {
            if path == icon_path {
                icon.clone()
            } else {
                slint::Image::default()
            }
        });
        let icon_path = path.clone();
        groups.on_has_icon(move |path, _| path == icon_path);
        groups.set_selected_id("".into());
        groups.set_selected_name("".into());
        groups.set_choosing_running(false);
        groups.set_foreground_apps(ModelRc::new(VecModel::from(vec![ForegroundAppRow {
            name: "explorer.exe".into(),
            path: path.clone().into(),
        }])));
        i18n::apply(ui, "zh-cn");
        groups.set_form_error(i18n::text("zh-cn", "groups.name_required").into());
        ui.invoke_show_group_editor();
        ui.window().take_snapshot().unwrap();
        let image = binding_element(ui, &format!("group-icon-{path}"));
        assert!(image.size().width >= 28. && image.size().height >= 28.);
        let title_id = format!("group-foreground-app-name-{path}");
        assert_eq!(
            binding_element(ui, &title_id).accessible_label().as_deref(),
            Some("explorer.exe")
        );
        let name_path = path.clone();
        groups.on_app_name(move |path, _| {
            if path == name_path {
                "文件资源管理器".into()
            } else {
                "".into()
            }
        });
        groups.set_icon_revision(groups.get_icon_revision() + 1);
        ui.window().take_snapshot().unwrap();
        assert_eq!(
            binding_element(ui, &title_id).accessible_label().as_deref(),
            Some("文件资源管理器")
        );
        save_groups_snapshot(ui, "groups-icons-footer-zh-cn");
        groups.on_app_missing(|_, _| true);
        groups.set_icon_revision(groups.get_icon_revision() + 1);
        ui.window().take_snapshot().unwrap();
        assert_eq!(
            binding_element(ui, &format!("group-missing-{path}"))
                .accessible_label()
                .as_deref(),
            Some(i18n::text("zh-cn", "groups.missing").as_str())
        );
        ui.invoke_close_group_editor();
        groups.on_app_name(|_, _| "".into());
        groups.on_app_missing(|_, _| false);
        groups.set_form_error("".into());
    }
}

fn check_foreground_app_list_virtualization(ui: &AppWindow) {
    struct CountedForegroundApps {
        reads: std::rc::Rc<std::cell::Cell<usize>>,
        notify: slint::ModelNotify,
    }
    impl Model for CountedForegroundApps {
        type Data = ForegroundAppRow;
        fn row_count(&self) -> usize {
            1000
        }
        fn row_data(&self, index: usize) -> Option<Self::Data> {
            self.reads.set(self.reads.get() + 1);
            (index < self.row_count()).then(|| ForegroundAppRow {
                name: format!("foreground-app-{index}.exe").into(),
                path: format!(r"C:\Apps\foreground-app-{index}.exe").into(),
            })
        }
        fn model_tracker(&self) -> &dyn slint::ModelTracker {
            &self.notify
        }
    }
    struct CountedGroups {
        reads: std::rc::Rc<std::cell::Cell<usize>>,
        notify: slint::ModelNotify,
    }
    impl Model for CountedGroups {
        type Data = ForegroundAppGroupRow;
        fn row_count(&self) -> usize {
            1000
        }
        fn row_data(&self, index: usize) -> Option<Self::Data> {
            self.reads.set(self.reads.get() + 1);
            (index < 1000).then(|| ForegroundAppGroupRow {
                id: format!("group-{index}").into(),
                name: format!("Group {index}").into(),
                count: 0,
                count_label: i18n::foreground_app_count("en", 0).into(),
                summary: "".into(),
            })
        }
        fn model_tracker(&self) -> &dyn slint::ModelTracker {
            &self.notify
        }
    }
    ui.set_mode(2);
    ui.set_page(4);
    ui.window().set_size(slint::LogicalSize::new(900., 500.));
    let groups = ui.global::<ForegroundAppUi>();
    let requested_icons =
        std::rc::Rc::new(std::cell::RefCell::new(std::collections::BTreeSet::new()));
    let requested = requested_icons.clone();
    groups.on_request_icon(move |path| {
        requested.borrow_mut().insert(path.to_string());
    });
    groups.set_saving(false);
    groups.set_confirming_delete(false);
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    groups.on_command(move |command, value| {
        observed
            .borrow_mut()
            .push((command.to_string(), value.to_string()))
    });
    for running in [false, true] {
        requested_icons.borrow_mut().clear();
        groups.set_choosing_running(running);
        let reads = std::rc::Rc::new(std::cell::Cell::new(0));
        let model = ModelRc::new(CountedForegroundApps {
            reads: reads.clone(),
            notify: Default::default(),
        });
        if running {
            groups.set_running(model);
        } else {
            groups.set_foreground_apps(model);
        }
        let started = std::time::Instant::now();
        ui.invoke_show_group_editor();
        ui.window().take_snapshot().unwrap();
        eprintln!(
            "Application list (running={running}): 1000 rows, {} row reads, {:?} initial render",
            reads.get(),
            started.elapsed()
        );
        assert!(
            reads.get() < 200,
            "Only visible application rows should be realized, got {} reads",
            reads.get()
        );
        assert!(
            requested_icons.borrow().len() < 200,
            "Icon requests must remain limited to visible rows"
        );
        let list = binding_element(
            ui,
            if running {
                "groups-running-list"
            } else {
                "groups-selected-list"
            },
        );
        scroll_list_to_end(ui, &list);
        assert!(
            reads.get() < 200,
            "Scrolling must keep the list virtualized"
        );
        let command = if running { "add" } else { "remove" };
        binding_element(
            ui,
            &format!(r"group-foreground-app-{command}-C:\Apps\foreground-app-999.exe"),
        )
        .invoke_accessible_default_action();
        assert_eq!(
            actions.borrow().last(),
            Some(&(command.into(), r"C:\Apps\foreground-app-999.exe".into()))
        );
        ui.invoke_close_group_editor();
        groups.set_running(ModelRc::default());
        groups.set_foreground_apps(ModelRc::default());
    }
    groups.set_choosing_running(false);
    let reads = std::rc::Rc::new(std::cell::Cell::new(0));
    groups.set_groups(ModelRc::new(CountedGroups {
        reads: reads.clone(),
        notify: Default::default(),
    }));
    ui.window().take_snapshot().unwrap();
    assert!(
        reads.get() < 200,
        "The group overview must also be virtualized"
    );
    scroll_list_to_end(ui, &binding_element(ui, "groups-list"));
    assert!(reads.get() < 200);
    binding_element(ui, "group-edit-group-999").invoke_accessible_default_action();
    assert_eq!(
        actions.borrow().last(),
        Some(&("edit".into(), "group-999".into()))
    );
    groups.set_groups(ModelRc::default());
}

fn scroll_list_to_end(ui: &AppWindow, list: &ElementHandle) {
    let position = slint::LogicalPosition::new(
        list.absolute_position().x + 50.,
        list.absolute_position().y + list.size().height / 2.,
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    // A virtual list refines its estimated content height as new rows appear.
    for _ in 0..3 {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position,
                delta_x: 0.,
                delta_y: -100_000.,
            });
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(300));
        slint::platform::update_timers_and_animations();
        ui.window().take_snapshot().unwrap();
    }
}
fn check_empty_foreground_app_groups(ui: &AppWindow) {
    let groups = ui.global::<ForegroundAppUi>();
    groups.set_groups(ModelRc::default());
    groups.set_selected_id("".into());
    ui.set_mode(2);
    ui.set_page(4);
    ui.set_toast("".into());
    let created = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = created.clone();
    groups.on_command(move |command, _| observed.set(command == "create"));
    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        for (width, height) in [(900., 500.), (1700., 950.)] {
            ui.window().set_size(slint::LogicalSize::new(width, height));
            ui.set_page(1);
            ui.window().take_snapshot().unwrap();
            let title = binding_element(ui, "page-title");
            let bindings_offset = binding_element(ui, "bindings-description")
                .absolute_position()
                .y
                - title.absolute_position().y;
            ui.set_page(4);
            let snapshot = ui.window().take_snapshot().unwrap();
            let create = binding_element(ui, "groups-create");
            let description = binding_element(ui, "groups-description");
            let title = binding_element(ui, "page-title");
            let groups_offset = description.absolute_position().y - title.absolute_position().y;
            assert!((groups_offset - bindings_offset).abs() < 0.5);
            assert!((create.size().height - title.size().height).abs() < 0.5);
            assert!(create.absolute_position().y < 65.);
            assert!(
                (create.absolute_position().x + create.size().width - (width - 28.)).abs() < 2.
            );
            assert!(description.absolute_position().y < 110.);
            assert!(description.size().height < 60.);
            if let Some(dir) = std::env::var_os("TAPRELAY_UI_SNAPSHOT_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join(format!("groups-empty-{locale}-{width}x{height}.rgba")),
                    snapshot.as_bytes(),
                )
                .unwrap();
            }
        }
    }
    binding_element(ui, "groups-create").invoke_accessible_default_action();
    assert!(created.get());
    ui.window().set_size(slint::LogicalSize::new(900., 500.));
}

fn check_foreground_app_groups(ui: &AppWindow) {
    let groups = ui.global::<ForegroundAppUi>();
    groups.set_groups(ModelRc::new(VecModel::from(vec![
        ForegroundAppGroupRow {
            id: "games".into(),
            name: "游戏 / Games".into(),
            count: 2,
            count_label: i18n::foreground_app_count("en", 2).into(),
            summary: "game.exe · another-game.exe".into(),
        },
        ForegroundAppGroupRow {
            id: "work".into(),
            name: "办公 / Work".into(),
            count: 0,
            count_label: i18n::foreground_app_count("en", 0).into(),
            summary: "".into(),
        },
    ])));
    ui.set_page(1);
    ui.window().take_snapshot().unwrap();
    let selected = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = selected.clone();
    let scope_rules = std::rc::Rc::new(std::cell::RefCell::new(
        taprelay_core::foreground_app::ForegroundAppRules::default(),
    ));
    let rules = scope_rules.clone();
    let window = ui.as_weak();
    groups.on_open_scope(move |function| {
        let ui = window.upgrade().unwrap();
        ui.global::<ForegroundAppUi>().set_scope_function(function);
        sync_test_scope_options(&ui, &rules.borrow());
    });
    let rules = scope_rules.clone();
    let window = ui.as_weak();
    groups.on_set_scope(move |function, group| {
        observed
            .borrow_mut()
            .push((function.to_string(), group.to_string()));
        let ui = window.upgrade().unwrap();
        let id = taprelay_core::function::FunctionId::from_stable_id(&function).unwrap();
        rules.borrow_mut().toggle_scope(id, &group);
        sync_test_scope_options(&ui, &rules.borrow());
        // Exercise the same publication path as an asynchronous runtime save.
        let rows = ui
            .global::<BindingUi>()
            .get_rows()
            .iter()
            .collect::<Vec<_>>();
        ui.global::<BindingUi>().set_rows(crate::gui::update_model(
            ui.global::<BindingUi>().get_rows(),
            rows,
        ));
    });
    binding_element(ui, "binding-scope-media.volume-up").invoke_accessible_default_action();
    ui.window().take_snapshot().unwrap();
    let choice = binding_element(ui, "scope-choice-games");
    assert!(choice.absolute_position().x >= 0.);
    assert!(choice.absolute_position().x + choice.size().width <= 900.);
    choice.invoke_accessible_default_action();
    assert_eq!(
        *selected.borrow(),
        vec![("media.volume-up".into(), "games".into())]
    );
    ui.window().take_snapshot().unwrap();
    assert_eq!(
        binding_element(ui, "scope-choice-games").accessible_checked(),
        Some(true)
    );
    assert_eq!(
        binding_element(ui, "scope-choice-all").accessible_checked(),
        Some(false)
    );
    binding_element(ui, "scope-choice-work").invoke_accessible_default_action();
    ui.window().take_snapshot().unwrap();
    assert_eq!(
        binding_element(ui, "scope-choice-games").accessible_checked(),
        Some(true)
    );
    assert_eq!(
        binding_element(ui, "scope-choice-work").accessible_checked(),
        Some(true)
    );
    save_groups_snapshot(ui, "scope-multiple");
    groups.set_busy(true);
    let before = selected.borrow().len();
    binding_element(ui, "scope-choice-all").invoke_accessible_default_action();
    assert_eq!(selected.borrow().len(), before);
    groups.set_busy(false);
    binding_element(ui, "scope-choice-all").invoke_accessible_default_action();
    ui.window().take_snapshot().unwrap();
    assert_eq!(
        binding_element(ui, "scope-choice-all").accessible_checked(),
        Some(true)
    );
    assert_eq!(
        binding_element(ui, "scope-choice-games").accessible_checked(),
        Some(false)
    );
    assert_eq!(
        binding_element(ui, "scope-choice-work").accessible_checked(),
        Some(false)
    );
    binding_element(ui, "scope-choice-work").invoke_accessible_default_action();
    binding_element(ui, "scope-choice-work").invoke_accessible_default_action();
    ui.window().take_snapshot().unwrap();
    assert_eq!(
        binding_element(ui, "scope-choice-all").accessible_checked(),
        Some(true)
    );
    ui.set_toast("".into());
    ui.set_mode(2);
    ui.set_page(4);
    ui.window().set_size(slint::LogicalSize::new(900., 500.));
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    let window = ui.as_weak();
    groups.on_command(move |command, value| {
        observed
            .borrow_mut()
            .push((command.to_string(), value.to_string()));
        if let Some(ui) = window.upgrade() {
            match command.as_str() {
                "running" => ui.global::<ForegroundAppUi>().set_choosing_running(true),
                "show-selected" => ui.global::<ForegroundAppUi>().set_choosing_running(false),
                _ => {}
            }
        }
    });
    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        for dark in [false, true] {
            let rows = groups
                .get_groups()
                .iter()
                .map(|mut row| {
                    row.count_label = i18n::foreground_app_count(locale, row.count as usize).into();
                    row
                })
                .collect::<Vec<_>>();
            groups.set_groups(ModelRc::new(VecModel::from(rows)));
            ui.global::<Theme>().set_mode(if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            });
            ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(300));
            slint::platform::update_timers_and_animations();
            for id in ["groups-create", "group-edit-games", "group-delete-games"] {
                let button = binding_element(ui, id);
                assert!(button.absolute_position().x + button.size().width <= 900.);
                assert!(button.absolute_position().y + button.size().height <= 500.);
            }
            save_groups_snapshot(ui, &format!("groups-list-{locale}-{dark}"));
            assert_eq!(
                binding_element(ui, "group-count-games")
                    .accessible_label()
                    .as_deref(),
                Some(i18n::foreground_app_count(locale, 2).as_str())
            );
            for (id, key) in [
                ("group-edit-games", "groups.edit"),
                ("group-delete-games", "groups.delete"),
            ] {
                let button = binding_element(ui, id);
                assert!((button.size().width - button.size().height).abs() < 1.);
                assert_eq!(
                    button.accessible_label().as_deref(),
                    Some(i18n::text(locale, key).as_str())
                );
            }
            groups.set_selected_id("".into());
            groups.set_selected_name("".into());
            groups.set_foreground_apps(ModelRc::default());
            groups.set_form_error("".into());
            groups.set_choosing_running(true);
            ui.invoke_show_group_editor();
            ui.window().take_snapshot().unwrap();
            let panel = binding_element(ui, "groups-editor");
            assert!(panel.absolute_position().x >= 0. && panel.absolute_position().y >= 0.);
            assert!(panel.absolute_position().y + panel.size().height <= 500.);
            save_groups_snapshot(ui, &format!("groups-create-{locale}-{dark}"));
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "G".into() });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "G".into() });
            assert_eq!(
                groups.get_selected_name(),
                "G",
                "The name input should receive initial focus"
            );
            assert_eq!(
                binding_element(ui, "page-title")
                    .accessible_label()
                    .as_deref(),
                Some(i18n::text(locale, "nav.groups").as_str())
            );
            assert_eq!(
                binding_element(ui, "groups-subpage-title")
                    .accessible_label()
                    .as_deref(),
                Some(i18n::text(locale, "groups.new_title").as_str())
            );
            binding_element(ui, "groups-browse");
            binding_element(ui, "groups-running-list");
            assert!(
                ui.root_element()
                    .query_descendants()
                    .match_predicate(|element| element.accessible_id().as_deref()
                        == Some("groups-selected-list"))
                    .find_first()
                    .is_none()
            );
            binding_element(ui, "groups-editor-save").invoke_accessible_default_action();
            assert_eq!(actions.borrow().last(), Some(&("save".into(), "".into())));
            groups.set_form_error(i18n::text(locale, "groups.conflict").into());
            ui.window().take_snapshot().unwrap();
            let error = binding_element(ui, "groups-editor-error");
            let save = binding_element(ui, "groups-editor-save");
            assert!(error.absolute_position().y >= save.absolute_position().y - 20.);
            assert!(error.absolute_position().x + error.size().width <= save.absolute_position().x);
            assert!((error.absolute_position().x - panel.absolute_position().x).abs() < 1.);
            save_groups_snapshot(ui, &format!("groups-error-{locale}-{dark}"));
            groups.set_form_error("".into());
            groups.set_selected_id("games".into());
            groups.set_selected_name("游戏 / Games".into());
            groups.set_choosing_running(false);
            groups.set_foreground_apps(ModelRc::new(VecModel::from(vec![ForegroundAppRow {
                name: "game.exe".into(),
                path: r"C:\Games\game.exe".into(),
            }])));
            save_groups_snapshot(ui, &format!("groups-edit-{locale}-{dark}"));
            ui.set_page(1);
            ui.window().take_snapshot().unwrap();
            ui.set_page(4);
            ui.window().take_snapshot().unwrap();
            assert_eq!(groups.get_selected_name(), "游戏 / Games");
            assert_eq!(groups.get_foreground_apps().row_count(), 1);
            let selected_tab = binding_element(ui, "groups-tab-selected");
            let running_tab = binding_element(ui, "groups-tab-running");
            assert!((selected_tab.size().width - running_tab.size().width).abs() < 0.5);
            assert!(
                selected_tab.absolute_position().x + selected_tab.size().width
                    <= running_tab.absolute_position().x + 0.5
            );
            assert!(
                (selected_tab.absolute_position().y - running_tab.absolute_position().y).abs()
                    < 0.5
            );
            running_tab.invoke_accessible_default_action();
            assert!(groups.get_choosing_running());
            ui.window().take_snapshot().unwrap();
            binding_element(ui, "groups-browse");
            assert_eq!(
                selected_tab.accessible_label().as_deref(),
                Some(format!("{} (1)", i18n::text(locale, "groups.selected")).as_str())
            );
            selected_tab.invoke_accessible_default_action();
            assert!(!groups.get_choosing_running());
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::RightArrow.into(),
                });
            assert!(groups.get_choosing_running());
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::LeftArrow.into(),
                });
            assert!(!groups.get_choosing_running());
            assert_eq!(groups.get_selected_name(), "游戏 / Games");
            assert_eq!(groups.get_foreground_apps().row_count(), 1);
            running_tab.invoke_accessible_default_action();
            groups.set_running(ModelRc::new(VecModel::from(vec![ForegroundAppRow {
                name: "browser.exe".into(),
                path: r"C:\Apps\browser.exe".into(),
            }])));
            ui.window().take_snapshot().unwrap();
            binding_element(ui, r"group-foreground-app-add-C:\Apps\browser.exe")
                .invoke_accessible_default_action();
            assert_eq!(
                actions.borrow().last(),
                Some(&("add".into(), r"C:\Apps\browser.exe".into()))
            );
            save_groups_snapshot(ui, &format!("groups-picker-{locale}-{dark}"));
            groups.set_choosing_running(false);
            groups.set_saving(true);
            let count = actions.borrow().len();
            ui.window().take_snapshot().unwrap();
            binding_element(ui, "groups-editor-save").invoke_accessible_default_action();
            binding_element(ui, "groups-editor-back").invoke_accessible_default_action();
            binding_element(ui, "groups-tab-running").invoke_accessible_default_action();
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::Escape.into(),
                });
            assert_eq!(actions.borrow().len(), count);
            groups.set_saving(false);
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::Escape.into(),
                });
            assert_eq!(actions.borrow().last(), Some(&("cancel".into(), "".into())));
            let count = actions.borrow().len();
            binding_element(ui, "groups-editor-back").invoke_accessible_default_action();
            assert_eq!(actions.borrow().len(), count + 1);
            assert_eq!(actions.borrow().last(), Some(&("cancel".into(), "".into())));
            assert!(
                ui.root_element()
                    .query_descendants()
                    .match_predicate(|element| element.accessible_id().as_deref()
                        == Some("groups-editor-cancel"))
                    .find_first()
                    .is_none()
            );
            ui.invoke_close_group_editor();
        }
        groups.set_confirming_delete(true);
        groups.set_deletion_warning(
            format!(
                "{}\nPlay / Pause, Volume up",
                i18n::text(locale, "groups.delete_warning")
            )
            .into(),
        );
        ui.invoke_show_group_delete();
        ui.window().take_snapshot().unwrap();
        binding_element(ui, "groups-delete-confirm").invoke_accessible_default_action();
        assert_eq!(actions.borrow().last(), Some(&("delete".into(), "".into())));
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: slint::platform::Key::Escape.into(),
            });
        assert_eq!(actions.borrow().last(), Some(&("cancel".into(), "".into())));
        ui.invoke_close_group_delete();
        groups.set_confirming_delete(false);
    }
    binding_element(ui, "group-edit-work").invoke_accessible_default_action();
    assert_eq!(
        actions.borrow().last(),
        Some(&("edit".into(), "work".into()))
    );
}

fn sync_test_scope_options(
    ui: &AppWindow,
    rules: &taprelay_core::foreground_app::ForegroundAppRules,
) {
    let view = ui.global::<ForegroundAppUi>();
    let id =
        taprelay_core::function::FunctionId::from_stable_id(&view.get_scope_function()).unwrap();
    let selected = rules.assignments.get(&id);
    let mut options = vec![ForegroundAppScopeOption {
        id: "".into(),
        name: "All applications".into(),
        checked: selected.is_none(),
    }];
    options.extend(
        view.get_groups()
            .iter()
            .map(|group| ForegroundAppScopeOption {
                checked: selected.is_some_and(|ids| ids.contains(group.id.as_str())),
                id: group.id,
                name: group.name,
            }),
    );
    view.set_scope_options(crate::gui::update_model(view.get_scope_options(), options));
}

fn save_groups_snapshot(ui: &AppWindow, name: &str) {
    let snapshot = ui.window().take_snapshot().unwrap();
    if let Some(dir) = std::env::var_os("TAPRELAY_UI_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{name}-900x500.rgba")),
            snapshot.as_bytes(),
        )
        .unwrap();
    }
}
fn check_runtime_feedback(ui: &AppWindow) {
    ui.set_mode(2);
    ui.set_page(0);
    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        for key in [
            i18n::keys::RUNTIME_BUSY,
            i18n::keys::RUNTIME_SLOW,
            i18n::keys::RUNTIME_FINISHING,
        ] {
            let text = i18n::text(locale, key);
            ui.set_toast(text.clone().into());
            assert_eq!(ui.get_toast().as_str(), text);
            let snapshot = ui.window().take_snapshot().unwrap();
            let width = snapshot.width();
            let height = snapshot.height();
            if key == i18n::keys::RUNTIME_SLOW
                && let Some(directory) = std::env::var_os("TAPRELAY_UI_RENDER_DIR")
            {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
                for pixel in snapshot.as_slice() {
                    ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(directory.join(format!("runtime-{locale}.ppm")), ppm).unwrap();
            }
        }
    }
}

fn check_environment_failure(ui: &AppWindow) {
    ui.set_mode(3);
    ui.set_busy(false);
    ui.set_problem("Invalid configuration; move config.json aside to start fresh: unknown field `passthrough_reverse_scroll`, expected one of `schema`, `functions`, `remembered_device`, `wizard`, `options`, `window` at line 103 column 30".into());
    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        ui.set_checks(ModelRc::new(VecModel::from(crate::startup_checks::rows(
            locale, true, [false; 4], true,
        ))));
        for dark in [false, true] {
            ui.global::<Theme>().set_mode(if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            });
            for (width, height) in [(900., 500.), (1120., 700.)] {
                ui.window().set_size(slint::LogicalSize::new(width, height));
                let snapshot = ui.window().take_snapshot().unwrap();
                let quit = binding_element(ui, "environment-quit");
                assert!(quit.absolute_position().y + quit.size().height <= height);
                if let Some(dir) = std::env::var_os("TAPRELAY_UI_SNAPSHOT_DIR") {
                    let dir = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(
                        dir.join(format!(
                            "environment-{locale}-{dark}-{}x{}.rgba",
                            snapshot.width(),
                            snapshot.height()
                        )),
                        snapshot.as_bytes(),
                    )
                    .unwrap();
                }
            }
        }
    }
    ui.set_problem("".into());
}

fn check_about_links(ui: &AppWindow) {
    use slint::platform::{PointerEventButton, WindowEvent};
    ui.set_mode(2);
    ui.set_page(3);
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    ui.on_action(move |name, value| {
        observed
            .borrow_mut()
            .push((name.to_string(), value.to_string()));
    });
    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        for dark in [false, true] {
            ui.global::<Theme>().set_mode(if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            });
            for (width, height) in [(900., 500.), (1120., 700.)] {
                ui.window().set_size(slint::LogicalSize::new(width, height));
                let _ = ui.window().take_snapshot().unwrap();
                ui.window().dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(500., 350.),
                    delta_x: 0.,
                    delta_y: -3000.,
                });
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
                slint::platform::update_timers_and_animations();
                let snapshot = ui.window().take_snapshot().unwrap();
                if let Some(dir) = std::env::var_os("TAPRELAY_UI_SNAPSHOT_DIR") {
                    let dir = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(
                        dir.join(format!(
                            "about-{locale}-{dark}-{}x{}.rgba",
                            snapshot.width(),
                            snapshot.height()
                        )),
                        snapshot.as_bytes(),
                    )
                    .unwrap();
                }
                let license = binding_element(ui, "settings-license");
                let notices = binding_element(ui, "settings-third-party-notices");
                let repository = binding_element(ui, "settings-repository");
                assert!(
                    license.absolute_position().x + license.size().width
                        <= notices.absolute_position().x
                );
                assert_eq!(license.absolute_position().y, notices.absolute_position().y);
                assert!(
                    license.absolute_position().y + license.size().height
                        < repository.absolute_position().y
                );
                for (command, label_key) in [
                    ("repository", i18n::keys::SETTINGS_REPOSITORY),
                    ("license", i18n::keys::SETTINGS_LICENSE),
                    (
                        "third-party-notices",
                        i18n::keys::SETTINGS_THIRD_PARTY_NOTICES,
                    ),
                ] {
                    let button = binding_element(ui, &format!("settings-{command}"));
                    assert_eq!(
                        button.accessible_label().as_deref(),
                        Some(i18n::text(locale, label_key).as_str())
                    );
                    let origin = button.absolute_position();
                    let size = button.size();
                    assert!(origin.x >= 0. && origin.x + size.width <= width);
                    assert!(origin.y >= 0. && origin.y + size.height <= height);
                    actions.borrow_mut().clear();
                    let position = slint::LogicalPosition::new(
                        origin.x + size.width / 2.,
                        origin.y + size.height / 2.,
                    );
                    ui.window()
                        .dispatch_event(WindowEvent::PointerMoved { position });
                    ui.window().dispatch_event(WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    });
                    ui.window().dispatch_event(WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    });
                    button.invoke_accessible_default_action();
                    assert_eq!(*actions.borrow(), vec![(command.into(), String::new()); 2]);
                    assert!(crate::action::Action::try_from(command).is_ok());
                }
            }
        }
    }
}

fn render_pages(ui: &AppWindow) {
    ui.set_device_name("Living room tablet".into());
    ui.set_device_selected(true);
    ui.set_stage("Connected; waiting for HID subscription".into());
    ui.set_bindings(ModelRc::new(VecModel::from(vec![
        BindingRow {
            text: "F8".into(),
            function_name: "Play / pause".into(),
            keys: ModelRc::new(VecModel::from(vec!["F8".into()])),
            enabled: true,
        },
        BindingRow {
            text: "LCtrl + ]".into(),
            function_name: "Next track".into(),
            keys: ModelRc::new(VecModel::from(vec!["LCtrl".into(), "]".into()])),
            enabled: true,
        },
    ])));
    ui.set_paired_devices(ModelRc::new(VecModel::from(vec![DeviceRow {
        name: "Living room tablet".into(),
        detail: "Connected; preparing media controls".into(),
        selected: true,
        paired: true,
        action: "device".into(),
        action_label: "Connect".into(),
        ..Default::default()
    }])));
    ui.set_checks(ModelRc::new(VecModel::from(vec![
        CheckRow {
            title: "Environment".into(),
            state: 2,
        },
        CheckRow {
            title: "Bluetooth available".into(),
            state: 2,
        },
        CheckRow {
            title: "BLE peripheral support".into(),
            state: 2,
        },
        CheckRow {
            title: "HID service".into(),
            state: 1,
        },
        CheckRow {
            title: "Advertising".into(),
            state: 0,
        },
    ])));
    ui.set_language_options(i18n::language_options("en"));
    ui.set_data_directory("D:\\Apps\\TapRelay".into());
    ui.set_app_version(version::VERSION.into());
    for (locale, dark, width, height) in [("en", true, 1000., 700.), ("zh-cn", false, 800., 560.)] {
        i18n::apply(ui, locale);
        let global = ui.global::<I18n>();
        assert_eq!(global.get_locale(), locale);
        assert_eq!(
            global.get_text().nav_overview,
            i18n::text(locale, i18n::keys::NAV_OVERVIEW)
        );
        ui.global::<Theme>().set_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        ui.set_bluetooth_available(dark);
        ui.window().set_size(slint::LogicalSize::new(width, height));
        for mode in 0..3 {
            ui.set_mode(mode);
            for page in 0..if mode == 0 { 1 } else { 4 } {
                ui.set_page(page);
                ui.set_wizard_page(page.min(3));
                ui.set_problem(
                    if !dark && ((mode == 1 && page == 2) || (mode == 2 && page == 2)) {
                        "Bluetooth service unavailable".into()
                    } else {
                        "".into()
                    },
                );
                ui.show().unwrap();
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
                let _ = ui.window().take_snapshot().unwrap();
            }
        }
    }
}

fn check_binding_capture(ui: &AppWindow) {
    ui.window().set_size(slint::LogicalSize::new(900., 500.));
    ui.set_mode(1);
    ui.set_wizard_page(1);
    let shortcut = ShortcutItem {
        text: "F8".into(),
        keys: ModelRc::new(VecModel::from(vec!["F8".into()])),
        slot: 0,
        enabled: true,
    };
    let row = FunctionBindingRow {
        scope_label: "All applications".into(),
        scope_editable: true,
        id: "media.play-pause".into(),
        label: "Play / Pause".into(),
        gestures: ModelRc::new(VecModel::from(vec![GestureAction {
            gesture: "Tap".into(),
            action: "Play / Pause".into(),
        }])),
        enabled: true,
        shortcuts: ModelRc::new(VecModel::from(vec![shortcut])),
    };
    set_function_bindings(ui, vec![row.clone()]);
    set_binding_capture(
        ui,
        "media.play-pause",
        0,
        "请按下快捷键，松开完成",
        "无效组合",
    );
    let _ = ui.window().take_snapshot().unwrap();
    let error = binding_element(ui, "binding-error-media.play-pause");
    assert_eq!(
        error.accessible_live_region(),
        Some(i_slint_backend_testing::AccessibleLiveness::Polite)
    );
    assert_eq!(error.accessible_label().as_deref(), Some("无效组合"));
    let captured_keys = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = captured_keys.clone();
    ui.global::<BindingUi>()
        .on_key_input(move |text, down| observed.borrow_mut().push((text.to_string(), down)));
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = cancelled.clone();
    ui.global::<BindingUi>()
        .on_cancel_capture(move || observed.set(true));
    let mut empty_row = row.clone();
    empty_row.shortcuts = ModelRc::new(VecModel::default());
    set_function_bindings(ui, vec![empty_row]);
    set_binding_capture(ui, "media.play-pause", 0, "请按下快捷键，松开完成", "");
    let _ = ui.window().take_snapshot().unwrap();
    for key in [
        slint::platform::Key::F8.into(),
        slint::SharedString::from("k"),
    ] {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: key.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: key });
    }
    let recorded: Vec<_> = captured_keys
        .borrow()
        .iter()
        .map(|(text, down)| (crate::capture_key::virtual_key(text), *down))
        .collect();
    assert_eq!(
        recorded,
        vec![
            (Some(0x77), true),
            (Some(0x77), false),
            (Some(0x4b), true),
            (Some(0x4b), false)
        ]
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(
        cancelled.get(),
        "An empty slot must receive Escape without an extra click"
    );
    set_binding_capture(ui, "", -1, "", "");
}

fn check_navigation_and_settings(ui: &AppWindow) {
    // Exercise real pointer routing through the tooltip wrapper, not just callback invocation.
    i18n::apply(ui, "en");
    ui.set_mode(2);
    ui.set_page(0);
    ui.window().set_size(slint::LogicalSize::new(1000., 700.));
    let _ = ui.window().take_snapshot().unwrap();
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let a = actions.clone();
    ui.on_action(move |name, value| a.borrow_mut().push((name.to_string(), value.to_string())));
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(30., 670.);
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(70., 500.),
    });
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(
        actions.borrow().contains(&("navigate".into(), "3".into())),
        "Sidebar tooltip must not consume navigation clicks"
    );
    ui.set_page(3);
    let _ = ui.window().take_snapshot().unwrap();

    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        ui.set_passthrough_overlay(true);
        let _ = ui.window().take_snapshot().unwrap();
        actions.borrow_mut().clear();
        binding_element(ui, "setting-passthrough-overlay").invoke_accessible_default_action();
        assert_eq!(
            *actions.borrow(),
            vec![("set-passthrough-overlay".into(), "0".into())]
        );
        for hz in taprelay_core::passthrough::MOUSE_REPORT_RATES {
            ui.set_passthrough_mouse_report_rate(i32::from(hz));
            let snapshot = ui.window().take_snapshot().unwrap();
            actions.borrow_mut().clear();
            let button = binding_element(ui, &format!("setting-mouse-report-rate-{hz}"));
            assert!(button.absolute_position().x + button.size().width <= 1000.);
            button.invoke_accessible_default_action();
            assert_eq!(
                *actions.borrow(),
                vec![("set-passthrough-mouse-report-rate".into(), hz.to_string())]
            );
            if hz == 125
                && let Some(dir) = std::env::var_os("TAPRELAY_UI_RENDER_DIR")
            {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let mut ppm =
                    format!("P6\n{} {}\n255\n", snapshot.width(), snapshot.height()).into_bytes();
                for pixel in snapshot.as_slice() {
                    ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(dir.join(format!("mouse-report-rate-{locale}.ppm")), ppm).unwrap();
            }
        }
    }
    i18n::apply(ui, "en");

    // The remaining log UI opens the on-disk directory from Settings.
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(500., 500.),
        delta_x: 0.,
        delta_y: -2000.,
    });
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
    let _ = ui.window().take_snapshot().unwrap();
    let position = slint::LogicalPosition::new(260., 654.);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(
        actions
            .borrow()
            .contains(&("logs-folder".into(), "".into()))
    );
    let _ = ui.window().take_snapshot().unwrap();

    // Every settings switch owns a named command, so one control can never be
    // wired to another's setting the way the shared "setting" id allowed.
    // Clipped rows leave the accessibility tree, so grow the viewport first.
    ui.window().set_size(slint::LogicalSize::new(1000., 1100.));
    let _ = ui.window().take_snapshot().unwrap();
    for (accessible_id, command) in [
        ("setting-autostart", "set-autostart"),
        ("setting-start-hidden", "set-start-hidden"),
        ("setting-auto-listen", "set-auto-listen"),
        ("setting-close-to-tray", "set-close-to-tray"),
        ("setting-always-admin", "set-always-admin"),
        (
            "setting-connection-wait-warning",
            "set-connection-wait-warning",
        ),
        ("setting-notifications", "set-notifications"),
    ] {
        actions.borrow_mut().clear();
        let switch = binding_element(ui, accessible_id);
        switch.invoke_accessible_default_action();
        switch.invoke_accessible_default_action();
        let emitted = actions.borrow().clone();
        assert_eq!(
            emitted.len(),
            2,
            "{accessible_id} must send one command per toggle"
        );
        assert!(
            emitted.iter().all(|(name, _)| name == command),
            "{accessible_id} must send {command}, got {emitted:?}"
        );
        assert!(
            crate::action::Action::try_from(command).is_ok(),
            "{command} must also be a known action id in src/action.rs"
        );
        assert_eq!(
            emitted[0].1,
            if emitted[1].1 == "1" { "0" } else { "1" },
            "{command} must carry the switch's new state"
        );
    }
    ui.window().set_size(slint::LogicalSize::new(1000., 700.));
    let _ = ui.window().take_snapshot().unwrap();
}

fn check_surface_redraw(
    ui: &AppWindow,
    render_window: &slint::platform::software_renderer::MinimalSoftwareWindow,
) {
    // Seed the retained renderer cache and confirm unchanged UI has no damage.
    use slint::platform::software_renderer::PremultipliedRgbaColor;
    let mut frame = vec![PremultipliedRgbaColor::default(); 1000 * 700];
    ui.window().request_redraw();
    let mut first_region = None;
    assert!(render_window.draw_if_needed(|renderer| {
        first_region = Some(renderer.render(frame.as_mut_slice(), 1000));
    }));
    assert_eq!(
        first_region.unwrap().bounding_box_size(),
        slint::PhysicalSize::new(1000, 700)
    );

    ui.window().request_redraw();
    let mut unchanged_region = None;
    assert!(render_window.draw_if_needed(|renderer| {
        unchanged_region = Some(renderer.render(frame.as_mut_slice(), 1000));
    }));
    assert_eq!(
        unchanged_region.unwrap().bounding_box_size(),
        slint::PhysicalSize::default()
    );

    #[cfg(windows)]
    {
        let expected = frame.clone();
        // Model a lost surface, without minimizing or changing any UI property.
        // Exercise the same invalidation used before native RedrawRequested.
        for _ in 0..3 {
            frame.fill(PremultipliedRgbaColor::default());
            crate::window_rendering::invalidate_surface(ui.window());
            ui.window().request_redraw();
            assert!(render_window.draw_if_needed(|renderer| {
                let region = renderer.render(frame.as_mut_slice(), 1000);
                assert_eq!(
                    region.bounding_box_origin(),
                    slint::PhysicalPosition::default()
                );
                assert_eq!(
                    region.bounding_box_size(),
                    slint::PhysicalSize::new(1000, 700)
                );
            }));
            assert!(
                frame
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| (a.red, a.green, a.blue, a.alpha)
                        == (b.red, b.green, b.blue, b.alpha)),
                "expose must recover every pixel without UI changes"
            );
            assert!(
                !render_window.draw_if_needed(|_| panic!("expose must not create a redraw loop"))
            );
        }
    }
}

fn check_binding_layouts_and_actions(ui: &AppWindow) {
    use taprelay_core::function::FUNCTION_CATALOG;
    set_binding_capture(ui, "", -1, "", "");
    ui.set_problem("".into());
    for (locale, dark, width, height) in [
        ("zh-cn", true, 900., 500.),
        ("en", false, 900., 500.),
        ("zh-cn", false, 1120., 700.),
        ("en", true, 1120., 700.),
    ] {
        i18n::apply(ui, locale);
        ui.global::<Theme>().set_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        ui.window().set_size(slint::LogicalSize::new(width, height));
        let rows: Vec<_> = FUNCTION_CATALOG
            .iter()
            .enumerate()
            .map(|(i, d)| FunctionBindingRow {
                scope_label: i18n::text(locale, "groups.all").into(),
                scope_editable: d.category == taprelay_core::function::CategoryId::Media,
                id: d.id.stable_id().into(),
                label: i18n::text(locale, d.name_key).into(),
                gestures: ModelRc::new(VecModel::from({
                    let mut gestures = vec![GestureAction {
                        gesture: i18n::text(locale, "bindings.gesture.press").into(),
                        action: i18n::text(locale, d.tap_action.name_key()).into(),
                    }];
                    if let Some(hold) = d.hold_action {
                        gestures.push(GestureAction {
                            gesture: i18n::text(locale, "bindings.gesture.hold").into(),
                            action: i18n::text(locale, hold.name_key()).into(),
                        });
                    }
                    if d.repeats() {
                        gestures.push(GestureAction {
                            gesture: i18n::text(locale, "bindings.gesture.hold").into(),
                            action: i18n::text(locale, "bindings.gesture.repeat").into(),
                        });
                    }
                    gestures
                })),
                enabled: true,
                shortcuts: ModelRc::new(VecModel::from(
                    (0..(2 - i % 3))
                        .map(|slot| ShortcutItem {
                            text: if slot == 0 {
                                "Ctrl + Shift + Alt + Win + PageDown"
                            } else {
                                "Ctrl + Shift + Alt + Mouse Button 5"
                            }
                            .into(),
                            slot: slot as i32,
                            enabled: true,
                            ..Default::default()
                        })
                        .collect::<Vec<_>>(),
                )),
            })
            .collect();
        let mut rows = rows;
        rows[3].enabled = false;
        rows[2].enabled = false;
        set_function_bindings(ui, rows);
        ui.set_mode(2);
        ui.set_page(1);
        let _ = ui.window().take_snapshot().unwrap();
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
        slint::platform::update_timers_and_animations();
        let _ = ui.window().take_snapshot().unwrap();
        let function_column = binding_element(ui, "binding-function-media.play-pause");
        let scope_header = binding_element(ui, "bindings-column-scope");
        assert_eq!(
            scope_header.accessible_label().as_deref(),
            Some(i18n::text(locale, "groups.scope").as_str())
        );
        for id in [
            "media.play-pause",
            "media.previous",
            "media.next",
            "media.mute",
        ] {
            let function = binding_element(ui, &format!("binding-function-{id}"));
            let scope = binding_element(ui, &format!("binding-scope-{id}"));
            assert!(
                function.absolute_position().x + function.size().width
                    <= scope.absolute_position().x,
                "Scope must follow the function in its own rightmost column"
            );
            assert!((scope.absolute_position().x - scope_header.absolute_position().x).abs() < 1.);
            assert!(scope.absolute_position().x + scope.size().width <= width);
            assert!(
                (scope.absolute_position().y + scope.size().height / 2.
                    - function.absolute_position().y
                    - function.size().height / 2.)
                    .abs()
                    < 1.,
                "Scope buttons must be vertically centered in the function row"
            );
        }
        save_groups_snapshot(ui, &format!("bindings-scope-{locale}-{width}"));
        for slot in 0..2 {
            let shortcut = binding_element(ui, &format!("binding-slot-media.play-pause-{slot}"));
            assert!(
                shortcut.absolute_position().x + shortcut.size().width
                    <= function_column.absolute_position().x,
                "Long shortcut labels must not invade the trigger column"
            );
        }
    }
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    ui.global::<BindingUi>()
        .on_begin_capture(move |function, slot| {
            observed
                .borrow_mut()
                .push(BindingUiAction::Begin(function.to_string(), slot));
        });
    let observed = actions.clone();
    ui.global::<BindingUi>()
        .on_delete_shortcut(move |function, slot| {
            observed
                .borrow_mut()
                .push(BindingUiAction::Delete(function.to_string(), slot));
        });
    let observed = actions.clone();
    ui.global::<BindingUi>()
        .on_set_function_enabled(move |function, enabled| {
            observed
                .borrow_mut()
                .push(BindingUiAction::Toggle(function.to_string(), enabled));
        });
    ui.set_mode(2);
    ui.set_page(1);
    let _ = ui.window().take_snapshot().unwrap();
    let disabled_second = binding_element(ui, "binding-disabled-slot-media.mute-1");
    let function_column = binding_element(ui, "binding-function-media.mute");
    assert!(
        disabled_second.absolute_position().x + disabled_second.size().width
            <= function_column.absolute_position().x,
        "Disabled shortcut chips must stay inside the shortcut column"
    );
    binding_element(ui, "binding-slot-media.play-pause-1").invoke_accessible_default_action();
    binding_element(ui, "binding-add-media.previous").invoke_accessible_default_action();
    assert_eq!(
        *actions.borrow(),
        vec![
            BindingUiAction::Begin("media.play-pause".into(), 1),
            BindingUiAction::Begin("media.previous".into(), 1),
        ],
        "Both an existing second shortcut and the add-second control must be interactive"
    );
    assert_eq!(
        ui.global::<BindingUi>()
            .get_rows()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2,
        "Presenting one shortcut must not truncate the multi-shortcut model"
    );
    actions.borrow_mut().clear();
    binding_element(ui, "binding-delete-media.play-pause-0").invoke_accessible_default_action();
    assert!(
        actions
            .borrow()
            .contains(&BindingUiAction::Delete("media.play-pause".into(), 0)),
        "The shortcut delete button must expose its default accessibility action"
    );
    assert!(
        !actions
            .borrow()
            .iter()
            .any(|action| matches!(action, BindingUiAction::Begin(_, _))),
        "Deleting a shortcut must not start recording"
    );
    actions.borrow_mut().clear();
    let enabled_switch = binding_element(ui, "binding-enabled-media.play-pause");
    assert_eq!(
        enabled_switch.accessible_role(),
        Some(i_slint_backend_testing::AccessibleRole::Switch)
    );
    enabled_switch.invoke_accessible_default_action();
    binding_element(ui, "binding-enabled-media.next").invoke_accessible_default_action();
    assert_eq!(
        *actions.borrow(),
        vec![
            BindingUiAction::Toggle("media.play-pause".into(), false),
            BindingUiAction::Toggle("media.next".into(), true),
        ],
        "Switches must disable active functions and enable disabled unbound functions"
    );
    assert_eq!(
        ui.global::<BindingUi>().get_rows().row_count(),
        FUNCTION_CATALOG.len()
    );
    assert_eq!(
        ui.global::<BindingUi>()
            .get_rows()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2
    );
    set_binding_capture(ui, "media.play-pause", 0, "Recording", "");
    let _ = ui.window().take_snapshot().unwrap();
    actions.borrow_mut().clear();
    let disabled_switch = binding_element(ui, "binding-enabled-media.next");
    assert_eq!(disabled_switch.accessible_enabled(), Some(false));
    disabled_switch.invoke_accessible_default_action();
    assert!(
        actions.borrow().is_empty(),
        "Recording must lock function switches"
    );
    set_binding_capture(ui, "", -1, "", "");
    ui.window().set_size(slint::LogicalSize::new(1120., 900.));
    let _ = ui.window().take_snapshot().unwrap();
    let app = ui
        .global::<BindingUi>()
        .get_rows()
        .iter()
        .find(|row| row.id.as_str() == "app.toggle-listening")
        .unwrap();
    assert_eq!(app.id.as_str(), "app.toggle-listening");
    actions.borrow_mut().clear();
    binding_element(ui, "binding-enabled-app.toggle-listening").invoke_accessible_default_action();
    binding_element(ui, "binding-slot-app.toggle-listening-0").invoke_accessible_default_action();
    assert_eq!(
        *actions.borrow(),
        vec![
            BindingUiAction::Toggle("app.toggle-listening".into(), false),
            BindingUiAction::Begin("app.toggle-listening".into(), 0),
        ]
    );
}

fn check_volume_bindings(ui: &AppWindow) {
    use taprelay_core::{
        function::{FunctionId, ModifierSet, PrimaryInput, Shortcut},
        input::MouseButton,
    };
    for locale in ["zh-cn", "en"] {
        i18n::apply(ui, locale);
        ui.window().set_size(slint::LogicalSize::new(900., 500.));
        let rows = [
            (FunctionId::MediaVolumeUp, PrimaryInput::keyboard(0x26)),
            (
                FunctionId::MediaVolumeDown,
                PrimaryInput::mouse(MouseButton::Side1),
            ),
        ]
        .into_iter()
        .map(|(id, primary)| {
            let definition = taprelay_core::function::function_definition(id);
            let shortcut = Shortcut::new(ModifierSet::from_keys([0x11]), primary);
            FunctionBindingRow {
                scope_label: i18n::text(locale, "groups.all").into(),
                scope_editable: true,
                id: id.stable_id().into(),
                label: i18n::text(locale, definition.name_key).into(),
                enabled: true,
                shortcuts: ModelRc::new(VecModel::from(vec![ShortcutItem {
                    text: shortcut
                        .key_labels_with(crate::platform::key_name)
                        .join("+")
                        .into(),
                    slot: 0,
                    enabled: true,
                    ..Default::default()
                }])),
                gestures: ModelRc::new(VecModel::from(vec![
                    GestureAction {
                        gesture: i18n::text(locale, "bindings.gesture.press").into(),
                        action: i18n::text(locale, definition.tap_action.name_key()).into(),
                    },
                    GestureAction {
                        gesture: i18n::text(locale, "bindings.gesture.hold").into(),
                        action: i18n::text(locale, "bindings.gesture.repeat").into(),
                    },
                ])),
            }
        })
        .collect();
        set_function_bindings(ui, rows);
        let snapshot = ui.window().take_snapshot().unwrap();
        for id in ["media.volume-up", "media.volume-down"] {
            let shortcut = binding_element(ui, &format!("binding-slot-{id}-0"));
            let action = binding_element(ui, &format!("binding-function-{id}"));
            assert!(
                shortcut.absolute_position().x + shortcut.size().width
                    <= action.absolute_position().x
            );
            assert!(action.absolute_position().y + action.size().height <= 500.);
        }
        if let Some(dir) = std::env::var_os("TAPRELAY_UI_SNAPSHOT_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("volume-{locale}-900x500.rgba")),
                snapshot.as_bytes(),
            )
            .unwrap();
        }
    }
}
