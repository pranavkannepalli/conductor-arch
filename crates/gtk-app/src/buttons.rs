use gtk::prelude::*;
use gtk::{Button, Widget};

pub(crate) fn text_button(label: &str) -> Button {
    let button = Button::with_label(label);
    style_text_button(&button);
    button
}

pub(crate) fn icon_button(icon: &str, tooltip: &str) -> Button {
    let button = Button::from_icon_name(icon);
    style_icon_button(&button);
    button.set_tooltip_text(Some(tooltip));
    button
}

pub(crate) fn style_text_button<W: IsA<Widget>>(button: &W) {
    button.add_css_class("text-button");
}

pub(crate) fn style_icon_button<W: IsA<Widget>>(button: &W) {
    button.add_css_class("icon-button");
}

pub(crate) fn style_text_toggle_button<W: IsA<Widget>>(button: &W) {
    button.add_css_class("text-button");
}
