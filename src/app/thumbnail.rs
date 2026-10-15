//! Small circular header artwork, reusing the scene's already-loaded cover.
use gtk::{gdk::prelude::GdkCairoContextExt, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone)]
pub(super) struct HeaderArtwork {
    pub widget: gtk::DrawingArea,
    pixels: Rc<RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>>,
    revision: Rc<Cell<Option<u64>>>,
}

impl HeaderArtwork {
    pub fn new() -> Self {
        let widget = gtk::DrawingArea::new();
        widget.set_widget_name("header-cover-art");
        widget.set_content_width(36);
        widget.set_content_height(36);
        widget.set_valign(gtk::Align::Center);
        widget.set_visible(false);
        widget.update_property(&[gtk::accessible::Property::Label("Album cover")]);
        let pixels = Rc::new(RefCell::new(None::<gtk::gdk_pixbuf::Pixbuf>));
        let image = pixels.clone();
        widget.set_draw_func(move |_, context, width, height| {
            let pixels = image.borrow();
            let Some(pixels) = pixels.as_ref() else {
                return;
            };
            let size = width.min(height) as f64;
            let x = (width as f64 - size) / 2.0;
            let y = (height as f64 - size) / 2.0;
            context.arc(
                width as f64 / 2.0,
                height as f64 / 2.0,
                size / 2.0,
                0.0,
                std::f64::consts::TAU,
            );
            context.clip();
            context.translate(x, y);
            context.scale(size / pixels.width() as f64, size / pixels.height() as f64);
            context.set_source_pixbuf(pixels, 0.0, 0.0);
            let _ = context.paint();
        });
        Self {
            widget,
            pixels,
            revision: Rc::new(Cell::new(None)),
        }
    }

    pub fn sync(&self, revision: u64, image: Option<&image::RgbaImage>) {
        if self.revision.get() == Some(revision) {
            return;
        }
        self.revision.set(Some(revision));
        *self.pixels.borrow_mut() = image.map(|image| {
            // The loader already provides square artwork. One small resize per
            // revision supplies enough detail for up to 4x display scaling.
            let pixels =
                image::imageops::resize(image, 144, 144, image::imageops::FilterType::Lanczos3);
            gtk::gdk_pixbuf::Pixbuf::from_bytes(
                &gtk::glib::Bytes::from_owned(pixels.into_raw()),
                gtk::gdk_pixbuf::Colorspace::Rgb,
                true,
                8,
                144,
                144,
                144 * 4,
            )
        });
        self.widget.set_visible(image.is_some());
        self.widget.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires a GTK display"]
    fn thumbnail_updates_only_on_revision_and_clears_missing_art() {
        gtk::init().unwrap();
        let thumbnail = HeaderArtwork::new();
        assert!(!thumbnail.widget.is_visible());
        let cover = image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 90, 40, 255]));
        thumbnail.sync(1, Some(&cover));
        assert!(thumbnail.widget.is_visible());
        let first = thumbnail.pixels.borrow().clone().unwrap();
        thumbnail.sync(1, Some(&cover));
        assert_eq!(thumbnail.pixels.borrow().as_ref(), Some(&first));
        thumbnail.sync(2, None);
        assert!(!thumbnail.widget.is_visible());
        assert!(thumbnail.pixels.borrow().is_none());
        thumbnail.sync(3, Some(&cover));
        assert!(thumbnail.widget.is_visible());
    }
}
