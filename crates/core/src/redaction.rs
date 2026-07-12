pub fn redact_sensitive_text(value: &str) -> String {
    let value = redact_structured_secrets(value);
    let mut redact_next = false;
    let mut output = String::new();
    for part in split_preserving_whitespace(&value) {
        if part.chars().all(char::is_whitespace) {
            output.push_str(part);
            continue;
        }

        if redact_next {
            output.push_str("[redacted]");
            redact_next = false;
            continue;
        }

        if is_bearer_marker(part) {
            output.push_str(part);
            redact_next = true;
            continue;
        }

        if let Some(redacted) = redact_assignment_secret(part) {
            output.push_str(&redacted);
            continue;
        }

        if is_sensitive_key_marker(part) {
            output.push_str(part);
            redact_next = true;
            continue;
        }

        if is_flag_secret(part) {
            output.push_str(part);
            redact_next = true;
            continue;
        }

        output.push_str(part);
    }
    output
}

fn redact_structured_secrets(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let Some(key_start) = value[index..].find('"').map(|offset| index + offset) else {
            output.push_str(&value[index..]);
            break;
        };
        output.push_str(&value[index..key_start]);

        let Some(key_end) = quoted_string_end(value, key_start) else {
            output.push_str(&value[key_start..]);
            break;
        };
        let key = &value[key_start + 1..key_end];
        let after_key = key_end + 1;
        let Some(colon_index) = skip_ascii_whitespace(value, after_key) else {
            output.push_str(&value[key_start..after_key]);
            index = after_key;
            continue;
        };
        if !value[colon_index..].starts_with(':') || !is_sensitive_key_or_flag(key) {
            output.push_str(&value[key_start..after_key]);
            index = after_key;
            continue;
        }

        let value_start = skip_ascii_whitespace(value, colon_index + 1).unwrap_or(value.len());
        output.push_str(&value[key_start..value_start]);
        output.push_str("[redacted]");
        index = structured_value_end(value, value_start);
    }
    output
}

fn quoted_string_end(value: &str, quote_index: usize) -> Option<usize> {
    let mut escaped = false;
    for (offset, ch) in value[quote_index + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(quote_index + 1 + offset),
            _ => {}
        }
    }
    None
}

fn skip_ascii_whitespace(value: &str, mut index: usize) -> Option<usize> {
    while index < value.len() {
        let ch = value[index..].chars().next()?;
        if !ch.is_ascii_whitespace() {
            return Some(index);
        }
        index += ch.len_utf8();
    }
    None
}

fn structured_value_end(value: &str, value_start: usize) -> usize {
    if value_start >= value.len() {
        return value_start;
    }
    if value[value_start..].starts_with('"') {
        return quoted_string_end(value, value_start)
            .map(|end| end + 1)
            .unwrap_or(value.len());
    }
    for (offset, ch) in value[value_start..].char_indices() {
        if matches!(ch, ',' | '}' | ']' | '\n' | '\r') {
            return value_start + offset;
        }
    }
    value.len()
}

fn split_preserving_whitespace(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_whitespace = None;
    for (index, ch) in value.char_indices() {
        let whitespace = ch.is_whitespace();
        match in_whitespace {
            None => in_whitespace = Some(whitespace),
            Some(current) if current != whitespace => {
                parts.push(&value[start..index]);
                start = index;
                in_whitespace = Some(whitespace);
            }
            _ => {}
        }
    }
    if start < value.len() {
        parts.push(&value[start..]);
    }
    parts
}

fn redact_assignment_secret(part: &str) -> Option<String> {
    for separator in ['=', ':'] {
        let Some((key, value)) = part.split_once(separator) else {
            continue;
        };
        if !value.is_empty() && is_sensitive_key_or_flag(key) {
            return Some(format!("{key}{separator}[redacted]"));
        }
    }
    None
}

fn is_sensitive_key_marker(part: &str) -> bool {
    let marker = part
        .strip_suffix('=')
        .or_else(|| part.strip_suffix(':'))
        .unwrap_or(part);
    marker.len() != part.len() && is_sensitive_key_or_flag(marker)
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

    #[test]
    fn redacts_structured_secrets_without_collapsing_whitespace() {
        let raw = concat!(
            "TOKEN=abc\n",
            "\"level\":\"info\",\"api_key\":\"json secret with spaces\",\n",
            "password: yaml-secret\n",
            "safe value\n",
        );

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("abc"));
        assert!(!redacted.contains("json secret with spaces"));
        assert!(!redacted.contains("yaml-secret"));
        assert!(redacted.contains("TOKEN=[redacted]\n"));
        assert!(redacted.contains("\"api_key\":[redacted],\n"));
        assert!(redacted.contains("password: [redacted]\n"));
        assert!(redacted.contains("safe value\n"));
    }

    #[test]
    fn redacts_json_like_sensitive_values_anywhere_in_token() {
        let raw = r#"{"ok":true,"api_key":"sk secret with spaces","nested":{"refresh_token":"refresh-secret"}}"#;

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("sk secret"));
        assert!(!redacted.contains("refresh-secret"));
        assert_eq!(
            redacted,
            r#"{"ok":true,"api_key":[redacted],"nested":{"refresh_token":[redacted]}}"#
        );
    }
}
