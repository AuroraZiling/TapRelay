use std::path::{Path, PathBuf};

#[path = "build/i18n.rs"]
mod i18n;

fn main() {
    assert!(
        matches!(
            std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
            Ok("windows" | "macos")
        ),
        "taprelay-app supports Windows and macOS targets"
    );
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let target = std::env::var("MACOSX_DEPLOYMENT_TARGET")
            .expect("Build from the workspace with its macOS deployment target configuration");
        assert!(
            target
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
                >= Some(13),
            "TapRelay requires MACOSX_DEPLOYMENT_TARGET >= 13.0"
        );
    }
    let version = std::env::var("TAPRELAY_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").expect("Cargo package version"));
    let windows_version = windows_version(&version);
    println!("cargo:rerun-if-env-changed=TAPRELAY_VERSION");
    println!("cargo:rustc-env=TAPRELAY_VERSION={version}");

    i18n::generate();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_windows_resources(&windows_version);
    }
}

fn compile_windows_resources(windows_version: &str) {
    let manifest_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    // GPUI supplies the asInvoker / PerMonitorV2 manifest. A second manifest conflicts at link time.
    let version_numbers = windows_version.replace('.', ",");
    let rc_source = format!(
        r#"1 ICON "{}"
1 VERSIONINFO
FILEVERSION {version_numbers}
PRODUCTVERSION {version_numbers}
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "ProductName", "TapRelay"
            VALUE "FileDescription", "TapRelay"
            VALUE "FileVersion", "{windows_version}"
            VALUE "ProductVersion", "{windows_version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#,
        rc_path(&manifest_dir.join("resources/taprelay.ico")),
    );
    let rc = out_dir.join("taprelay.rc");
    std::fs::write(&rc, rc_source).expect("Write generated Windows resource script");
    embed_resource::compile_for(&rc, ["taprelay"], embed_resource::NONE)
        .manifest_required()
        .expect("Compile app icon and version");
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-changed=resources/taprelay.ico");
}

fn windows_version(version: &str) -> String {
    let components = version.split('.').collect::<Vec<_>>();
    assert_eq!(
        components.len(),
        3,
        "TapRelay version must use the stable X.Y.Z format"
    );
    for component in components {
        assert!(
            component == "0"
                || (!component.is_empty()
                    && !component.starts_with('0')
                    && component.chars().all(|c| c.is_ascii_digit())),
            "TapRelay version must use the stable X.Y.Z format"
        );
        component
            .parse::<u16>()
            .expect("Windows version components must be at most 65535");
    }
    format!("{version}.0")
}

fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
