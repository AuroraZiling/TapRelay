use gpui_kit::{Pixels, RenderImage, Window};
use image::{Frame, RgbaImage, imageops::FilterType};
use std::sync::{Arc, LazyLock};

#[derive(Default)]
pub(in crate::gui::view) struct AppIcon {
    raster: Option<(u32, Arc<RenderImage>)>,
}

impl AppIcon {
    pub(in crate::gui::view) fn image(
        &mut self,
        size: Pixels,
        window: &Window,
    ) -> Arc<RenderImage> {
        static SOURCE: LazyLock<RgbaImage> = LazyLock::new(|| {
            image::load_from_memory(include_bytes!("../../../../resources/taprelay-128.png"))
                .expect("embedded application icon must decode")
                .into_rgba8()
        });
        let pixels = (f32::from(size) * window.scale_factor()).round().max(1.) as u32;
        if self
            .raster
            .as_ref()
            .is_none_or(|(cached, _)| *cached != pixels)
        {
            // Resample at the display's physical size before GPUI's bilinear sampling.
            let mut raster =
                image::imageops::resize(&*SOURCE, pixels, pixels, FilterType::Lanczos3);
            // GPUI's render cache consumes BGRA.
            for pixel in raster.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            self.raster = Some((pixels, Arc::new(RenderImage::new(vec![Frame::new(raster)]))));
        }
        self.raster.as_ref().unwrap().1.clone()
    }
}
