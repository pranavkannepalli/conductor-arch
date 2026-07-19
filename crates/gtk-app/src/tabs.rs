use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Image, Label, Orientation};

use crate::buttons::resolve_icon_name;

const TAB_SHELL_CLASS: &str = "ws-tab-shell";
const TAB_LABEL_CLASS: &str = "ws-tab-label";
const TAB_ACTIVE_CLASS: &str = "ws-tab-active";
const TAB_RUNNING_CLASS: &str = "ws-tab-running";
const TAB_UNREAD_CLASS: &str = "ws-tab-unread";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabTone {
    Normal,
    Running,
    Unread,
}

pub(crate) fn standard_tab(label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_accessible_role(gtk::AccessibleRole::Tab);
    button.add_css_class(TAB_SHELL_CLASS);
    let label = gtk::Label::new(Some(label));
    label.add_css_class(TAB_LABEL_CLASS);
    button.set_child(Some(&label));
    button
}

pub(crate) fn closable_tab_surface(label: &str) -> (GBox, Button) {
    let shell = GBox::new(Orientation::Horizontal, 6);
    shell.add_css_class(TAB_SHELL_CLASS);
    shell.set_valign(Align::Center);

    let label = Label::new(Some(label));
    label.add_css_class(TAB_LABEL_CLASS);
    label.set_valign(Align::Center);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    shell.append(&label);

    let close = Button::new();
    close.add_css_class("ws-tab-close-button");
    close.set_valign(Align::Center);
    close.set_tooltip_text(Some("Close tab"));
    let close_icon = Image::from_icon_name(resolve_icon_name("window-close-symbolic"));
    close_icon.add_css_class("ws-tab-close-icon");
    close_icon.set_valign(Align::Center);
    close.set_child(Some(&close_icon));
    shell.append(&close);

    (shell, close)
}

pub(crate) fn set_standard_tab_active(button: &gtk::Button, active: bool) {
    if active {
        button.add_css_class(TAB_ACTIVE_CLASS);
    } else {
        button.remove_css_class(TAB_ACTIVE_CLASS);
    }
    button.update_state(&[gtk::accessible::State::Selected(Some(active))]);
}

pub(crate) fn set_tab_tone<W: IsA<gtk::Widget>>(widget: &W, tone: TabTone) {
    widget.remove_css_class(TAB_RUNNING_CLASS);
    widget.remove_css_class(TAB_UNREAD_CLASS);
    if let Some(class_name) = tab_tone_class(tone) {
        widget.add_css_class(class_name);
    }
}

pub(crate) fn tab_tone_class(tone: TabTone) -> Option<&'static str> {
    match tone {
        TabTone::Normal => None,
        TabTone::Running => Some(TAB_RUNNING_CLASS),
        TabTone::Unread => Some(TAB_UNREAD_CLASS),
    }
}

pub(crate) fn standard_tab_strip() -> (gtk::ScrolledWindow, gtk::Box) {
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    tabs.set_accessible_role(gtk::AccessibleRole::TabList);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    scroll.set_hexpand(true);
    scroll.set_propagate_natural_width(false);
    scroll.set_child(Some(&tabs));
    (scroll, tabs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_tabs_reuse_workspace_chat_tab_classes() {
        assert_eq!(TAB_SHELL_CLASS, "ws-tab-shell");
        assert_eq!(TAB_LABEL_CLASS, "ws-tab-label");
        assert_eq!(TAB_ACTIVE_CLASS, "ws-tab-active");
    }

    #[test]
    fn tab_tones_map_to_shared_workspace_tab_classes() {
        assert_eq!(tab_tone_class(TabTone::Normal), None);
        assert_eq!(tab_tone_class(TabTone::Running), Some("ws-tab-running"));
        assert_eq!(tab_tone_class(TabTone::Unread), Some("ws-tab-unread"));
    }
}
