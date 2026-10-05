//! Small widget helpers shared by every screen.

use gtk4 as gtk;
use gtk4::prelude::*;

/// Flat icon button used in toolbars. The tooltip doubles as the accessible name.
pub fn tool_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).css_classes(["tool"]).build();
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

pub fn tool_toggle(icon: &str, tooltip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::builder().icon_name(icon).tooltip_text(tooltip).css_classes(["tool"]).build();
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

/// Toolbar toggle with an icon and a text label (top bar panel buttons).
pub fn labelled_toggle(icon: &str, label: &str, tooltip: &str) -> gtk::ToggleButton {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    gtk::ToggleButton::builder().child(&content).tooltip_text(tooltip).css_classes(["tool"]).build()
}

/// Outlined pill: every text button in the app.
pub fn pill(label: &str) -> gtk::Button {
    gtk::Button::builder().label(label).css_classes(["pill-outline"]).build()
}

/// Toolbar toggle that shows a word. Words are clearer than icons for a beginner
/// and do not depend on the icon theme.
pub fn text_toggle(text: &str, tooltip: &str) -> gtk::ToggleButton {
    gtk::ToggleButton::builder().label(text).tooltip_text(tooltip).css_classes(["tool", "text"]).build()
}

pub fn text_button(text: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder().label(text).tooltip_text(tooltip).css_classes(["tool", "text"]).build()
}

pub fn divider() -> gtk::Box {
    gtk::Box::builder().css_classes(["bar-divider"]).build()
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder().label(text).css_classes(classes.to_vec()).build()
}

/// `1:02` or `1:02:03` for durations in lists.
pub fn short_duration(us: i64) -> String {
    let s = us.max(0) / 1_000_000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Look a named CSS colour up from a widget, so colours live only in tempo.css.
#[allow(deprecated)]
pub fn css_color(widget: &impl IsA<gtk::Widget>, name: &str) -> gtk::gdk::RGBA {
    widget.style_context().lookup_color(name).unwrap_or(gtk::gdk::RGBA::new(1.0, 0.0, 1.0, 1.0))
}
