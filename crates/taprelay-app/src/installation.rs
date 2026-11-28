pub(crate) fn run_hooks() {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut app = velopack::VelopackApp::build().set_auto_apply_on_startup(false);
    #[cfg(windows)]
    {
        app = app.on_before_uninstall_fast_callback(|_| {
            if let Err(error) = remove_installed_autostart() {
                eprintln!("TapRelay uninstall: {error:#}");
                std::process::exit(1);
            }
        });
    }
    app.run();
}

#[cfg(windows)]
pub(crate) fn is_installed(executable: &std::path::Path) -> anyhow::Result<bool> {
    Ok(installed_manifest(executable)?.is_some())
}

#[cfg(windows)]
pub(crate) fn installed_manifest(
    executable: &std::path::Path,
) -> anyhow::Result<Option<velopack::bundle::Manifest>> {
    let Some(directory) = executable.parent() else {
        return Ok(None);
    };
    let Some(root) = directory.parent() else {
        return Ok(None);
    };
    if !directory
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("current"))
    {
        return Ok(None);
    }
    let layout = velopack::locator::create_config_from_root_dir(root);
    if layout.IsPortable || !layout.UpdateExePath.is_file() || !layout.ManifestPath.is_file() {
        return Ok(None);
    }
    let manifest = layout.load_manifest()?;
    let matches = manifest.id == "TapRelay"
        && executable
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(&manifest.main_exe));
    Ok(matches.then_some(manifest))
}

#[cfg(windows)]
fn remove_installed_autostart() -> anyhow::Result<()> {
    use crate::platform::desktop;
    if is_installed(&std::env::current_exe()?)? && desktop::autostart_enabled() {
        desktop::set_autostart(false)?;
    }
    Ok(())
}

pub(crate) fn app_directory() -> anyhow::Result<std::path::PathBuf> {
    app_directory_for(&std::env::current_exe()?)
}

fn app_directory_for(executable: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;

    #[cfg(windows)]
    if installed_manifest(executable)?.is_some() {
        return Ok(executable
            .parent()
            .and_then(|dir| dir.parent())
            .context("Missing installed application directory")?
            .to_path_buf());
    }
    #[cfg(target_os = "macos")]
    if let Some(macos_dir) = executable.parent()
        && macos_dir.file_name().is_some_and(|name| name == "MacOS")
        && let Some(contents_dir) = macos_dir.parent()
        && contents_dir
            .file_name()
            .is_some_and(|name| name == "Contents")
        && let Some(bundle_dir) = contents_dir.parent()
        && bundle_dir
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        && let Some(applications_dir) = bundle_dir.parent()
    {
        return Ok(applications_dir.to_path_buf());
    }
    Ok(executable
        .parent()
        .context("Missing executable directory")?
        .to_path_buf())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn installed_app_directory_points_to_localappdata_root_and_portable_stays_beside_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Local/TapRelay");
        let current = root.join("current");
        let installed_exe = current.join("TapRelay.exe");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(root.join("Update.exe"), []).unwrap();
        std::fs::write(
            current.join("sq.version"),
            "<package><metadata><id>TapRelay</id><version>1.2.3</version><mainExe>TapRelay.exe</mainExe></metadata></package>",
        )
        .unwrap();
        assert_eq!(app_directory_for(&installed_exe).unwrap(), root);

        let portable = tmp.path().join("Portable");
        std::fs::create_dir_all(&portable).unwrap();
        assert_eq!(
            app_directory_for(&portable.join("TapRelay.exe")).unwrap(),
            portable
        );
    }
}
