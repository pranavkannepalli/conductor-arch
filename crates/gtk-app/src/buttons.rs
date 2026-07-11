use gtk::prelude::*;
use gtk::{Align, Button, Label, Widget};

pub(crate) fn text_button(label: &str) -> Button {
    let button = Button::with_label(label);
    style_text_button(&button);
    button
}

pub(crate) fn menu_text_button(label: &str) -> Button {
    let button = Button::new();
    style_text_button(&button);
    button.add_css_class("chat-menu-item");

    let label = Label::new(Some(label));
    label.add_css_class("chat-menu-item-label");
    label.set_xalign(0.0);
    label.set_halign(Align::Fill);
    label.set_hexpand(true);
    button.set_child(Some(&label));

    button
}

pub(crate) fn icon_button(icon: &str, tooltip: &str) -> Button {
    let button = Button::from_icon_name(resolve_icon_name(icon));
    style_icon_button(&button);
    button.set_tooltip_text(Some(tooltip));
    button
}

pub(crate) fn resolve_icon_name(icon: &str) -> &'static str {
    match icon {
        "send-symbolic" => "mail-send-symbolic",
        "focus-windows-symbolic" => "find-location-symbolic",
        "code-symbolic" => "application-x-executable-symbolic",
        "zed-symbolic" => "accessories-text-editor-symbolic",
        "sidebar-hide-symbolic" => "view-left-pane-symbolic",
        "sidebar-show-symbolic" => "view-left-pane-symbolic",
        "emblem-system-symbolic" => "preferences-system-symbolic",
        "document-new-symbolic" => "document-new-symbolic",
        "folder-new-symbolic" => "folder-new-symbolic",
        "folder-open-symbolic" => "folder-open-symbolic",
        "network-workgroup-symbolic" => "network-workgroup-symbolic",
        "open-menu-symbolic" => "open-menu-symbolic",
        "pan-down-symbolic" => "pan-down-symbolic",
        "dialog-error-symbolic" => "dialog-error-symbolic",
        "application-x-executable-symbolic" => "application-x-executable-symbolic",
        "accessories-text-editor-symbolic" => "accessories-text-editor-symbolic",
        "folder-symbolic" => "folder-symbolic",
        "go-next-symbolic" => "go-next-symbolic",
        "go-previous-symbolic" => "go-previous-symbolic",
        "go-home-symbolic" => "go-home-symbolic",
        "view-list-symbolic" => "view-list-symbolic",
        "view-filter-symbolic" => "view-filter-symbolic",
        "list-add-symbolic" => "list-add-symbolic",
        "list-drag-handle-symbolic" => "list-drag-handle-symbolic",
        "utilities-terminal-symbolic" => "utilities-terminal-symbolic",
        "view-more-symbolic" => "view-more-symbolic",
        "window-close-symbolic" => "window-close-symbolic",
        "window-minimize-symbolic" => "window-minimize-symbolic",
        "window-maximize-symbolic" => "window-maximize-symbolic",
        _ => "application-x-executable-symbolic",
    }
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

#[cfg(test)]
mod tests {
    use super::resolve_icon_name;

    #[test]
    fn resolves_custom_icon_names_to_common_symbolic_fallbacks() {
        assert_eq!(resolve_icon_name("send-symbolic"), "mail-send-symbolic");
        assert_eq!(
            resolve_icon_name("focus-windows-symbolic"),
            "find-location-symbolic"
        );
        assert_eq!(
            resolve_icon_name("code-symbolic"),
            "application-x-executable-symbolic"
        );
        assert_eq!(
            resolve_icon_name("zed-symbolic"),
            "accessories-text-editor-symbolic"
        );
        assert_eq!(
            resolve_icon_name("sidebar-hide-symbolic"),
            "view-left-pane-symbolic"
        );
    }

    #[test]
    fn preserves_standard_icon_names() {
        assert_eq!(resolve_icon_name("folder-symbolic"), "folder-symbolic");
        assert_eq!(resolve_icon_name("go-next-symbolic"), "go-next-symbolic");
        assert_eq!(resolve_icon_name("open-menu-symbolic"), "open-menu-symbolic");
        assert_eq!(
            resolve_icon_name("utilities-terminal-symbolic"),
            "utilities-terminal-symbolic"
        );
    }
}
