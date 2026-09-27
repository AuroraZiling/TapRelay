//! Native executable icons, converted by WIC to straight-alpha RGBA off the UI thread.
use windows::{
    Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapSource,
            IWICImagingFactory, WICBitmapInterpolationModeFant, WICConvertBitmapSource,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::{Shell::SHDefExtractIconW, WindowsAndMessaging::HICON},
    },
    core::{Interface, PCWSTR},
};

pub struct IconPixels {
    pub size: u32,
    pub rgba: Vec<u8>,
}

pub fn executable_icon(path: &str) -> Option<IconPixels> {
    if !taprelay_core::foreground_app::valid_executable(path) {
        return None;
    }
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    const SIZE: u32 = 64;
    unsafe {
        let mut handle = HICON::default();
        let result = SHDefExtractIconW(PCWSTR(path.as_ptr()), 0, 0, Some(&mut handle), None, SIZE);
        if handle.0.is_null() {
            return None;
        }
        let icon = crate::native::OwnedIcon(handle);
        result.ok().ok()?;
        icon_pixels(icon.0, SIZE)
    }
}

fn icon_pixels(icon: HICON, size: u32) -> Option<IconPixels> {
    // Initialize the metadata worker's apartment. If the caller already owns an
    // STA, RoInitialize cannot switch it; WIC can still use that existing apartment.
    // All COM objects below are dropped before this guard uninitializes our MTA.
    let _apartment = crate::native::Apartment::new().ok();
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let bitmap = factory.CreateBitmapFromHICON(icon).ok()?;
        let (mut width, mut height) = (0, 0);
        bitmap.GetSize(&mut width, &mut height).ok()?;
        let source: IWICBitmapSource = if (width, height) == (size, size) {
            bitmap.cast().ok()?
        } else {
            let scaler = factory.CreateBitmapScaler().ok()?;
            scaler
                .Initialize(&bitmap, size, size, WICBitmapInterpolationModeFant)
                .ok()?;
            scaler.cast().ok()?
        };
        let source = WICConvertBitmapSource(&GUID_WICPixelFormat32bppRGBA, &source).ok()?;
        let stride = size.checked_mul(4)?;
        let mut rgba = vec![0; stride.checked_mul(size)? as usize];
        source
            .CopyPixels(std::ptr::null(), stride, &mut rgba)
            .ok()?;
        Some(IconPixels { size, rgba })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIcon, CreateIconFromResourceEx, LR_DEFAULTCOLOR,
    };

    #[test]
    fn wic_preserves_color_and_alpha_and_resizes_icons() {
        // A 2x2 executable icon resource with straight BGRA pixels. Let Windows
        // load it just as it loads resource icons extracted from executable files.
        for (pixel, expected) in [
            ([10u8, 20, 30, 255], [30u8, 20, 10, 255]),
            ([100, 40, 20, 128], [20, 40, 100, 128]),
        ] {
            let mut resource = vec![0u8; 40];
            resource[..4].copy_from_slice(&40u32.to_le_bytes());
            resource[4..8].copy_from_slice(&2i32.to_le_bytes());
            resource[8..12].copy_from_slice(&4i32.to_le_bytes());
            resource[12..14].copy_from_slice(&1u16.to_le_bytes());
            resource[14..16].copy_from_slice(&32u16.to_le_bytes());
            resource.extend(pixel.repeat(4));
            resource.extend([0u8; 8]); // DWORD-aligned AND mask.
            let icon = crate::native::OwnedIcon(unsafe {
                CreateIconFromResourceEx(&resource, true, 0x30000, 2, 2, LR_DEFAULTCOLOR).unwrap()
            });
            for size in [2, 64] {
                let image = icon_pixels(icon.0, size).unwrap();
                assert_eq!(image.rgba.len(), (size * size * 4) as usize);
                for actual in image.rgba.as_chunks::<4>().0 {
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert!(actual.abs_diff(expected) <= 1, "{actual} != {expected}");
                    }
                }
            }
        }
    }

    #[test]
    fn wic_honors_transparency_in_legacy_monochrome_masks() {
        // WORD-aligned rows: left pixel transparent, right pixel opaque black.
        let and_mask = [0x80u8, 0, 0x80, 0];
        let xor_mask = [0u8; 4];
        let icon = crate::native::OwnedIcon(unsafe {
            CreateIcon(None, 2, 2, 1, 1, and_mask.as_ptr(), xor_mask.as_ptr()).unwrap()
        });
        let image = icon_pixels(icon.0, 2).unwrap();
        for row in image.rgba.as_chunks::<8>().0 {
            assert_eq!(row[3], 0);
            assert_eq!(&row[4..], &[0, 0, 0, 255]);
        }
    }

    #[test]
    fn conversion_also_works_in_a_callers_existing_sta() {
        std::thread::spawn(|| {
            use windows::Win32::System::Com::{
                COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
            };
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            }
            let path = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("explorer.exe");
            let image = executable_icon(&path.to_string_lossy());
            unsafe {
                CoUninitialize();
            }
            assert!(image.is_some());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn executable_icon_loads_windows_icon_and_missing_files_fail_without_an_icon() {
        let path =
            std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("explorer.exe");
        let image = executable_icon(&path.to_string_lossy()).expect("Explorer icon");
        assert_eq!(image.size, 64);
        assert_eq!(image.rgba.len(), (image.size * image.size * 4) as usize);
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] == 0)
        );
        assert!(executable_icon(r"C:\TapRelay-missing-icon-test\missing.exe").is_none());
        assert!(executable_icon("invalid").is_none());
    }
}
