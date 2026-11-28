use super::*;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::mpsc,
};
use velopack::{
    UpdateManager,
    locator::{VelopackLocator, VelopackLocatorConfig},
};
use zip::{ZipWriter, write::SimpleFileOptions};

const DIGEST_ABC: &str = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";

fn feed() -> Value {
    json!({"Assets": [{
        "Type": "Full", "PackageId": "TapRelay", "Version": "1.10.0",
        "FileName": "TapRelay-1.10.0-full.nupkg", "Size": 3, "SHA256": DIGEST_ABC
    }]})
}

fn available(check: Check) -> Target {
    match check {
        Check::Available(target) => target,
        other => panic!("Expected an update, got {other:?}"),
    }
}

#[test]
fn stable_feeds_use_semantic_order_and_exact_platform_assets() {
    let body = feed().to_string();
    let target = available(select_target(&body, "win", Delivery::Portable, "1.9.9", None).unwrap());
    assert_eq!(target.tag(), "v1.10.0");
    assert_eq!(
        target.portable_url().unwrap(),
        format!("{RELEASES}/download/v1.10.0/TapRelay_1.10.0_Windows_x64.zip")
    );
    assert_eq!(target.release_url(), format!("{RELEASES}/tag/v1.10.0"));
    for current in ["1.10.0", "2.0.0"] {
        assert!(matches!(
            select_target(&body, "win", Delivery::Installed, current, None).unwrap(),
            Check::Current
        ));
    }
    for channel in ["osx-arm64", "osx-x64"] {
        assert!(select_target(&body, channel, Delivery::Installed, "1.0.0", None).is_err());
        let mut data = feed();
        data["Assets"][0]["FileName"] = format!("TapRelay-1.10.0-{channel}-full.nupkg").into();
        assert!(
            available(
                select_target(
                    &data.to_string(),
                    channel,
                    Delivery::Installed,
                    "1.0.0",
                    None
                )
                .unwrap()
            )
            .portable_url()
            .is_none()
        );
    }
    assert!(select_target(&body, "win", Delivery::Installed, "1.0.0", Some("v1.0.1")).is_err());
}

#[test]
fn malformed_releases_cannot_supply_a_download_target() {
    for (key, value) in [
        ("Type", "Delta"),
        ("PackageId", "Other"),
        ("Version", "1.10.0-beta"),
        ("Version", "1.10.0+build"),
        ("FileName", "../TapRelay-1.10.0-full.nupkg"),
        ("FileName", "https://example.invalid/file"),
        ("SHA256", ""),
    ] {
        let mut data = feed();
        data["Assets"][0][key] = value.into();
        assert!(
            select_target(&data.to_string(), "win", Delivery::Installed, "1.0.0", None).is_err(),
            "{key}={value}"
        );
    }
    assert!(!is_stable_tag("v01.10.0"));
    assert!(!is_stable_tag("1.10.0"));
    assert!(is_stable_tag("v0.1.0"));
}

#[test]
fn loose_executables_never_offer_an_in_place_update() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    assert_eq!(
        delivery(&dir.path().join("TapRelay.exe"), "win").unwrap(),
        Some(Delivery::Portable)
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        delivery(
            &dir.path().join("TapRelay.app/Contents/MacOS/taprelay"),
            platform_channel().unwrap()
        )
        .unwrap(),
        None
    );
    let target = Target::for_test("1.10.0", true);
    assert!(download(&target, mpsc::channel().0).is_err());
}

fn server(responses: Vec<Vec<u8>>) -> (String, mpsc::Receiver<String>) {
    let socket = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let base = format!("http://{}", socket.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = socket.accept().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            for header in reader.lines() {
                if header.unwrap().is_empty() {
                    break;
                }
            }
            tx.send(request.split_whitespace().nth(1).unwrap().to_owned())
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (base, rx)
}

#[test]
fn discovery_requests_only_the_static_index_and_download_verifies_the_confirmed_tag() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("fixture.nupkg");
    let mut zip = ZipWriter::new(fs::File::create(&fixture).unwrap());
    zip.start_file("TapRelay.nuspec", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"<metadata><id>TapRelay</id><version>1.10.0</version><mainExe>TapRelay.exe</mainExe></metadata>").unwrap();
    zip.start_file("lib/app/Squirrel.exe", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"fixture update helper").unwrap();
    drop(zip.finish().unwrap());
    let package = fs::read(&fixture).unwrap();
    let mut damaged = package.clone();
    damaged[0] ^= 1;
    let mut meta = feed();
    meta["Assets"][0]["Size"] = package.len().into();
    meta["Assets"][0]["SHA256"] = velopack::misc::calculate_sha1_sha256(&fixture)
        .unwrap()
        .1
        .into();
    let mut newer = meta.clone();
    newer["Assets"][0]["Version"] = "1.11.0".into();
    newer["Assets"][0]["FileName"] = "TapRelay-1.11.0-full.nupkg".into();
    let (base, requests) = server(vec![
        meta.to_string().into_bytes(),
        newer.to_string().into_bytes(),
        meta.to_string().into_bytes(),
        damaged.clone(),
        package.clone(),
    ]);
    let target = available(check_feed(&base, "win", Delivery::Installed, "1.0.0", None).unwrap());
    assert_eq!(
        requests.try_recv().unwrap(),
        "/latest/download/releases.win.json"
    );
    assert!(requests.try_recv().is_err());
    let latest = available(check_feed(&base, "win", Delivery::Installed, "1.0.0", None).unwrap());
    assert_eq!(latest.tag(), "v1.11.0");
    let confirmed = available(
        check_feed(
            &base,
            "win",
            Delivery::Installed,
            "1.0.0",
            Some(&target.tag()),
        )
        .unwrap(),
    );
    assert_eq!(confirmed.tag(), "v1.10.0");
    assert_eq!(
        requests.try_iter().collect::<Vec<_>>(),
        [
            "/latest/download/releases.win.json",
            "/download/v1.10.0/releases.win.json"
        ]
    );

    let root = dir.path();
    let cfg = VelopackLocatorConfig {
        RootAppDir: root.to_path_buf(),
        UpdateExePath: root.join("updater"),
        PackagesDir: root.join("packages"),
        ManifestPath: root.join("sq.version"),
        CurrentBinaryDir: root.to_path_buf(),
        IsPortable: false,
    };
    fs::write(&cfg.UpdateExePath, []).unwrap();
    fs::write(&cfg.ManifestPath, "<metadata><id>TapRelay</id><version>1.0.0</version><mainExe>TapRelay.exe</mainExe><channel>win</channel></metadata>").unwrap();
    let locator = VelopackLocator::new(&cfg).unwrap();
    let manager = UpdateManager::new(package::source(&base, &confirmed), None, Some(cfg)).unwrap();
    let (progress, events) = mpsc::channel();
    let download =
        || package::download_checked(manager.clone(), &locator, &target, progress.clone());
    let cached = locator.get_packages_dir().join(&target.asset.FileName);
    assert!(matches!(
        download().err().unwrap().downcast_ref::<velopack::Error>(),
        Some(velopack::Error::ChecksumInvalid(..))
    ));
    assert!(!cached.exists());
    fs::write(&cached, &damaged).unwrap();
    download().unwrap();
    assert_eq!(fs::read(&cached).unwrap(), package);
    #[cfg(windows)]
    assert_eq!(
        fs::read(locator.get_update_path()).unwrap(),
        b"fixture update helper"
    );
    download().unwrap();
    let asset_url = "/download/v1.10.0/TapRelay-1.10.0-full.nupkg";
    assert_eq!(
        requests.try_iter().collect::<Vec<_>>(),
        [asset_url, asset_url]
    );
    assert_eq!(events.try_iter().last(), Some(100));
}

#[test]
fn changelog_retains_both_languages_and_falls_back_on_invalid_responses() {
    let body = json!({
        "tag":"v1.10.0","en":{"fallback":false,"blocks":[{"kind":"bullet","text":"Fix"}]},
        "zh":{"fallback":false,"blocks":[{"kind":"fixed","text":"修复"}]}
    })
    .to_string();
    let (base, requests) = server(vec![
        body.clone().into_bytes(),
        body.into_bytes(),
        b"not json".to_vec(),
    ]);
    let notes = notes::at(&base, "v1.10.0").unwrap();
    assert_eq!(notes.en.blocks[0].kind, NoteKind::Bullet);
    assert_eq!(notes.zh.blocks[0].text, "修复");
    assert!(notes::at(&base, "v1.20.0").is_none());
    assert!(notes::at(&base, "v1.10.0").is_none());
    assert_eq!(requests.try_recv().unwrap(), "/api/changelog/v1.10.0/");
}
