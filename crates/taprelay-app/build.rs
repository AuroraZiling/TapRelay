use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[path = "build/i18n.rs"]
mod i18n;

fn main() {
    let version = std::env::var("TAPRELAY_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").expect("Cargo package version"));
    let windows_version = windows_version(&version);
    println!("cargo:rerun-if-env-changed=TAPRELAY_VERSION");
    println!("cargo:rustc-env=TAPRELAY_VERSION={version}");

    let i18n = i18n::generate();
    let lucide = PathBuf::from(lucide_slint::lib());
    // FlexboxLayout is the native wrapping layout used by the bindings page.
    unsafe {
        std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1");
    }
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_library_paths(HashMap::from([
                // Use i18n
                ("i18n".into(), i18n),
                // Use Lucide Icons
                ("lucide".to_string(), lucide),
                // Use Lucide Lab Icons
                // (
                //     "lucide-lab".to_string(),
                //     PathBuf::from(lucide_slint::lib_lab()),),
            ]))
            .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles),
    )
    .expect("Compile TapRelay UI");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_windows_resources(&windows_version);
    }
}

fn compile_windows_resources(windows_version: &str) {
    let manifest_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let manifest_template = manifest_dir.join("resources/taprelay.manifest.in");
    let manifest = out_dir.join("taprelay.manifest");
    let template = std::fs::read_to_string(&manifest_template).expect("Read Windows manifest");
    assert_eq!(
        template.matches("@WINDOWS_VERSION@").count(),
        1,
        "Windows manifest must contain exactly one @WINDOWS_VERSION@ placeholder"
    );
    std::fs::write(
        &manifest,
        template.replace("@WINDOWS_VERSION@", windows_version),
    )
    .expect("Write generated Windows manifest");

    let rc_source = format!(
        "1 ICON \"{}\"\n1 24 \"{}\"\n",
        rc_path(&manifest_dir.join("resources/taprelay.ico")),
        rc_path(&manifest),
    );
    let rc = out_dir.join("taprelay.rc");
    std::fs::write(&rc, rc_source).expect("Write generated Windows resource script");
    embed_resource::compile_for(&rc, ["taprelay"], embed_resource::NONE)
        .manifest_required()
        .expect("Compile app icon and manifest");
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-changed=resources/taprelay.ico");
    println!("cargo:rerun-if-changed=resources/taprelay.manifest.in");
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
