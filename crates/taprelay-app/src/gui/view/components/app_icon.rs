use gpui_kit::{Pixels, RenderImage, Window};
use image::{Frame, RgbaImage, imageops::FilterType};
use std::sync::{Arc, LazyLock};

#[derive(Default)]
pub(in crate::gui::view) struct AppIcon {
    rasters: Vec<(u32, Arc<RenderImage>)>,
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
        if let Some((_, image)) = self.rasters.iter().find(|(cached, _)| *cached == pixels) {
            return image.clone();
        }
        // Resample at the display's physical size before GPUI's bilinear sampling.
        let mut raster = image::imageops::resize(&*SOURCE, pixels, pixels, FilterType::Lanczos3);
        // GPUI's render cache consumes BGRA.
        for pixel in raster.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        let image = Arc::new(RenderImage::new(vec![Frame::new(raster)]));
        if self.rasters.len() >= 4 {
            self.rasters.remove(0);
        }
        self.rasters.push((pixels, image.clone()));
        image
    }
}
