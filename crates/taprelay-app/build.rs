use std::{collections::HashMap, path::PathBuf};

#[path = "build/i18n.rs"]
mod translations;

fn main() {
    let i18n = translations::generate();
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
        use std::path::PathBuf;
        let resource = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("taprelay.res");
        let compiler = std::env::var_os("RC")
            .map(PathBuf::from)
            .or_else(|| {
                let kits = PathBuf::from(std::env::var_os("ProgramFiles(x86)")?)
                    .join("Windows Kits/10/bin");
                let mut versions = std::fs::read_dir(kits)
                    .ok()?
                    .filter_map(Result::ok)
                    .map(|d| d.path().join("x64/rc.exe"))
                    .filter(|p| p.is_file())
                    .collect::<Vec<_>>();
                versions.sort();
                versions.pop()
            })
            .expect("Windows SDK resource compiler is required (or set RC)");
        let status = std::process::Command::new(compiler)
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg("resources/taprelay.rc")
            .status()
            .expect("Run Windows resource compiler");
        assert!(status.success(), "Compile app icon and manifest");
        println!("cargo:rustc-link-arg-bin=taprelay={}", resource.display());
        println!("cargo:rerun-if-changed=resources");
    }
}
