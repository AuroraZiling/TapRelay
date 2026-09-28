//! A non-activating, click-through layered window. All handles stay on the UI thread.
use crate::native::OwnedWindow;
use std::{
    mem::size_of,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::*,
        },
    },
    core::{Result, w},
};

const FADE: Duration = Duration::from_millis(150);
const ICON_SIZE: f32 = 18.0;
const ICON_LEFT: f32 = 24.0;
const ICON_TEXT_GAP: f32 = 14.0;
const TEXT_LEFT: f32 = ICON_LEFT + ICON_SIZE + ICON_TEXT_GAP;
const CONTENT_INSET: f32 = TEXT_LEFT + 32.0;

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err(windows::core::Error::from_thread());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

struct Font(HFONT);
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

#[derive(Clone, Copy)]
struct Placement {
    work: RECT,
    scale: f32,
}

fn foreground_placement() -> Result<Placement> {
    unsafe {
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).ok()?;
        let (mut x, mut y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y)?;
        Ok(Placement {
            work: info.rcWork,
            scale: x as f32 / 96.0,
        })
    }
}

/// Pure timeline: replacing a visible notice resets its deadline, not its entrance.
struct Timeline {
    entered: Instant,
    until: Instant,
}
impl Timeline {
    fn replace(&mut self, now: Instant, duration: Duration) {
        if now >= self.until + FADE {
            self.entered = now;
        }
        self.until = now + duration;
    }
    fn frame(&self, now: Instant) -> Option<(u8, f32)> {
        if now >= self.until + FADE {
            return None;
        }
        let entrance = (now.saturating_duration_since(self.entered).as_secs_f32()
            / FADE.as_secs_f32())
        .min(1.0);
        let entrance = 1.0 - (1.0 - entrance).powi(3);
        let exit = if now > self.until {
            1.0 - now.duration_since(self.until).as_secs_f32() / FADE.as_secs_f32()
        } else {
            1.0
        };
        Some((
            (entrance * exit * 255.0).round() as u8,
            6.0 * (1.0 - entrance),
        ))
    }
}

pub struct Overlay {
    window: OwnedWindow,
    surface: Option<Surface>,
    placement: Option<Placement>,
    timeline: Option<Timeline>,
    last_frame: Option<(u8, i32)>,
}

impl Overlay {
    pub fn new() -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("TapRelay.PassthroughOverlay.v1");
            let registered = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            });
            if registered == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(windows::core::Error::from_thread());
            }
            let window = OwnedWindow(CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class,
                w!("TapRelay"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?);
            Ok(Self {
                window,
                surface: None,
                placement: None,
                timeline: None,
                last_frame: None,
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        prefix: &str,
        device: &str,
        suffix: &str,
        background: [u8; 3],
        foreground: [u8; 3],
        duration: Option<Duration>,
    ) -> Result<()> {
        let now = Instant::now();
        let visible = self
            .timeline
            .as_ref()
            .is_some_and(|t| t.frame(now).is_some());
        if duration.is_none() && !visible {
            return Ok(());
        }
        if !visible {
            self.placement = Some(foreground_placement()?);
        }
        let placement = self.placement.expect("visible placement");
        self.surface = Some(render(
            prefix, device, suffix, background, foreground, placement,
        )?);
        if let Some(duration) = duration {
            if let Some(timeline) = &mut self.timeline {
                timeline.replace(now, duration);
            } else {
                self.timeline = Some(Timeline {
                    entered: now,
                    until: now + duration,
                });
            }
        }
        self.last_frame = None;
        self.tick()?;
        unsafe {
            SetWindowPos(
                self.window.0,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        if self.timeline.take().is_some() {
            unsafe {
                let _ = ShowWindow(self.window.0, SW_HIDE);
            }
        }
        self.last_frame = None;
    }

    pub fn is_visible(&self) -> bool {
        self.timeline.is_some()
    }

    pub fn tick(&mut self) -> Result<()> {
        let Some(timeline) = &self.timeline else {
            return Ok(());
        };
        let Some((alpha, offset)) = timeline.frame(Instant::now()) else {
            self.hide();
            return Ok(());
        };
        let placement = self.placement.unwrap();
        let surface = self.surface.as_ref().unwrap();
        let offset = (offset * placement.scale).round() as i32;
        if self.last_frame == Some((alpha, offset)) {
            return Ok(());
        }
        let origin = POINT {
            x: placement.work.left
                + (placement.work.right - placement.work.left - surface.width) / 2,
            // Surface includes an 8-DIP shadow gutter; capsule itself is 16 DIPs above the work area.
            y: placement.work.bottom - surface.height - (8.0 * placement.scale).round() as i32
                + offset,
        };
        unsafe {
            UpdateLayeredWindow(
                self.window.0,
                None,
                Some(&origin),
                Some(&SIZE {
                    cx: surface.width,
                    cy: surface.height,
                }),
                Some(surface.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: alpha,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                }),
                ULW_ALPHA,
            )?;
        }
        self.last_frame = Some((alpha, offset));
        Ok(())
    }
}

fn measure(dc: HDC, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let mut text: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT::default();
    unsafe {
        DrawTextW(
            dc,
            &mut text,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
    }
    rect.right
}

fn elide(dc: HDC, text: &str, available: i32) -> String {
    if measure(dc, text) <= available {
        return text.into();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        let candidate: String = chars[..mid].iter().chain(std::iter::once(&'…')).collect();
        if measure(dc, &candidate) <= available {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    chars[..low].iter().chain(std::iter::once(&'…')).collect()
}

fn render(
    prefix: &str,
    device: &str,
    suffix: &str,
    background: [u8; 3],
    foreground: [u8; 3],
    placement: Placement,
) -> Result<Surface> {
    let scale = placement.scale;
    let px = |value: f32| (value * scale).round() as i32;
    let max_width = px(520.0)
        .min(placement.work.right - placement.work.left - px(16.0))
        .max(1);
    let mut surface = Surface::new(max_width, px(60.0))?;
    let font = Font(unsafe {
        CreateFontW(
            -px(14.0),
            0,
            0,
            0,
            FW_MEDIUM.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        )
    });
    if font.0.is_invalid() {
        return Err(windows::core::Error::from_thread());
    }
    let previous = unsafe { SelectObject(surface.dc, font.0.into()) };
    let device = elide(
        surface.dc,
        device,
        max_width - px(CONTENT_INSET) - measure(surface.dc, prefix) - measure(surface.dc, suffix),
    );
    let text = format!("{prefix}{device}{suffix}");
    let width = (measure(surface.dc, &text) + px(CONTENT_INSET)).min(max_width);
    unsafe {
        SelectObject(surface.dc, previous);
    }
    if width != max_width {
        surface = Surface::new(width, px(60.0))?;
    }
    surface.pixels().fill(0);
    unsafe {
        let previous = SelectObject(surface.dc, font.0.into());
        SetBkMode(surface.dc, TRANSPARENT);
        SetTextColor(surface.dc, COLORREF(0x00ffffff));
        let mut rect = RECT {
            left: px(TEXT_LEFT),
            top: px(8.0),
            right: width - px(24.0),
            bottom: px(52.0),
        };
        let mut text: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            surface.dc,
            &mut text,
            &mut rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        let _ = GdiFlush();
        SelectObject(surface.dc, previous);
    }
    let logical_width = width as f32 / scale;
    let pixels = surface.pixels();
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let x = (index as i32 % width) as f32 / scale + 0.5 / scale;
        let y = (index as i32 / width) as f32 / scale + 0.5 / scale;
        let distance = capsule_distance(x, y, logical_width);
        let cover = (0.5 - distance * scale).clamp(0.0, 1.0);
        let shadow = (-(distance.max(0.0) / 3.0).powi(2)).exp() * 0.16;
        let alpha = cover + shadow * (1.0 - cover);
        let mask = (*pixel & 255) as f32 / 255.0;
        let icon = screen_share_coverage(x - ICON_LEFT, y - (30.0 - ICON_SIZE / 2.0), scale);
        let ink = mask.max(icon) * cover;
        let mut channels = [0u8; 3];
        for channel in 0..3 {
            channels[channel] = (background[channel] as f32 * cover * (1.0 - ink)
                + foreground[channel] as f32 * ink)
                .round() as u8;
        }
        *pixel = ((alpha * 255.0).round() as u32) << 24
            | (channels[0] as u32) << 16
            | (channels[1] as u32) << 8
            | channels[2] as u32;
    }
    Ok(surface)
}

fn capsule_distance(x: f32, y: f32, width: f32) -> f32 {
    let center_x = x.clamp(30.0, (width - 30.0).max(30.0));
    ((x - center_x).powi(2) + (y - 30.0).powi(2)).sqrt() - 22.0
}

// Lucide ScreenShare's 24-unit path, matching the bundled lucide-slint icon.
// Render its round 2-unit strokes directly into the layered window's alpha mask.
fn screen_share_coverage(x: f32, y: f32, scale: f32) -> f32 {
    let icon_scale = ICON_SIZE / 24.0;
    let (x, y) = (x / icon_scale, y / icon_scale);
    let lines = [
        ((13.0, 3.0), (4.0, 3.0)),
        ((2.0, 5.0), (2.0, 15.0)),
        ((4.0, 17.0), (20.0, 17.0)),
        ((22.0, 15.0), (22.0, 12.0)),
        ((8.0, 21.0), (16.0, 21.0)),
        ((12.0, 17.0), (12.0, 21.0)),
        ((17.0, 8.0), (22.0, 3.0)),
        ((17.0, 3.0), (22.0, 3.0)),
        ((22.0, 3.0), (22.0, 8.0)),
    ];
    let mut distance = f32::MAX;
    for ((ax, ay), (bx, by)) in lines {
        let (dx, dy): (f32, f32) = (bx - ax, by - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        distance = distance.min(((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt());
    }
    for (cx, cy, in_corner) in [
        (4.0, 5.0, x <= 4.0 && y <= 5.0),
        (4.0, 15.0, x <= 4.0 && y >= 15.0),
        (20.0, 15.0, x >= 20.0 && y >= 15.0),
    ] {
        if in_corner {
            distance = distance.min((((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - 2.0).abs());
        }
    }
    (0.5 + (1.0 - distance) * icon_scale * scale).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_renders_at_multiple_scales_with_transparent_corners() {
        for scale in [1.0, 1.5, 2.0] {
            for dark in [false, true] {
                for (locale, prefix, suffix) in [
                    ("zh", "已断开与 ", " 的连接"),
                    ("en", "Disconnected from ", ""),
                ] {
                    let placement = Placement {
                        work: RECT {
                            left: -1920,
                            top: 0,
                            right: 0,
                            bottom: 1040,
                        },
                        scale,
                    };
                    let (background, foreground) = if dark {
                        ([23; 3], [250; 3])
                    } else {
                        ([245; 3], [23; 3])
                    };
                    let mut surface = render(
                        prefix,
                        "Artemis 的 iPad Pro / A very long receiver name that must be truncated without losing the connection status",
                        suffix,
                        background,
                        foreground,
                        placement,
                    )
                    .unwrap();
                    let (width, height) = (surface.width, surface.height);
                    assert!(width <= (520.0 * scale) as i32);
                    let pixels = surface.pixels();
                    assert_eq!(pixels[0] >> 24, 0);
                    assert_eq!(pixels[(width * height / 2 + width / 2) as usize] >> 24, 255);
                    assert!(
                        pixels
                            .iter()
                            .any(|pixel| pixel & 255 == foreground[2] as u32)
                    );
                    if let Some(dir) = std::env::var_os("TAPRELAY_UI_RENDER_DIR") {
                        let dir = std::path::PathBuf::from(dir);
                        std::fs::create_dir_all(&dir).unwrap();
                        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
                        for pixel in pixels {
                            let alpha = pixel.to_be_bytes()[0] as u16;
                            for shift in [16, 8, 0] {
                                let channel = ((*pixel >> shift) & 255) as u16;
                                ppm.push((channel + 110 * (255 - alpha) / 255).min(255) as u8);
                            }
                        }
                        std::fs::write(
                            dir.join(format!("capsule-{locale}-{dark}-{scale}.ppm")),
                            ppm,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "Briefly shows a native capsule; requires an interactive Windows desktop"]
    fn native_window_keeps_focus_and_has_click_through_toolwindow_styles() {
        unsafe {
            let foreground = GetForegroundWindow();
            let mut overlay = Overlay::new().unwrap();
            overlay
                .update(
                    "正在控制 ",
                    "测试设备",
                    "",
                    [23; 3],
                    [250; 3],
                    Some(Duration::from_secs(3)),
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(180));
            overlay.tick().unwrap();
            assert_eq!(GetForegroundWindow(), foreground);
            let style = GetWindowLongPtrW(overlay.window.0, GWL_EXSTYLE) as u32;
            let expected = WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | WS_EX_TOPMOST;
            assert_eq!(style & expected.0, expected.0);
            assert_eq!(style & WS_EX_APPWINDOW.0, 0);
            let placement = overlay.placement.unwrap();
            let surface = overlay.surface.as_ref().unwrap();
            let mut bounds = RECT::default();
            GetWindowRect(overlay.window.0, &mut bounds).unwrap();
            assert_eq!(
                bounds.left,
                placement.work.left
                    + (placement.work.right - placement.work.left - surface.width) / 2
            );
            assert_eq!(
                bounds.bottom,
                placement.work.bottom - (8.0 * placement.scale).round() as i32
            );
            overlay.hide();
            assert!(!IsWindowVisible(overlay.window.0).as_bool());
        }
    }

    #[test]
    fn replacement_extends_deadline_without_repeating_entrance() {
        let now = Instant::now();
        let mut timeline = Timeline {
            entered: now,
            until: now + Duration::from_secs(3),
        };
        assert_eq!(timeline.frame(now), Some((0, 6.0)));
        let later = now + Duration::from_secs(2);
        assert_eq!(timeline.frame(later), Some((255, 0.0)));
        timeline.replace(later, Duration::from_secs(5));
        assert_eq!(timeline.frame(later), Some((255, 0.0)));
        assert_eq!(
            timeline.frame(now + Duration::from_secs(4)),
            Some((255, 0.0))
        );
        assert!(
            timeline
                .frame(now + Duration::from_secs(7) + FADE)
                .is_none()
        );
    }

    #[test]
    fn a_new_notice_during_fade_cancels_the_old_fade() {
        let now = Instant::now();
        let mut timeline = Timeline {
            entered: now,
            until: now + Duration::from_secs(3),
        };
        let fading = now + Duration::from_millis(3075);
        assert!(timeline.frame(fading).unwrap().0 < 255);
        timeline.replace(fading, Duration::from_secs(3));
        assert_eq!(timeline.frame(fading), Some((255, 0.0)));
    }
}
