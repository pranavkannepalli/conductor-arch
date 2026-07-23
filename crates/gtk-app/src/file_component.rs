use gtk::prelude::*;
use gtk::{Button, Label};
use std::path::PathBuf;
use std::rc::Rc;

pub(crate) type OpenWorkspaceFile = Rc<dyn Fn(&str)>;

pub(crate) fn workspace_file_link_component(
    label: &str,
    target_path: &str,
    open_file: OpenWorkspaceFile,
) -> Button {
    let button = Button::new();
    button.add_css_class("workspace-file-link");
    button.set_tooltip_text(Some(&workspace_file_link_tooltip_text(target_path)));
    let label_widget = Label::new(Some(label));
    label_widget.set_xalign(0.0);
    label_widget.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    button.set_child(Some(&label_widget));

    let target_path = target_path.to_owned();
    button.connect_clicked(move |_| open_file(target_path.as_str()));
    button
}

pub(crate) fn workspace_file_link_tooltip_text(target_path: &str) -> String {
    format!("Open {target_path}")
}

pub(crate) fn markdown_file_link_target(url: &str) -> Option<PathBuf> {
    let target = url.trim();
    if target.is_empty()
        || target.starts_with('#')
        || target.contains("://")
        || target.starts_with("mailto:")
    {
        return None;
    }

    let without_fragment = target.split_once('#').map_or(target, |(path, _)| path);
    let without_line = strip_markdown_line_suffix(without_fragment);
    let path = without_line.trim();
    if path.is_empty() || !workspace_file_target_looks_path_like(path) {
        return None;
    }
    Some(PathBuf::from(path))
}

fn strip_markdown_line_suffix(target: &str) -> &str {
    let mut end = target.len();
    for _ in 0..2 {
        let Some(colon) = target[..end].rfind(':') else {
            break;
        };
        let suffix = &target[colon + 1..end];
        if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }
        end = colon;
    }
    &target[..end]
}

fn workspace_file_target_looks_path_like(target: &str) -> bool {
    target.starts_with('/')
        || target.starts_with("./")
        || target.starts_with("../")
        || target.contains('/')
        || target.contains('\\')
        || target.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_file_link_target_accepts_workspace_paths_and_line_suffixes() {
        assert_eq!(
            markdown_file_link_target("crates/gtk-app/src/session_surface.rs"),
            Some(PathBuf::from("crates/gtk-app/src/session_surface.rs"))
        );
        assert_eq!(
            markdown_file_link_target("src/main.rs:42"),
            Some(PathBuf::from("src/main.rs"))
        );
        assert_eq!(
            markdown_file_link_target("./README.md#install"),
            Some(PathBuf::from("./README.md"))
        );
    }

    #[test]
    fn markdown_file_link_target_rejects_external_links_and_fragments() {
        assert_eq!(
            markdown_file_link_target("https://example.com/README.md"),
            None
        );
        assert_eq!(markdown_file_link_target("#local-heading"), None);
        assert_eq!(markdown_file_link_target("mailto:test@example.com"), None);
    }

    #[test]
    fn workspace_file_link_tooltip_names_open_target() {
        assert_eq!(
            workspace_file_link_tooltip_text("README.md"),
            "Open README.md"
        );
    }
}
