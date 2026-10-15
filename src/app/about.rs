//! Static About content; no polling, settings writes or scene work.
use super::style;
mod system;
use gtk::prelude::*;

const LINKS: [(&str, &str, &str, &str, &str); 3] = [
    (
        "about-github",
        "GitHub",
        "github.com/AzizStark/talis",
        "https://github.com/AzizStark/talis",
        include_str!("../../assets/ui/icons/github.svg"),
    ),
    (
        "about-email",
        "Email",
        "rrahman182@gmail.com",
        "mailto:rrahman182@gmail.com",
        include_str!("../../assets/ui/icons/mail.svg"),
    ),
    (
        "about-support",
        "Buy Me a Coffee",
        "Support Talis and future creations",
        "https://buymeacoffee.com/azizstark",
        include_str!("../../assets/ui/icons/coffee.svg"),
    ),
];

fn link(name: &str, title: &str, detail: &str, uri: &str, icon: &'static str) -> gtk::LinkButton {
    let button = gtk::LinkButton::with_label(uri, title);
    button.set_widget_name(name);
    button.add_css_class("about-link");
    if name == "about-support" {
        button.add_css_class("about-support");
    }
    button.set_hexpand(true);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let colour = if name == "about-support" {
        "#c7ab83"
    } else {
        "#b8c3c6"
    };
    let icon = style::svg_icon(icon, 20, colour);
    content.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    text.append(&style::label(title, "setting-title"));
    text.append(&style::label(detail, "setting-detail"));
    content.append(&text);
    let arrow = gtk::Image::from_icon_name("go-next-symbolic");
    arrow.set_pixel_size(14);
    content.append(&arrow);
    button.set_child(Some(&content));
    // Native LinkButton activation opens the browser/mail handler only on a
    // user action. No custom URI launcher, timers or configuration writes.
    button
}

fn reference(name: &str, title: &str, detail: &str, uri: &str) -> gtk::LinkButton {
    let button = gtk::LinkButton::with_label(uri, &format!("{title}\n{detail}"));
    button.set_widget_name(name);
    button.add_css_class("about-link");
    button.set_hexpand(true);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.append(&style::label(title, "setting-title"));
    text.append(&style::label(detail, "setting-detail"));
    button.set_child(Some(&text));
    button
}

fn reference_section(
    title: &str,
    name: &str,
    entries: &[(&str, &str, &str, &str)],
    note: Option<&str>,
) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for &(name, title, detail, uri) in entries {
        content.append(&reference(name, title, detail, uri));
    }
    if let Some(note) = note {
        let note = style::label(note, "setting-detail");
        note.set_margin_start(18);
        note.set_margin_end(18);
        note.set_margin_top(6);
        note.set_margin_bottom(12);
        content.append(&note);
    }
    let section = style::section(title, &content);
    section.set_widget_name(name);
    section
}

fn license() -> gtk::Box {
    reference_section(
        "License",
        "about-license",
        &[
            (
                "about-code-license",
                "GNU GPL v3.0",
                "Application code",
                "https://www.gnu.org/licenses/gpl-3.0.html",
            ),
            (
                "about-scene-license",
                "CC BY 4.0",
                "Original scene assets",
                "https://creativecommons.org/licenses/by/4.0/",
            ),
        ],
        None,
    )
}

fn credits() -> gtk::Box {
    reference_section(
        "Credits",
        "about-credits",
        &[
            (
                "about-icon-credit",
                "Tabler Icons",
                "Paweł Kuna · MIT",
                "https://github.com/tabler/tabler-icons/blob/main/LICENSE",
            ),
            (
                "about-artwork-credit",
                "Suvimaastik",
                "Lilly Walther (1936) · Tartu Art Museum\nEuropeana / Unsplash · CC0",
                "https://unsplash.com/photos/a-winding-path-leads-to-a-cottage-surrounded-by-trees-QSO-ewOX860",
            ),
            (
                "about-photo-credit",
                "Vinyl record photograph",
                "David DINTSH · Unsplash License",
                "https://unsplash.com/photos/close-up-of-a-vinyl-record-playing-on-a-turntable-XdRf3vsLh5Y",
            ),
            (
                "about-flower-credit",
                "Flower model",
                "Ramin98 · CGTrader\nRoyalty Free License (no AI)",
                "https://www.cgtrader.com/free-3d-models/furniture/tableware/concrete-pot-plant-and-flower",
            ),
        ],
        Some("Third-party assets retain their own licenses."),
    )
}

pub(super) fn page() -> gtk::Box {
    let page = style::page("About");
    page.set_spacing(12);
    let system = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(18)
        .build();
    system.add_css_class("settings-card");
    system.add_css_class("about-system");
    for (row, (title, name, value)) in system::details().into_iter().enumerate() {
        system.attach(&style::label(title, "setting-detail"), 0, row as i32, 1, 1);
        let label = style::label(&value, "setting-title");
        label.set_widget_name(name);
        label.set_hexpand(true);
        label.set_selectable(true);
        system.attach(&label, 1, row as i32, 1, 1);
    }
    let links = gtk::Box::new(gtk::Orientation::Vertical, 0);
    links.add_css_class("settings-card");
    for (name, title, detail, uri, icon) in LINKS {
        links.append(&link(name, title, detail, uri, icon));
    }
    page.append(&links);
    page.append(&system);
    page.append(&license());
    page.append(&credits());
    let space = gtk::Box::new(gtk::Orientation::Vertical, 0);
    space.set_vexpand(true);
    page.append(&space);
    let credit = style::label("Crafted with <3 by Aziz", "setting-detail");
    credit.set_widget_name("about-credit");
    credit.set_xalign(0.5);
    page.append(&credit);
    page
}
