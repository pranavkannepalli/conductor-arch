use adw::{Toast, ToastOverlay};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastVariant {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToastMessage {
    pub variant: ToastVariant,
    pub text: String,
}

impl ToastMessage {
    pub(crate) fn info(text: impl Into<String>) -> Self {
        Self {
            variant: ToastVariant::Info,
            text: text.into(),
        }
    }

    pub(crate) fn success(text: impl Into<String>) -> Self {
        Self {
            variant: ToastVariant::Success,
            text: text.into(),
        }
    }

    pub(crate) fn warning(text: impl Into<String>) -> Self {
        Self {
            variant: ToastVariant::Warning,
            text: text.into(),
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            variant: ToastVariant::Error,
            text: text.into(),
        }
    }

    pub(crate) fn timeout_seconds(&self) -> u32 {
        match self.variant {
            ToastVariant::Info | ToastVariant::Success => 4,
            ToastVariant::Warning => 6,
            ToastVariant::Error => 8,
        }
    }

    pub(crate) fn display_text(&self) -> String {
        match self.variant {
            ToastVariant::Error if !self.text.starts_with("Error: ") => {
                format!("Error: {}", self.text)
            }
            ToastVariant::Warning if !self.text.starts_with("Warning: ") => {
                format!("Warning: {}", self.text)
            }
            _ => self.text.clone(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ToastManager {
    overlay: ToastOverlay,
}

impl ToastManager {
    pub(crate) fn new(overlay: &ToastOverlay) -> Self {
        Self {
            overlay: overlay.clone(),
        }
    }

    pub(crate) fn show(&self, message: ToastMessage) {
        show_toast(&self.overlay, message);
    }
}

pub(crate) fn show_toast(overlay: &ToastOverlay, message: ToastMessage) {
    let toast = Toast::new(&message.display_text());
    toast.set_timeout(message.timeout_seconds());
    overlay.add_toast(toast);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_variants_use_expected_timeouts() {
        assert_eq!(ToastMessage::info("Saved").timeout_seconds(), 4);
        assert_eq!(ToastMessage::success("Done").timeout_seconds(), 4);
        assert_eq!(ToastMessage::warning("Check setup").timeout_seconds(), 6);
        assert_eq!(ToastMessage::error("Failed").timeout_seconds(), 8);
    }

    #[test]
    fn toast_variants_prefix_attention_copy_without_rewriting_success() {
        assert_eq!(
            ToastMessage::success("Chat finished").display_text(),
            "Chat finished"
        );
        assert_eq!(
            ToastMessage::error("Clone failed").display_text(),
            "Error: Clone failed"
        );
    }
}
