//! Fixed logical size with the original high-resolution texture for sharp scaling.
use gtk::{gdk, glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Wordmark {
        pub texture: std::cell::OnceCell<gdk::Texture>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Wordmark {
        const NAME: &'static str = "TalisSettingsWordmark";
        type Type = super::Wordmark;
        type Interfaces = (gdk::Paintable,);
    }
    impl ObjectImpl for Wordmark {}
    impl gdk::subclass::prelude::PaintableImpl for Wordmark {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::SIZE | gdk::PaintableFlags::CONTENTS
        }
        fn intrinsic_width(&self) -> i32 {
            54
        }
        fn intrinsic_height(&self) -> i32 {
            39
        }
        fn intrinsic_aspect_ratio(&self) -> f64 {
            54.0 / 39.0
        }
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            self.texture
                .get()
                .expect("initialized wordmark")
                .snapshot(snapshot, width, height);
        }
    }
}
glib::wrapper! {
    pub struct Wordmark(ObjectSubclass<imp::Wordmark>) @implements gdk::Paintable;
}
impl Wordmark {
    pub(super) fn new(texture: gdk::Texture) -> Self {
        let wordmark: Self = glib::Object::new();
        wordmark
            .imp()
            .texture
            .set(texture)
            .expect("one wordmark texture");
        wordmark
    }
}
