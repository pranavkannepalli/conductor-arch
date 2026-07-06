pub fn explicit_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
    )
}

pub fn enabled(name: &str) -> bool {
    explicit_truthy(std::env::var(name).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::explicit_truthy;

    #[test]
    fn explicit_truthy_accepts_only_intentional_boolean_values() {
        assert!(!explicit_truthy(None));
        assert!(!explicit_truthy(Some("")));
        assert!(!explicit_truthy(Some("0")));
        assert!(!explicit_truthy(Some("false")));
        assert!(explicit_truthy(Some("1")));
        assert!(explicit_truthy(Some("true")));
        assert!(explicit_truthy(Some(" TRUE ")));
        assert!(explicit_truthy(Some("yes")));
        assert!(explicit_truthy(Some("on")));
    }
}
