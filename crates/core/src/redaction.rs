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
        let decoded_key = decode_json_string_key(key);
        let after_key = key_end + 1;
        let Some(colon_index) = skip_ascii_whitespace(value, after_key) else {
            output.push_str(&value[key_start..after_key]);
            index = after_key;
            continue;
        };
        if !value[colon_index..].starts_with(':') || !is_sensitive_key_or_flag(&decoded_key) {
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

fn decode_json_string_key(key: &str) -> String {
    serde_json::from_str::<String>(&format!("\"{key}\"")).unwrap_or_else(|_| key.to_owned())
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
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in value[value_start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return value_start + offset + ch.len_utf8();
                }
            }
            ',' | '}' | ']' | '\n' | '\r' if depth == 0 => return value_start + offset,
            _ => {}
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
    if part.contains('{')
        || part.contains('[')
        || part.contains(',')
        || part.contains('?')
        || part.contains('&')
        || part.contains('#')
    {
        return redact_embedded_assignment_secret(part);
    }
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

fn redact_embedded_assignment_secret(part: &str) -> Option<String> {
    let mut output = String::with_capacity(part.len());
    let mut cursor = 0;
    let mut changed = false;

    while let Some((_key_start, _separator_index, value_start)) =
        find_embedded_assignment_secret(part, cursor)
    {
        output.push_str(&part[cursor..value_start]);
        output.push_str("[redacted]");
        cursor = embedded_assignment_value_end(part, value_start);
        changed = true;
    }

    if changed {
        output.push_str(&part[cursor..]);
        Some(output)
    } else {
        None
    }
}

fn find_embedded_assignment_secret(part: &str, start: usize) -> Option<(usize, usize, usize)> {
    for (offset, separator) in part[start..].char_indices() {
        if !matches!(separator, '=' | ':') {
            continue;
        }
        let separator_index = start + offset;
        let key_start = part[..separator_index]
            .rfind(['"', '\'', ' ', ',', '{', '[', ';', '?', '&', '#'])
            .map(|offset| offset + 1)
            .unwrap_or(0);
        if key_start < start {
            continue;
        }

        let key = &part[key_start..separator_index];
        let value_start = separator_index + separator.len_utf8();
        if !key.is_empty() && value_start < part.len() && is_sensitive_key_or_flag(key) {
            return Some((key_start, separator_index, value_start));
        }
    }
    None
}

fn embedded_assignment_value_end(part: &str, value_start: usize) -> usize {
    let quote_or_whitespace_end = part[value_start..]
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace() || matches!(ch, '"' | '\'' | '&' | '#'))
        .map(|(offset, _)| value_start + offset)
        .unwrap_or(part.len());

    let next_assignment_start =
        find_embedded_assignment_secret(part, value_start).map(|(key_start, _, _)| {
            part[..key_start]
                .char_indices()
                .rev()
                .find_map(|(index, ch)| matches!(ch, ',' | ';' | '&' | '#').then_some(index))
                .unwrap_or(key_start)
        });

    next_assignment_start
        .map(|index| index.min(quote_or_whitespace_end))
        .unwrap_or(quote_or_whitespace_end)
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

    #[test]
    fn redacts_json_escaped_sensitive_keys() {
        let raw = r#"{"api\u005fkey":"escaped-secret","safe":"visible"}"#;

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("escaped-secret"));
        assert_eq!(redacted, r#"{"api\u005fkey":[redacted],"safe":"visible"}"#);
    }

    #[test]
    fn redacts_complete_structured_secret_values() {
        let raw = r#"{"token":["first","second"],"secret":{"a":"one","b":"two"},"safe":"visible"}"#;

        let redacted = redact_sensitive_text(raw);

        for leaked in ["first", "second", "one", "two"] {
            assert!(!redacted.contains(leaked), "{leaked} leaked in {redacted}");
        }
        assert_eq!(
            redacted,
            r#"{"token":[redacted],"secret":[redacted],"safe":"visible"}"#
        );
    }

    #[test]
    fn redacts_embedded_assignment_inside_json_string_value() {
        let raw = r#"{"input":"OPENAI_API_KEY=sk-secret bearer ghp_secret --password swordfish"}"#;

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("sk-secret"));
        assert!(redacted.contains("OPENAI_API_KEY=[redacted]"));
    }

    #[test]
    fn redacts_embedded_assignment_values_with_punctuation() {
        let raw = r#"{"input":"TOKEN=first,second]tail","safe":"visible"}"#;

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("first"));
        assert!(!redacted.contains("second"));
        assert!(!redacted.contains("tail"));
        assert_eq!(redacted, r#"{"input":"TOKEN=[redacted]","safe":"visible"}"#);
    }

    #[test]
    fn redacts_multiple_embedded_assignments_in_one_token() {
        let raw = r#"{"input":"TOKEN=one,API_KEY=two,visible"}"#;

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("one"));
        assert!(!redacted.contains("two"));
        assert_eq!(
            redacted,
            r#"{"input":"TOKEN=[redacted],API_KEY=[redacted]"}"#
        );
    }

    #[test]
    fn redacts_url_query_secret_assignments() {
        let raw = "https://example.test/callback?api_key=sk-secret&token=ghp-secret&safe=visible";

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("sk-secret"));
        assert!(!redacted.contains("ghp-secret"));
        assert_eq!(
            redacted,
            "https://example.test/callback?api_key=[redacted]&token=[redacted]&safe=visible"
        );
    }

    #[test]
    fn redacts_url_fragment_secret_assignments() {
        let raw = "https://example.test/callback#token=ghp-secret&safe=visible";

        let redacted = redact_sensitive_text(raw);

        assert!(!redacted.contains("ghp-secret"));
        assert_eq!(
            redacted,
            "https://example.test/callback#token=[redacted]&safe=visible"
        );
    }
}
