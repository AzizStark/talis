//! Scoped controls styling and reusable settings rows; no scene styling.
mod wordmark;
use gtk::prelude::*;

const LOGO_PNG: &[u8] = include_bytes!("../../assets/ui/branding/talis-logo.png");

const NAVIGATION_ICONS: [(&str, &str); 4] = [
    (
        "appearance",
        include_str!("../../assets/ui/icons/appearance.svg"),
    ),
    ("general", include_str!("../../assets/ui/icons/general.svg")),
    (
        "advanced",
        include_str!("../../assets/ui/icons/advanced.svg"),
    ),
    ("about", include_str!("../../assets/ui/icons/about.svg")),
];
const NAVIGATION_ICON_SIZE: i32 = 18;
pub(super) const APP_ICON_NAME: &str = "io.github.azizstark.Talis";

/// GTK's named icon lookup alone does not publish X11 window icon pixels on
/// every GTK/backend combination. Use the public surface API on realization;
/// unsupported backends may ignore it. Decode once, never during scene frames.
pub(super) fn bind_window_icon(window: &impl IsA<gtk::Window>) {
    window.as_ref().connect_realize(|window| {
        let Some(surface) = window.surface() else {
            return;
        };
        let Ok(toplevel) = surface.dynamic_cast::<gtk::gdk::Toplevel>() else {
            return;
        };
        thread_local! {
            static ICONS: std::cell::OnceCell<Vec<gtk::gdk::Texture>> = const { std::cell::OnceCell::new() };
        }
        ICONS.with(|slot| {
            let icons = slot.get_or_init(|| {
                [16, 24, 32, 48, 64, 128]
                    .into_iter()
                    .filter_map(|size| {
                        let bytes = gtk::glib::Bytes::from_static(include_bytes!(
                            "../../assets/ui/branding/talis-monogram-light.svg"
                        ));
                        let stream = gtk::gio::MemoryInputStream::from_bytes(&bytes);
                        match gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
                            &stream, size, size, true, None::<&gtk::gio::Cancellable>,
                        ) {
                            Ok(pixbuf) => Some(gtk::gdk::Texture::for_pixbuf(&pixbuf)),
                            Err(error) => {
                                tracing::warn!(size, "Cannot decode window icon: {error}");
                                None
                            }
                        }
                    })
                    .collect()
            });
            if !icons.is_empty() {
                toplevel.set_icon_list(icons);
            }
        });
    });
}

pub(super) fn install_app_icon(display: &gtk::gdk::Display) {
    let theme = gtk::IconTheme::for_display(display);
    // Scene launches already use a package working directory. Also support
    // development launches outside it without requiring a system icon install.
    for root in [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        std::env::current_dir().unwrap_or_default(),
    ] {
        let icons = root.join("assets/ui/branding");
        if icons.join(format!("{APP_ICON_NAME}.svg")).is_file() {
            theme.add_search_path(icons);
        }
    }
    gtk::Window::set_default_icon_name(APP_ICON_NAME);
}

fn icon_texture(
    svg: &str,
    scale: i32,
    size: i32,
    colour: &str,
) -> Result<gtk::gdk::Texture, gtk::glib::Error> {
    // Resolve currentColor to the scoped sidebar foreground before SVG loading.
    let bytes = gtk::glib::Bytes::from_owned(svg.replace("currentColor", colour).into_bytes());
    let stream = gtk::gio::MemoryInputStream::from_bytes(&bytes);
    let pixels = size * scale.max(1);
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
        &stream,
        pixels,
        pixels,
        true,
        None::<&gtk::gio::Cancellable>,
    )?;
    Ok(gtk::gdk::Texture::for_pixbuf(&pixbuf))
}

/// Embedded SVGs are decoded only at creation or a monitor scale change.
pub(super) fn navigation_icon(page: &str) -> gtk::Image {
    let svg = NAVIGATION_ICONS
        .iter()
        .find(|(id, _)| *id == page)
        .expect("known settings page")
        .1;
    let image = svg_icon(svg, NAVIGATION_ICON_SIZE, "#b8c3c6");
    image.set_widget_name(&format!("{page}-icon"));
    image
}

/// Shared vector decoding for sidebar/About; sized for the actual display scale.
pub(super) fn svg_icon(svg: &'static str, size: i32, colour: &'static str) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    image.set_valign(gtk::Align::Center);
    let update =
        move |image: &gtk::Image| match icon_texture(svg, image.scale_factor(), size, colour) {
            Ok(texture) => image.set_paintable(Some(&texture)),
            Err(error) => {
                tracing::warn!("Cannot load UI SVG: {error}");
                image.set_icon_name(Some("image-missing-symbolic"));
            }
        };
    update(&image);
    image.connect_scale_factor_notify(update);
    image
}

/// Treat the supplied black-on-transparent wordmark like a foreground glyph.
/// The original PNG remains unmodified; only its display colour changes.
pub(super) fn logo() -> gtk::Picture {
    let mut pixels = image::load_from_memory(LOGO_PNG)
        .expect("bundled Talis logo must be a valid PNG")
        .into_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0[..3].copy_from_slice(&[213, 218, 221]);
    }
    let (width, height) = pixels.dimensions();
    let texture = gtk::gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &gtk::glib::Bytes::from_owned(pixels.into_raw()),
        width as usize * 4,
    );
    let logo = gtk::Picture::for_paintable(&wordmark::Wordmark::new(texture.upcast()));
    logo.set_widget_name("talis-logo");
    logo.set_alternative_text(Some("Talis"));
    logo.set_content_fit(gtk::ContentFit::Contain);
    logo.set_can_shrink(true);
    logo.set_size_request(54, 39);
    logo.set_halign(gtk::Align::Start);
    logo
}

pub(super) fn install(display: &gtk::gdk::Display) {
    thread_local! {
        static PROVIDER: std::cell::OnceCell<gtk::CssProvider> = const { std::cell::OnceCell::new() };
    }
    PROVIDER.with(|slot| {
        slot.get_or_init(|| {
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(|_, _, error| tracing::error!("Controls CSS: {error}"));
            provider.load_from_data(include_str!("controls.css"));
            gtk::style_context_add_provider_for_display(
                display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            provider
        });
    });
}

pub(super) fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class(class);
    label
}

pub(super) fn section(title: &str, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    section.append(&label(title, "section-title"));
    body.add_css_class("settings-card");
    section.append(body);
    section
}

/// Each desktop settings page scrolls independently; the sidebar stays put.
pub(super) fn page(title: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 22);
    page.set_margin_top(24);
    page.set_margin_bottom(24);
    page.set_margin_start(24);
    page.set_margin_end(24);
    page.append(&label(title, "page-title"));
    page
}

pub(super) fn row(title: &str, detail: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    row_with_detail(title, detail, control).0
}

pub(super) fn row_with_detail(
    title: &str,
    detail: &str,
    control: &impl IsA<gtk::Widget>,
) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    // Constrain descriptions, not right-aligned controls, on narrow windows.
    text.set_size_request(208, -1);
    let title = label(title, "setting-title");
    title.set_mnemonic_widget(Some(control));
    text.append(&title);
    let detail_label = label(detail, "setting-detail");
    detail_label.set_visible(!detail.is_empty());
    text.append(&detail_label);
    control.set_valign(gtk::Align::Center);
    if control.as_ref().is::<gtk::SpinButton>() {
        control.set_size_request(145, -1);
        disable_value_scroll(control.as_ref());
    } else if control.as_ref().is::<gtk::ComboBoxText>() {
        control.set_size_request(200, -1);
        disable_value_scroll(control.as_ref());
    }
    control.add_css_class("setting-control");
    row.append(&text);
    row.append(control);
    (row, detail_label)
}

/// Let wheel/trackpad scrolling reach the page instead of changing a value.
/// Inspect only once while building the row; do not retain a controller model or
/// intercept input. Popup lists and click/keyboard controllers remain untouched.
fn disable_value_scroll(control: &gtk::Widget) {
    let controllers = control.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(scroll) = controllers
            .item(index)
            .and_then(|item| item.downcast::<gtk::EventControllerScroll>().ok())
        {
            scroll.set_propagation_phase(gtk::PropagationPhase::None);
        }
    }
}

pub(super) fn separator(body: &gtk::Box) {
    body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
}

pub(super) fn find_widget(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

#[cfg(test)]
mod tests;
