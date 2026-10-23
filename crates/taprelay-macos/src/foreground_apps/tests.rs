use super::*;

fn bitmap(size: usize, pixels: &[u8]) -> Retained<NSBitmapImageRep> {
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), size as isize, size as isize,
            8, 4, true, false, NSDeviceRGBColorSpace, NSBitmapFormat::empty(), (size * 4) as isize, 32,
        ).unwrap()
    };
    assert_eq!(pixels.len(), size * size * 4);
    assert_eq!(bitmap.bytesPerRow(), (size * 4) as isize);
    assert!(!bitmap.bitmapData().is_null());
    unsafe {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bitmap.bitmapData(), pixels.len());
    }
    bitmap
}

fn image(bitmap: &NSBitmapImageRep) -> Retained<NSImage> {
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(2., 2.));
    image.addRepresentation(bitmap);
    image
}

#[test]
fn direct_raster_preserves_rgba_alpha_and_restores_the_thread_context() {
    objc2::rc::autoreleasepool(|_| {
        let source = bitmap(2, &[64, 32, 16, 128].repeat(4));
        let previous = NSGraphicsContext::graphicsContextWithBitmapImageRep(&source).unwrap();
        let _restore = RestoreContext(NSGraphicsContext::currentContext());
        NSGraphicsContext::setCurrentContext(Some(&previous));
        let pixels = icon_pixels(&image(&source)).unwrap();
        assert_eq!(pixels.size, 64);
        assert_eq!(pixels.rgba.len(), 64 * 64 * 4);
        for pixel in pixels.rgba.as_chunks::<4>().0 {
            for (actual, expected) in pixel.iter().zip([128u8, 64, 32, 128]) {
                assert!(actual.abs_diff(expected) <= 1, "{pixel:?}");
            }
        }
        assert_eq!(
            Retained::as_ptr(&NSGraphicsContext::currentContext().unwrap()),
            Retained::as_ptr(&previous),
        );
    });
}

#[test]
fn direct_raster_preserves_vertical_orientation_and_transparent_pixels() {
    objc2::rc::autoreleasepool(|_| {
        let source = bitmap(
            2,
            &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0],
        );
        let pixels = icon_pixels(&image(&source)).unwrap();
        let at = |x: usize, y: usize| &pixels.rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(at(0, 0), [255, 0, 0, 255]);
        assert_eq!(at(63, 0), [0, 255, 0, 255]);
        assert_eq!(at(0, 63), [0, 0, 255, 255]);
        assert_eq!(at(63, 63), [0, 0, 0, 0]);
    });
}

#[test]
fn metadata_reuses_the_running_path_and_name_and_resolves_stale_paths() {
    let directory = tempfile::tempdir().unwrap();
    let app = directory.path().join("Example.app");
    std::fs::create_dir_all(app.join("Contents")).unwrap();
    std::fs::write(
        app.join("Contents/Info.plist"),
        br#"<?xml version="1.0"?><plist version="1.0"><dict>
        <key>CFBundleIdentifier</key><string>org.taprelay.metadata-test</string>
        <key>CFBundleDisplayName</key><string>Friendly Example</string>
        <key>CFBundleName</key><string>Example</string>
        </dict></plist>"#,
    )
    .unwrap();
    let current = metadata(
        "bundle:org.taprelay.metadata-test",
        Some(&app),
        Some("Running Example"),
    );
    assert_eq!(current.path.as_deref(), Some(app.as_path()));
    assert_eq!(current.name.as_deref(), Some("Running Example"));
    assert!(!current.missing);
    assert_eq!(
        identity_for_selection(app.to_str().unwrap()).unwrap(),
        "bundle:org.taprelay.metadata-test"
    );
    let id = format!("mac-path:{}", app.display());
    let stale = metadata(
        &id,
        Some(&directory.path().join("Removed.app")),
        Some("Old name"),
    );
    assert_eq!(stale.path.as_deref(), Some(app.as_path()));
    assert_eq!(stale.name.as_deref(), Some("Friendly Example"));
    assert!(!stale.missing);
    let missing = metadata("mac-path:/taprelay-test/nonexistent-app", None, None);
    assert!(missing.missing);
    assert!(missing.icon().is_none());
}

#[test]
#[ignore = "requires access to the logged-in desktop; run explicitly to measure native discovery"]
fn benchmark_running_application_metadata() {
    use std::time::{Duration, Instant};
    for pass in 1..=2 {
        let start = Instant::now();
        let apps = running().unwrap();
        let enumeration = start.elapsed();
        assert!(
            !apps.is_empty(),
            "desktop application discovery is unavailable"
        );
        let mut details_time = Duration::ZERO;
        let mut icons_time = Duration::ZERO;
        let mut bytes = 0;
        let mut loaded = 0;
        for app in &apps {
            let start = Instant::now();
            let details = metadata(&app.identity, app.path.as_deref(), app.name.as_deref());
            details_time += start.elapsed();
            let start = Instant::now();
            if let Some(icon) = details.icon() {
                loaded += 1;
                bytes += icon.rgba.len();
                assert_eq!(icon.size, 64);
            }
            icons_time += start.elapsed();
        }
        println!(
            "pass={pass} apps={} loaded={loaded} enumeration_ms={:.3} metadata_ms={:.3} icons_ms={:.3} rgba_bytes={bytes}",
            apps.len(),
            enumeration.as_secs_f64() * 1000.,
            details_time.as_secs_f64() * 1000.,
            icons_time.as_secs_f64() * 1000.
        );
    }
}
