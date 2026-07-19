use gtk::prelude::*;
use gtk::Label;

const PAGE_TITLE_CLASS: &str = "text-page-title";
const SECTION_TITLE_CLASS: &str = "text-section-title";
const META_CLASS: &str = "text-meta";
const MONO_CLASS: &str = "text-mono";
const EMPTY_CLASS: &str = "text-empty";
const STATUS_CLASS: &str = "text-status";

pub(crate) fn page_title(text: &str) -> Label {
    label_with_class(text, PAGE_TITLE_CLASS)
}

pub(crate) fn section_title(text: &str) -> Label {
    label_with_class(text, SECTION_TITLE_CLASS)
}

pub(crate) fn meta_label(text: &str) -> Label {
    label_with_class(text, META_CLASS)
}

pub(crate) fn mono_label(text: &str) -> Label {
    label_with_class(text, MONO_CLASS)
}

pub(crate) fn empty_label(text: &str) -> Label {
    label_with_class(text, EMPTY_CLASS)
}

pub(crate) fn status_label(text: &str, class_name: &str) -> Label {
    let label = label_with_class(text, STATUS_CLASS);
    label.add_css_class(class_name);
    label
}

pub(crate) fn detail_label(text: &str) -> Label {
    label_with_class(text, "detail-label")
}

pub(crate) fn detail_value(text: &str) -> Label {
    label_with_class(text, "detail-value")
}

fn label_with_class(text: &str, class_name: &'static str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class(class_name);
    label
}

fn semantic_text_classes() -> [&'static str; 6] {
    [
        PAGE_TITLE_CLASS,
        SECTION_TITLE_CLASS,
        META_CLASS,
        MONO_CLASS,
        EMPTY_CLASS,
        STATUS_CLASS,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_helpers_use_semantic_classes() {
        assert_eq!(
            semantic_text_classes(),
            [
                "text-page-title",
                "text-section-title",
                "text-meta",
                "text-mono",
                "text-empty",
                "text-status",
            ]
        );
    }
}
