pub fn redact_sensitive_text(value: &str) -> String {
    let mut redact_next = false;
    let mut parts = Vec::new();
    for part in value.split_whitespace() {
        if redact_next {
            parts.push("[redacted]".to_owned());
            redact_next = false;
            continue;
        }

        if is_bearer_marker(part) {
            parts.push(part.to_owned());
            redact_next = true;
            continue;
        }

        if let Some(redacted) = redact_assignment_secret(part) {
            parts.push(redacted);
            continue;
        }

        if is_flag_secret(part) {
            parts.push(part.to_owned());
            redact_next = true;
            continue;
        }

        parts.push(part.to_owned());
    }
    parts.join(" ")
}

fn redact_assignment_secret(part: &str) -> Option<String> {
    let (key, _) = part.split_once('=')?;
    is_sensitive_key_or_flag(key).then(|| format!("{key}=[redacted]"))
}

fn is_flag_secret(part: &str) -> bool {
    part.starts_with("--") && is_sensitive_key_or_flag(part.trim_start_matches('-'))
}

fn is_bearer_marker(part: &str) -> bool {
    part.trim_matches(|ch: char| matches!(ch, '\'' | '"' | ':' | ',' | '{' | '}' | '[' | ']'))
        .eq_ignore_ascii_case("bearer")
}

fn is_sensitive_key_or_flag(key: &str) -> bool {
    let normalized = key
        .trim_matches(|ch: char| matches!(ch, '\'' | '"' | ':' | ',' | '{' | '}' | '[' | ']'))
        .trim_start_matches('-')
        .replace('-', "_")
        .to_ascii_lowercase();
    normalized == "token"
        || normalized.ends_with("_token")
        || normalized == "api_key"
        || normalized.ends_with("_api_key")
        || normalized == "key"
        || normalized.ends_with("_key")
        || normalized == "password"
        || normalized.ends_with("_password")
        || normalized == "secret"
        || normalized.ends_with("_secret")
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive_text;

    #[test]
    fn redacts_common_secret_forms() {
        let redacted = redact_sensitive_text(
            "OPENAI_API_KEY=sk-secret bearer ghp_secret --password swordfish --token abc",
        );

        assert_eq!(
            redacted,
            "OPENAI_API_KEY=[redacted] bearer [redacted] --password [redacted] --token [redacted]"
        );
    }
}
