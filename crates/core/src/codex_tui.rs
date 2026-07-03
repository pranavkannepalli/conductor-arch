use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexInlineEvent {
    Tool(CodexToolCall),
    Skill(CodexSkillAnnouncement),
    FileReference(CodexFileReference),
    FileChange(CodexFileChange),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexToolCall {
    pub namespace: String,
    pub name: String,
    pub marker: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexSkillAnnouncement {
    pub skill: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexFileReference {
    pub path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexFileChange {
    pub action: CodexFileChangeAction,
    pub path: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub lines: Vec<CodexFileChangeLine>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexFileChangeAction {
    Added,
    Edited,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexFileChangeLine {
    pub kind: CodexFileChangeLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexFileChangeLineKind {
    Context,
    Added,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexContextUsage {
    pub percent: Option<u8>,
    pub used_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexParsedLine {
    Message { content: String },
    InlineEvent { event: CodexInlineEvent },
    ContextUsage { usage: CodexContextUsage },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMessageRole {
    User,
    Agent,
}

impl ScreenMessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenMessage {
    pub role: ScreenMessageRole,
    pub content: String,
}

pub fn encode_send_line(line: &str) -> Vec<u8> {
    let mut encoded = line.as_bytes().to_vec();
    encoded.push(b'\r');
    encoded
}

pub fn is_trust_prompt_visible(screen: &str, trust_enabled: bool) -> bool {
    trust_enabled
        && screen.contains("Do you trust the contents of this directory?")
        && screen.contains("1. Yes, continue")
}

pub fn detect_directory_trust_prompt(screen: &str) -> bool {
    is_trust_prompt_visible(screen, true)
}

pub fn parse_codex_inline_event(line: &str) -> Option<CodexInlineEvent> {
    parse_codex_tool_call(line)
        .map(CodexInlineEvent::Tool)
        .or_else(|| parse_codex_skill_announcement(line).map(CodexInlineEvent::Skill))
        .or_else(|| parse_codex_file_reference(line).map(CodexInlineEvent::FileReference))
        .or_else(|| parse_codex_file_change(line).map(CodexInlineEvent::FileChange))
}

pub fn parse_codex_context_usage(line: &str) -> Option<CodexContextUsage> {
    parse_context_window_percent(line).or_else(|| parse_context_token_fraction(line))
}

pub fn parse_codex_file_change_block(text: &str) -> Option<CodexFileChange> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let mut change = parse_codex_file_change(header)?;
    change.lines = lines.filter_map(parse_codex_file_change_line).collect();
    Some(change)
}

pub fn parse_codex_structured_lines(text: &str) -> Vec<CodexParsedLine> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some(usage) = parse_codex_context_usage(trimmed) {
                return Some(CodexParsedLine::ContextUsage { usage });
            }
            if let Some(event) = parse_codex_inline_event(trimmed) {
                return Some(CodexParsedLine::InlineEvent { event });
            }
            Some(CodexParsedLine::Message {
                content: trimmed.to_owned(),
            })
        })
        .collect()
}

pub fn parse_codex_screen_messages(screen: &str) -> Vec<ScreenMessage> {
    let lines = relevant_codex_screen_lines(screen);
    let lines = lines.iter().map(String::as_str).collect::<Vec<_>>();
    let mut messages = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index];

        if let Some(role) = parse_box_role(line) {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() {
                let line = lines[index];
                if is_box_bottom(line) {
                    index += 1;
                    break;
                }
                if let Some(content) = parse_box_content(line) {
                    body.push(content);
                }
                index += 1;
            }
            push_message(&mut messages, role, body);
            continue;
        }

        if is_live_user_prompt_line(line) {
            push_live_prompt_message(&mut messages, ScreenMessageRole::User, line);
            index += 1;
            while index < lines.len() {
                if is_live_user_prompt_line(lines[index])
                    || is_live_agent_prompt_line(lines[index])
                    || is_box_header_line(lines[index])
                {
                    break;
                }
                if let Some(first_line) = parse_live_agent_bullet(lines[index]) {
                    let mut body = vec![first_line];
                    index += 1;
                    while index < lines.len() {
                        if is_live_user_prompt_line(lines[index])
                            || is_live_agent_prompt_line(lines[index])
                            || is_box_header_line(lines[index])
                        {
                            break;
                        }
                        if let Some(content) = parse_live_continuation(lines[index]) {
                            body.push(content);
                            index += 1;
                            continue;
                        }
                        if is_transient_bullet_line(lines[index]) {
                            index += 1;
                            continue;
                        }
                        break;
                    }
                    push_message(&mut messages, ScreenMessageRole::Agent, body);
                    continue;
                }
                index += 1;
            }
            continue;
        }

        if is_live_bullet_user_prompt(line, lines.get(index + 1).copied()) {
            push_live_prompt_message(&mut messages, ScreenMessageRole::User, line);
            index += 1;
            while index < lines.len() {
                if is_live_user_prompt_line(lines[index])
                    || is_live_bullet_user_prompt(lines[index], lines.get(index + 1).copied())
                    || is_box_header_line(lines[index])
                {
                    break;
                }
                if let Some(first_line) = parse_live_agent_prompt(lines[index]) {
                    let mut body = vec![first_line];
                    index += 1;
                    while index < lines.len() {
                        if is_live_user_prompt_line(lines[index])
                            || is_live_bullet_user_prompt(
                                lines[index],
                                lines.get(index + 1).copied(),
                            )
                            || is_box_header_line(lines[index])
                        {
                            break;
                        }
                        if let Some(content) = parse_live_continuation(lines[index]) {
                            body.push(content);
                            index += 1;
                            continue;
                        }
                        break;
                    }
                    push_message(&mut messages, ScreenMessageRole::Agent, body);
                    continue;
                }
                index += 1;
            }
            continue;
        }

        if is_ignorable_transcript_line(line) {
            index += 1;
            continue;
        }

        let mut body = Vec::new();
        while index < lines.len() {
            let line = lines[index];
            if is_box_header_line(line)
                || is_live_user_prompt_line(line)
                || is_live_bullet_user_prompt(line, lines.get(index + 1).copied())
                || is_live_agent_prompt_line(line)
            {
                break;
            }
            if line.trim().is_empty() {
                body.push(String::new());
                index += 1;
                continue;
            }
            if is_ignorable_transcript_line(line) {
                if body.is_empty() {
                    index += 1;
                    continue;
                }
                break;
            }
            let trimmed = line.trim();
            if body.is_empty() {
                if let Some(bullet) = trimmed.strip_prefix('•') {
                    body.push(bullet.trim_start().to_owned());
                } else {
                    body.push(trimmed.to_owned());
                }
            } else {
                body.push(trimmed.to_owned());
            }
            index += 1;
        }
        push_message(&mut messages, ScreenMessageRole::Agent, body);
    }

    messages
}

fn parse_codex_tool_call(line: &str) -> Option<CodexToolCall> {
    const KNOWN_NAMESPACES: &[&str] = &[
        "functions",
        "web",
        "multi_agent_v1",
        "tool_search",
        "image_gen",
        "multi_tool_use",
    ];

    line.split_whitespace()
        .filter_map(normalize_tool_token)
        .find_map(|token| {
            parse_mcp_tool_marker(token)
                .or_else(|| parse_dotted_tool_marker(token, KNOWN_NAMESPACES, line))
        })
}

fn parse_codex_skill_announcement(line: &str) -> Option<CodexSkillAnnouncement> {
    if let Some(skill_read) = parse_skill_read_line(line) {
        return Some(skill_read);
    }
    let rest = line.strip_prefix("Using ")?;
    let (skill, message) = rest.split_once(" to ")?;
    if !is_skill_name(skill) || message.trim().is_empty() {
        return None;
    }
    Some(CodexSkillAnnouncement {
        skill: skill.to_owned(),
        message: message.trim().to_owned(),
    })
}

fn parse_skill_read_line(line: &str) -> Option<CodexSkillAnnouncement> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("Read SKILL.md ")?;
    let mut skills = Vec::new();
    let mut remaining = rest;
    while let Some(start) = remaining.find('(') {
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find(')') else {
            break;
        };
        let skill = after_start[..end].trim();
        if is_skill_name(skill) {
            skills.push(skill.to_owned());
        }
        remaining = &after_start[end + 1..];
    }
    if skills.is_empty() {
        return None;
    }
    Some(CodexSkillAnnouncement {
        skill: skills.join(", "),
        message: "Read SKILL.md".to_owned(),
    })
}

fn parse_codex_file_reference(line: &str) -> Option<CodexFileReference> {
    for marker in ["file:", "path:", "output path:"] {
        if let Some(index) = line.find(marker) {
            let raw = line[index + marker.len()..].trim_start();
            return parse_path_reference(raw);
        }
    }
    None
}

fn parse_codex_file_change(line: &str) -> Option<CodexFileChange> {
    let trimmed = normalize_codex_bullet_line(line);
    let (action, rest) = parse_file_change_action(trimmed)?;
    let path = rest
        .split_whitespace()
        .next()?
        .trim_matches(|ch: char| matches!(ch, '`' | '"' | '\'' | ',' | ';'));
    if !is_probable_changed_file_path(path) {
        return None;
    }
    Some(CodexFileChange {
        action,
        path: path.to_owned(),
        additions: parse_file_change_count(trimmed, '+'),
        deletions: parse_file_change_count(trimmed, '-'),
        lines: Vec::new(),
    })
}

fn normalize_codex_bullet_line(line: &str) -> &str {
    line.trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| line.trim())
}

fn parse_codex_file_change_line(line: &str) -> Option<CodexFileChangeLine> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed == "⋮" {
        return None;
    }
    let (line_number, rest) = parse_leading_line_number(trimmed)?;
    let rest = rest.strip_prefix(' ')?;
    if let Some(content) = rest.strip_prefix('+') {
        return Some(CodexFileChangeLine {
            kind: CodexFileChangeLineKind::Added,
            old_line: None,
            new_line: Some(line_number),
            content: content.to_owned(),
        });
    }
    if let Some(content) = rest.strip_prefix('-') {
        return Some(CodexFileChangeLine {
            kind: CodexFileChangeLineKind::Deleted,
            old_line: Some(line_number),
            new_line: None,
            content: content.to_owned(),
        });
    }
    Some(CodexFileChangeLine {
        kind: CodexFileChangeLineKind::Context,
        old_line: Some(line_number),
        new_line: Some(line_number),
        content: rest.trim_start().to_owned(),
    })
}

fn parse_leading_line_number(value: &str) -> Option<(u32, &str)> {
    let first_non_digit = value
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_ascii_digit()).then_some(index))?;
    if first_non_digit == 0 {
        return None;
    }
    let line_number = value[..first_non_digit].parse::<u32>().ok()?;
    Some((line_number, &value[first_non_digit..]))
}

fn parse_file_change_action(line: &str) -> Option<(CodexFileChangeAction, &str)> {
    if let Some(rest) = line.strip_prefix("Added ") {
        return Some((CodexFileChangeAction::Added, rest));
    }
    if let Some(rest) = line.strip_prefix("Edited ") {
        return Some((CodexFileChangeAction::Edited, rest));
    }
    if let Some(rest) = line.strip_prefix("Deleted ") {
        return Some((CodexFileChangeAction::Deleted, rest));
    }
    None
}

fn parse_file_change_count(line: &str, sign: char) -> Option<u32> {
    line.split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | ',' | ';'))
        .find_map(|token| {
            let digits = token.strip_prefix(sign)?;
            (!digits.is_empty())
                .then(|| digits.parse::<u32>().ok())
                .flatten()
        })
}

fn is_probable_changed_file_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('-')
        && (path.contains('/') || path.contains('\\') || path.contains('.'))
}

fn parse_path_reference(raw: &str) -> Option<CodexFileReference> {
    let candidate = raw.split_whitespace().next()?.trim_matches(|ch| {
        matches!(
            ch,
            '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | ',' | ';'
        )
    });
    let candidate = candidate.trim_end_matches('.');
    if !is_probable_output_path(candidate) {
        return None;
    }

    let (path, line, column) = split_path_line_column(candidate);
    Some(CodexFileReference { path, line, column })
}

fn split_path_line_column(candidate: &str) -> (String, Option<u32>, Option<u32>) {
    let Some((before_last, last)) = candidate.rsplit_once(':') else {
        return (candidate.to_owned(), None, None);
    };
    let Ok(last_number) = last.parse::<u32>() else {
        return (candidate.to_owned(), None, None);
    };
    if let Some((path, line_part)) = before_last.rsplit_once(':') {
        if let Ok(line) = line_part.parse::<u32>() {
            return (path.to_owned(), Some(line), Some(last_number));
        }
    }
    (before_last.to_owned(), Some(last_number), None)
}

fn is_probable_output_path(candidate: &str) -> bool {
    !candidate.is_empty()
        && (candidate.starts_with('/')
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || candidate.starts_with("~")
            || candidate.starts_with("r")
                && candidate
                    .chars()
                    .nth(1)
                    .is_some_and(|ch| ch.is_ascii_digit()))
        && candidate.contains('/')
}

fn parse_context_window_percent(line: &str) -> Option<CodexContextUsage> {
    let rest = line.trim().strip_prefix("Context window:")?.trim();
    let percent = rest.strip_suffix('%')?.trim().parse::<u8>().ok()?;
    if percent > 100 {
        return None;
    }
    Some(CodexContextUsage {
        percent: Some(percent),
        used_tokens: None,
        total_tokens: None,
    })
}

fn parse_context_token_fraction(line: &str) -> Option<CodexContextUsage> {
    let trimmed = line.trim();
    let token_prefix = trimmed.strip_suffix("tokens")?.trim();
    let (used, total) = token_prefix.split_once('/')?;
    Some(CodexContextUsage {
        percent: None,
        used_tokens: Some(parse_token_count(used.trim())?),
        total_tokens: Some(parse_token_count(total.trim())?),
    })
}

fn parse_token_count(value: &str) -> Option<u64> {
    let value = value.replace(',', "");
    if let Some(number) = value.strip_suffix(['k', 'K']) {
        return number
            .parse::<u64>()
            .ok()
            .map(|tokens| tokens.saturating_mul(1_000));
    }
    value.parse::<u64>().ok()
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn normalize_tool_token(token: &str) -> Option<&str> {
    let token = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | '`' | ',' | ';' | ':'
        )
    });
    (!token.is_empty()).then_some(token)
}

fn parse_mcp_tool_marker(token: &str) -> Option<CodexToolCall> {
    let rest = token.strip_prefix("mcp__")?;
    let (server, name) = rest.split_once("__")?;
    if !is_tool_identifier_segment(server) || !is_tool_identifier_segment(name) {
        return None;
    }
    Some(CodexToolCall {
        namespace: format!("mcp__{server}"),
        name: name.to_owned(),
        marker: format!("mcp__{server}__{name}"),
    })
}

fn parse_dotted_tool_marker(
    token: &str,
    known_namespaces: &[&str],
    line: &str,
) -> Option<CodexToolCall> {
    let (namespace, name) = token.split_once('.')?;
    if name.contains('.')
        || !is_tool_identifier_segment(namespace)
        || !is_tool_identifier_segment(name)
    {
        return None;
    }
    let known_namespace = known_namespaces.contains(&namespace);
    if !known_namespace
        && !is_explicit_tool_call_line(line)
        && !namespace.contains('_')
        && !name.contains('_')
    {
        return None;
    }
    Some(CodexToolCall {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        marker: token.to_owned(),
    })
}

fn is_tool_identifier_segment(value: &str) -> bool {
    !value.is_empty() && value.chars().all(is_identifier_char)
}

fn is_explicit_tool_call_line(line: &str) -> bool {
    let lower = line.trim_start().to_ascii_lowercase();
    lower.starts_with("calling ")
        || lower.starts_with("called ")
        || lower.starts_with("queued ")
        || lower.starts_with("running ")
        || lower.starts_with("ran ")
        || lower.contains(" tool ")
}

fn is_skill_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '-' | '_' | '.'))
}

pub fn merge_screen_messages(existing: &mut Vec<ScreenMessage>, incoming: &[ScreenMessage]) {
    if incoming.is_empty() {
        return;
    }

    if let Some(last) = existing.last_mut() {
        let mut index = 0usize;
        while index < incoming.len() && incoming[index].role == last.role {
            if let Some(merged) = merge_message_content(&last.content, &incoming[index].content) {
                last.content = merged;
                index += 1;
                continue;
            }
            break;
        }
        if index > 0 {
            append_non_overlapping(existing, &incoming[index..]);
            dedupe_adjacent(existing);
            return;
        }
    }

    let overlap = find_overlap(existing, incoming);
    if overlap > 0 {
        if let (Some(last_existing), Some(last_incoming)) =
            (existing.last_mut(), incoming.get(overlap - 1))
        {
            if last_incoming.role == last_existing.role {
                if let Some(merged) =
                    merge_message_content(&last_existing.content, &last_incoming.content)
                {
                    last_existing.content = merged;
                }
            }
        }
        existing.extend_from_slice(&incoming[overlap..]);
        dedupe_adjacent(existing);
        return;
    }

    append_non_overlapping(existing, incoming);
    dedupe_adjacent(existing);
}

fn append_non_overlapping(existing: &mut Vec<ScreenMessage>, incoming: &[ScreenMessage]) {
    let overlap = longest_overlap(existing, incoming);
    existing.extend_from_slice(&incoming[overlap..]);
}

fn longest_overlap(existing: &[ScreenMessage], incoming: &[ScreenMessage]) -> usize {
    let max_overlap = existing.len().min(incoming.len());
    for overlap in (1..=max_overlap).rev() {
        if existing[existing.len() - overlap..] == incoming[..overlap] {
            return overlap;
        }
    }
    0
}

fn find_overlap(existing: &[ScreenMessage], incoming: &[ScreenMessage]) -> usize {
    let max_overlap = existing.len().min(incoming.len());
    for overlap in (1..=max_overlap).rev() {
        let existing_slice = &existing[existing.len() - overlap..];
        let incoming_slice = &incoming[..overlap];
        if slices_overlap(existing_slice, incoming_slice) {
            return overlap;
        }
    }
    0
}

fn slices_overlap(existing: &[ScreenMessage], incoming: &[ScreenMessage]) -> bool {
    for index in 0..existing.len() {
        if existing[index].role != incoming[index].role {
            return false;
        }
        if index + 1 == existing.len() {
            if merge_message_content(&existing[index].content, &incoming[index].content).is_some() {
                continue;
            }
            return false;
        }
        if existing[index].content != incoming[index].content {
            return false;
        }
    }
    true
}

fn dedupe_adjacent(messages: &mut Vec<ScreenMessage>) {
    messages.dedup_by(|right, left| left == right);
}

fn merge_message_content(existing: &str, incoming: &str) -> Option<String> {
    if incoming == existing {
        return Some(existing.to_owned());
    }
    if incoming.starts_with(existing) {
        return Some(incoming.to_owned());
    }
    if existing.starts_with(incoming) {
        return Some(existing.to_owned());
    }
    merge_message_content_by_line_overlap(existing, incoming)
}

fn merge_message_content_by_line_overlap(existing: &str, incoming: &str) -> Option<String> {
    let existing_lines = existing.lines().collect::<Vec<_>>();
    let incoming_lines = incoming.lines().collect::<Vec<_>>();
    let max_overlap = existing_lines.len().min(incoming_lines.len());
    for overlap in (1..=max_overlap).rev() {
        if existing_lines[existing_lines.len() - overlap..] == incoming_lines[..overlap] {
            let mut merged = existing_lines
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>();
            merged.extend(
                incoming_lines[overlap..]
                    .iter()
                    .map(|line| (*line).to_owned()),
            );
            return Some(merged.join("\n"));
        }
    }
    None
}

fn parse_box_role(line: &str) -> Option<ScreenMessageRole> {
    if !is_box_header_line(line) {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    if lower.contains("you") || lower.contains("user") {
        return Some(ScreenMessageRole::User);
    }
    if lower.contains("codex") || lower.contains("assistant") || lower.contains("agent") {
        return Some(ScreenMessageRole::Agent);
    }
    None
}

fn is_box_header_line(line: &str) -> bool {
    line.trim_start().starts_with('╭')
}

fn is_box_bottom(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('╰') || trimmed.starts_with('└')
}

fn parse_box_content(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let border = trimmed.chars().next()?;
    if border != '│' && border != '┃' {
        return None;
    }
    let content = trimmed[border.len_utf8()..].trim_start();
    let content = content.trim_end();
    let content = content
        .strip_suffix('│')
        .or_else(|| content.strip_suffix('┃'))
        .unwrap_or(content)
        .trim_end();
    Some(content.to_owned())
}

fn is_live_user_prompt_line(line: &str) -> bool {
    line.trim_start().starts_with('›')
}

fn is_live_agent_prompt_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn is_live_bullet_user_prompt(line: &str, next_line: Option<&str>) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('•') {
        return false;
    }
    next_line
        .map(|line| line.trim_start().starts_with('>'))
        .unwrap_or(false)
}

fn parse_live_prompt_content(line: &str) -> String {
    let trimmed = line.trim_start();
    for marker in ['›', '•', '>'] {
        if let Some(content) = trimmed.strip_prefix(marker) {
            return content.trim_start().to_owned();
        }
    }
    String::new()
}

fn push_live_prompt_message(
    messages: &mut Vec<ScreenMessage>,
    role: ScreenMessageRole,
    line: &str,
) {
    let content = parse_live_prompt_content(line);
    if !content.is_empty() {
        messages.push(ScreenMessage { role, content });
    }
}

fn parse_live_agent_prompt(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let content = trimmed.strip_prefix('>')?.trim_start();
    Some(content.to_owned())
}

fn parse_live_agent_bullet(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let bullet = trimmed.strip_prefix('•')?.trim_start();
    if is_transient_status_bullet(bullet) {
        return None;
    }
    Some(bullet.to_owned())
}

fn parse_live_continuation(line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let trimmed_end = line.trim_end();
    if !(trimmed_end.starts_with(' ') || trimmed_end.starts_with('\t')) {
        return None;
    }
    let trimmed = trimmed_end.trim_start();
    if let Some(bullet) = trimmed.strip_prefix('•') {
        let bullet = bullet.trim_start();
        if is_transient_status_bullet(bullet) {
            return None;
        }
        return Some(bullet.to_owned());
    }
    Some(trimmed.to_owned())
}

fn is_transient_bullet_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix('•')
        .map(|content| is_transient_status_bullet(content.trim_start()))
        .unwrap_or(false)
}

fn is_transient_status_bullet(content: &str) -> bool {
    content.starts_with("Starting MCP servers")
        || content.starts_with("Working (")
        || content.starts_with("Thinking (")
}

fn relevant_codex_screen_lines(screen: &str) -> Vec<String> {
    let lines = screen.lines().collect::<Vec<_>>();
    let start = transcript_start_index(&lines);
    let end = live_footer_start_index(&lines).unwrap_or(lines.len());
    let mut kept = Vec::new();
    let mut started = false;
    let mut index = start;
    while index < end {
        let line = lines[index];
        if !started && line.trim().is_empty() {
            index += 1;
            continue;
        }
        if !started && is_ignorable_transcript_line(line) {
            index += 1;
            while index < end && is_ignorable_transcript_continuation(lines[index]) {
                index += 1;
            }
            continue;
        }
        started = true;
        kept.push(line.to_owned());
        index += 1;
    }
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    kept
}

fn transcript_start_index(lines: &[&str]) -> usize {
    let Some(first_bottom) = lines.iter().position(|line| is_box_bottom(line)) else {
        return 0;
    };

    let leading_block = &lines[..=first_bottom];
    if leading_block
        .iter()
        .any(|line| parse_box_role(line).is_some())
    {
        return 0;
    }

    first_bottom + 1
}

fn live_footer_start_index(lines: &[&str]) -> Option<usize> {
    let model_index = lines.iter().rposition(|line| {
        let trimmed = line.trim();
        trimmed.contains(" · ") && trimmed.contains("gpt-")
    })?;
    let prompt_index = (0..=model_index)
        .rev()
        .find(|index| is_live_user_prompt_line(lines[*index]))?;
    let transcript_start = transcript_start_index(lines);
    let mut has_transcript_before_prompt = false;
    let mut index = transcript_start;
    while index < prompt_index {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if is_ignorable_transcript_line(line) {
            index += 1;
            while index < prompt_index && is_ignorable_transcript_continuation(lines[index]) {
                index += 1;
            }
            continue;
        }
        has_transcript_before_prompt = true;
        break;
    }
    has_transcript_before_prompt.then_some(prompt_index)
}

fn is_ignorable_transcript_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty()
        || trimmed.starts_with("Tip:")
        || trimmed == "immediately (except !)."
        || trimmed.starts_with("status:")
        || trimmed.starts_with("• You have ")
        || trimmed.starts_with("• Booting MCP server")
        || trimmed.starts_with("• Starting MCP servers")
        || trimmed.starts_with("• Working (")
        || trimmed.starts_with("• Thinking (")
        || trimmed.starts_with("─ Worked for ")
}

fn is_ignorable_transcript_continuation(line: &str) -> bool {
    let trimmed = line.trim_end();
    !trimmed.is_empty() && (trimmed.starts_with(' ') || trimmed.starts_with('\t'))
}

fn push_message(messages: &mut Vec<ScreenMessage>, role: ScreenMessageRole, body: Vec<String>) {
    let content = trim_blank_edges(&body.join("\n"));
    if content.is_empty() {
        return;
    }
    messages.push(ScreenMessage { role, content });
}

fn trim_blank_edges(content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(start);
    lines[start..end].join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        detect_directory_trust_prompt, encode_send_line, is_trust_prompt_visible,
        merge_screen_messages, parse_codex_context_usage, parse_codex_file_change_block,
        parse_codex_inline_event, parse_codex_screen_messages, parse_codex_structured_lines,
        CodexContextUsage, CodexFileChange, CodexFileChangeAction, CodexFileChangeLine,
        CodexFileChangeLineKind, CodexFileReference, CodexInlineEvent, CodexParsedLine,
        CodexSkillAnnouncement, CodexToolCall, ScreenMessage, ScreenMessageRole,
    };

    #[test]
    fn encode_send_line_returns_line_bytes_plus_carriage_return() {
        assert_eq!(encode_send_line("status"), b"status\r");
    }

    #[test]
    fn parses_known_tool_markers_as_inline_events() {
        assert_eq!(
            parse_codex_inline_event("Calling functions.exec_command with cargo test"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "functions".to_owned(),
                name: "exec_command".to_owned(),
                marker: "functions.exec_command".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("queued tool_search.tool_search_tool for discovery"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "tool_search".to_owned(),
                name: "tool_search_tool".to_owned(),
                marker: "tool_search.tool_search_tool".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Calling web.run to verify current docs"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "web".to_owned(),
                name: "run".to_owned(),
                marker: "web.run".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Calling multi_agent_v1.spawn_agent for review"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "multi_agent_v1".to_owned(),
                name: "spawn_agent".to_owned(),
                marker: "multi_agent_v1.spawn_agent".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Calling image_gen.imagegen for bitmap asset"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "image_gen".to_owned(),
                name: "imagegen".to_owned(),
                marker: "image_gen.imagegen".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Calling multi_tool_use.parallel for independent reads"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "multi_tool_use".to_owned(),
                name: "parallel".to_owned(),
                marker: "multi_tool_use.parallel".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Calling node_repl.js for browser automation"),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "node_repl".to_owned(),
                name: "js".to_owned(),
                marker: "node_repl.js".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event(
                "Calling mcp__xcodebuildmcp__session_show_defaults before build"
            ),
            Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "mcp__xcodebuildmcp".to_owned(),
                name: "session_show_defaults".to_owned(),
                marker: "mcp__xcodebuildmcp__session_show_defaults".to_owned(),
            }))
        );
        assert_eq!(parse_codex_inline_event("version 1.2.3 is available"), None);
        assert_eq!(
            parse_codex_inline_event("this prose mentions config.toml casually"),
            None
        );
    }

    #[test]
    fn parses_skill_announcements_as_inline_events() {
        assert_eq!(
            parse_codex_inline_event("Using superpowers:brainstorming to shape the implementation"),
            Some(CodexInlineEvent::Skill(CodexSkillAnnouncement {
                skill: "superpowers:brainstorming".to_owned(),
                message: "shape the implementation".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Using graphify to map the repository"),
            Some(CodexInlineEvent::Skill(CodexSkillAnnouncement {
                skill: "graphify".to_owned(),
                message: "map the repository".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Using skill-creator to add a local workflow"),
            Some(CodexInlineEvent::Skill(CodexSkillAnnouncement {
                skill: "skill-creator".to_owned(),
                message: "add a local workflow".to_owned(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("using lowercase is plain prose"),
            None
        );
    }

    #[test]
    fn parses_skill_read_lines_as_inline_events() {
        assert_eq!(
            parse_codex_inline_event("Read SKILL.md (graphify), SKILL.md (skill-creator)"),
            Some(CodexInlineEvent::Skill(CodexSkillAnnouncement {
                skill: "graphify, skill-creator".to_owned(),
                message: "Read SKILL.md".to_owned(),
            }))
        );
    }

    #[test]
    fn parses_file_references_from_tool_and_skill_output_paths() {
        assert_eq!(
            parse_codex_inline_event("skill body lives at (file: r3/brainstorming/SKILL.md:12)"),
            Some(CodexInlineEvent::FileReference(CodexFileReference {
                path: "r3/brainstorming/SKILL.md".to_owned(),
                line: Some(12),
                column: None,
            }))
        );
        assert_eq!(
            parse_codex_inline_event("wrote output path: /tmp/codex-results/report.json"),
            Some(CodexInlineEvent::FileReference(CodexFileReference {
                path: "/tmp/codex-results/report.json".to_owned(),
                line: None,
                column: None,
            }))
        );
        assert_eq!(
            parse_codex_inline_event("this line mentions src/lib.rs casually"),
            None
        );
    }

    #[test]
    fn parses_file_change_summaries_as_inline_events() {
        assert_eq!(
            parse_codex_inline_event("Edited crates/core/src/codex_tui.rs (+12 -3)"),
            Some(CodexInlineEvent::FileChange(CodexFileChange {
                action: CodexFileChangeAction::Edited,
                path: "crates/core/src/codex_tui.rs".to_owned(),
                additions: Some(12),
                deletions: Some(3),
                lines: Vec::new(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Added docs/superpowers/plans/manual.md"),
            Some(CodexInlineEvent::FileChange(CodexFileChange {
                action: CodexFileChangeAction::Added,
                path: "docs/superpowers/plans/manual.md".to_owned(),
                additions: None,
                deletions: None,
                lines: Vec::new(),
            }))
        );
        assert_eq!(
            parse_codex_inline_event("Deleted crates/old.rs (-22)"),
            Some(CodexInlineEvent::FileChange(CodexFileChange {
                action: CodexFileChangeAction::Deleted,
                path: "crates/old.rs".to_owned(),
                additions: None,
                deletions: Some(22),
                lines: Vec::new(),
            }))
        );
    }

    #[test]
    fn parses_codex_numbered_file_change_blocks() {
        assert_eq!(
            parse_codex_file_change_block("• Edited docs/superpowers/plans/manual.md (+17 -2)\n    378  assert_eq!(parse_codex_inline_event(\"this prose mentions config.toml casually\"), None);\n    379 +assert!(matches!(\n    381 -- [ ] **Step 2: Verify**\n    389 +- [ ] **Step 2: Add transcript grouping tests**\n    420 -"),
            Some(CodexFileChange {
                action: CodexFileChangeAction::Edited,
                path: "docs/superpowers/plans/manual.md".to_owned(),
                additions: Some(17),
                deletions: Some(2),
                lines: vec![
                    CodexFileChangeLine {
                        kind: CodexFileChangeLineKind::Context,
                        old_line: Some(378),
                        new_line: Some(378),
                        content:
                            "assert_eq!(parse_codex_inline_event(\"this prose mentions config.toml casually\"), None);"
                                .to_owned(),
                    },
                    CodexFileChangeLine {
                        kind: CodexFileChangeLineKind::Added,
                        old_line: None,
                        new_line: Some(379),
                        content: "assert!(matches!(".to_owned(),
                    },
                    CodexFileChangeLine {
                        kind: CodexFileChangeLineKind::Deleted,
                        old_line: Some(381),
                        new_line: None,
                        content: "- [ ] **Step 2: Verify**".to_owned(),
                    },
                    CodexFileChangeLine {
                        kind: CodexFileChangeLineKind::Added,
                        old_line: None,
                        new_line: Some(389),
                        content: "- [ ] **Step 2: Add transcript grouping tests**".to_owned(),
                    },
                    CodexFileChangeLine {
                        kind: CodexFileChangeLineKind::Deleted,
                        old_line: Some(420),
                        new_line: None,
                        content: String::new(),
                    },
                ],
            })
        );
    }

    #[test]
    fn parses_context_usage_percent_and_token_fraction() {
        assert_eq!(
            parse_codex_context_usage("Context window: 42%"),
            Some(CodexContextUsage {
                percent: Some(42),
                used_tokens: None,
                total_tokens: None,
            })
        );
        assert_eq!(
            parse_codex_context_usage("128k / 200k tokens"),
            Some(CodexContextUsage {
                percent: None,
                used_tokens: Some(128_000),
                total_tokens: Some(200_000),
            })
        );
        assert_eq!(parse_codex_context_usage("42% of tests are skipped"), None);
    }

    #[test]
    fn trust_prompt_detection_requires_both_strings_and_can_be_gated_externally() {
        let full_prompt = "\
Do you trust the contents of this directory?
1. Yes, continue";

        assert!(detect_directory_trust_prompt(full_prompt));
        assert!(is_trust_prompt_visible(full_prompt, true));
        assert!(!is_trust_prompt_visible(
            "Do you trust the contents of this directory?",
            true
        ));
        assert!(!is_trust_prompt_visible("1. Yes, continue", true));
        assert!(!is_trust_prompt_visible(full_prompt, false));
    }

    #[test]
    fn parses_boxed_you_and_codex_messages() {
        let screen = "\
╭─ You ─────────────────╮
│ Summarize the test.   │
╰───────────────────────╯
╭─ Codex ───────────────╮
│ Ready.                │
┃ Running checks now.   │
└───────────────────────╯";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "Summarize the test.".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "Ready.\nRunning checks now.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn parses_structured_lines_with_events_usage_and_messages() {
        assert_eq!(
            parse_codex_structured_lines(
                "Context window: 42%\nUsing superpowers:brainstorming to shape work\nPlain reply"
            ),
            vec![
                CodexParsedLine::ContextUsage {
                    usage: CodexContextUsage {
                        percent: Some(42),
                        used_tokens: None,
                        total_tokens: None,
                    },
                },
                CodexParsedLine::InlineEvent {
                    event: CodexInlineEvent::Skill(CodexSkillAnnouncement {
                        skill: "superpowers:brainstorming".to_owned(),
                        message: "shape work".to_owned(),
                    }),
                },
                CodexParsedLine::Message {
                    content: "Plain reply".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn parses_boxed_codex_bullet_content() {
        let screen = "\
╭─ Assistant ───────────╮
│ • Inspect the repo    │
│ • Run the tests       │
╰───────────────────────╯";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![ScreenMessage {
                role: ScreenMessageRole::Agent,
                content: "• Inspect the repo\n• Run the tests".to_owned(),
            }]
        );
    }

    #[test]
    fn skips_leading_chrome_box_but_keeps_following_boxed_transcript() {
        let screen = "\
╭────────────────────────────────────────────────────────╮
│ model:       gpt-5.4 medium                            │
│ directory:   ~/archductor/workspaces/chandelier/hoi-an │
│ permissions: YOLO mode                                 │
╰────────────────────────────────────────────────────────╯

╭─ You ─────────────────╮
│ Summarize the test.   │
╰───────────────────────╯
╭─ Codex ───────────────╮
│ Ready.                │
╰───────────────────────╯";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "Summarize the test.".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "Ready.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn parses_headerless_live_tui_bullet_responses_after_prompt() {
        let screen = "\
› User prompt
• Fix auth callback
  continuation line";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "User prompt".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "Fix auth callback\ncontinuation line".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn ignores_transient_status_bullets() {
        let screen = "\
› User prompt
• Starting MCP servers
• Working (4s)
• Search complete";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "User prompt".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "Search complete".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn parses_live_tui_when_user_is_bullet_and_agent_is_gt_marker() {
        let screen = "\
• user prompt
> first agent line
  continuation line";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "user prompt".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "first agent line\ncontinuation line".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn ignores_live_footer_and_parses_scrolled_agent_tail() {
        let screen = "\
  5. Medium: production builds intentionally ignore TypeScript errors.
     next.config.ts:3 sets typescript.ignoreBuildErrors = true. Impact: type
     regressions can ship to production instead of blocking CI/build.

  6. Low: the repo has test files but no runnable test script. package.json:5
     defines no test command, and npm test fails. Impact: there is no standard
     verification path for the existing tests, which makes regressions easier to
     miss.

  Verification

  npm test fails because there is no test script. npm run typecheck, npm run
  lint, and npm run build also could not run here because dependencies are not
  installed in this checkout.

  If you want, I can fix the auth holes and the webhook idempotency issue first.

─ Worked for 2m 24s ────────────────────────────────────────────────────────────


› Improve documentation in @filename

  gpt-5.4 medium · ~/archductor/workspaces/chandelier/islamabad";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![ScreenMessage {
                role: ScreenMessageRole::Agent,
                content: "5. Medium: production builds intentionally ignore TypeScript errors.\nnext.config.ts:3 sets typescript.ignoreBuildErrors = true. Impact: type\nregressions can ship to production instead of blocking CI/build.\n\n6. Low: the repo has test files but no runnable test script. package.json:5\ndefines no test command, and npm test fails. Impact: there is no standard\nverification path for the existing tests, which makes regressions easier to\nmiss.\n\nVerification\n\nnpm test fails because there is no test script. npm run typecheck, npm run\nlint, and npm run build also could not run here because dependencies are not\ninstalled in this checkout.\n\nIf you want, I can fix the auth holes and the webhook idempotency issue first.".to_owned(),
            }]
        );
    }

    #[test]
    fn parses_pty_screen_log_startup_prompt_when_model_is_loading() {
        let screen = "\
╭──────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.142.3)                       │
│                                                  │
│ model:       loading   /model to change          │
│ directory:   ~/archductor/…/chandelier/islamabad │
│ permissions: YOLO mode                           │
╰──────────────────────────────────────────────────╯


› Improve documentation in @filename

  gpt-5.4 default · ~/archductor/workspaces/chandelier/islamabad";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![ScreenMessage {
                role: ScreenMessageRole::User,
                content: "Improve documentation in @filename".to_owned(),
            }]
        );
    }

    #[test]
    fn parses_pty_screen_log_prompt_while_ignoring_boot_noise() {
        let screen = "\
╭──────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.142.3)                       │
│                                                  │
│ model:       gpt-5.4 medium   /model to change   │
│ directory:   ~/archductor/…/chandelier/islamabad │
│ permissions: YOLO mode                           │
╰──────────────────────────────────────────────────╯

  Tip: NEW: Network proxy can now be enabled from /experimental. Restart Codex
  after enabling it.

• You have 2 usage limit resets available. Run /usage to use one.

• Booting MCP server: codex_apps (0s • esc to interrupt)


› Improve documentation in @filename

  gpt-5.4 medium · ~/archductor/workspaces/chandelier/islamabad";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![ScreenMessage {
                role: ScreenMessageRole::User,
                content: "Improve documentation in @filename".to_owned(),
            }]
        );
    }

    #[test]
    fn parses_pty_screen_log_scrolled_agent_tail_above_footer_prompt() {
        let screen = "\
• Repo state is quiet.

  You’re in /home/kitts/archductor/workspaces/chandelier/hoi-an on branch lc/
  hoi-an, and HEAD matches origin/main at commit 7f7ab37 (Add custom payment
  split (#14)). There are no tracked file changes. The only uncommitted thing is
  an untracked .context/ folder with placeholder files:

  - .context/brief.md
  - .context/todos.md
  - .context/agent-notes.md

  Project-wise, this is a Next.js 16.2.9 / React 19 app for Chandelier
  Consulting with public marketing pages, admin routes, Supabase, and Stripe
  APIs. The last few merged changes were:

  - Add custom payment split
  - simplify client agreement flow
  - Stack projects page layout


› Use /skills to list available skills

  gpt-5.4 medium · ~/archductor/workspaces/chandelier/hoi-an";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![ScreenMessage {
                role: ScreenMessageRole::Agent,
                content: "Repo state is quiet.\n\nYou’re in /home/kitts/archductor/workspaces/chandelier/hoi-an on branch lc/\nhoi-an, and HEAD matches origin/main at commit 7f7ab37 (Add custom payment\nsplit (#14)). There are no tracked file changes. The only uncommitted thing is\nan untracked .context/ folder with placeholder files:\n\n- .context/brief.md\n- .context/todos.md\n- .context/agent-notes.md\n\nProject-wise, this is a Next.js 16.2.9 / React 19 app for Chandelier\nConsulting with public marketing pages, admin routes, Supabase, and Stripe\nAPIs. The last few merged changes were:\n\n- Add custom payment split\n- simplify client agreement flow\n- Stack projects page layout".to_owned(),
            }]
        );
    }

    #[test]
    fn preserves_wrapped_live_agent_reply_before_footer() {
        let screen = "\
│                                                        │
│ model:       gpt-5.4 medium   /model to change         │
│ directory:   ~/archductor/workspaces/chandelier/hoi-an │
│ permissions: YOLO mode                                 │
╰────────────────────────────────────────────────────────╯

  Tip: Press Tab to queue a message when a task is running; otherwise it sends
  immediately (except !).

• You have 2 usage limit resets available. Run /usage to use one.


› What's my name?


• I don’t know your name from the context here. If you want, tell me and I’ll
  use it.


› Implement {feature}

  gpt-5.4 medium · ~/archductor/workspaces/chandelier/hoi-an";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "What's my name?".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content:
                        "I don’t know your name from the context here. If you want, tell me and I’ll\nuse it."
                            .to_owned(),
                },
            ]
        );
    }

    #[test]
    fn ignores_plain_status_lines_after_boxed_transcript() {
        let screen = "\
╭─ You\n│ run tests\n╰─\n╭─ Codex\n│ Running now.\n╰─\nstatus: spinner";

        assert_eq!(
            parse_codex_screen_messages(screen),
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "run tests".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "Running now.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn merges_same_agent_message_when_visible_window_scrolls() {
        let mut existing = vec![ScreenMessage {
            role: ScreenMessageRole::Agent,
            content: "2. High: several admin pages never check auth even when auth is configured.\nThe generic admin section route loads privileged data.\n3. High: the Stripe webhook can lose events permanently after a partial failure.".to_owned(),
        }];
        let incoming = vec![ScreenMessage {
            role: ScreenMessageRole::Agent,
            content: "3. High: the Stripe webhook can lose events permanently after a partial failure.\n4. Medium: AUTO_ADVANCE_PHASE_ON_SIGN is dead config.".to_owned(),
        }];

        merge_screen_messages(&mut existing, &incoming);

        assert_eq!(
            existing,
            vec![ScreenMessage {
                role: ScreenMessageRole::Agent,
                content: "2. High: several admin pages never check auth even when auth is configured.\nThe generic admin section route loads privileged data.\n3. High: the Stripe webhook can lose events permanently after a partial failure.\n4. Medium: AUTO_ADVANCE_PHASE_ON_SIGN is dead config.".to_owned(),
            }]
        );
    }

    #[test]
    fn dedupes_and_merges_repainted_messages_when_same_role_prefix_is_extended() {
        let mut existing = vec![ScreenMessage {
            role: ScreenMessageRole::Agent,
            content: "Inspect".to_owned(),
        }];
        let incoming = vec![
            ScreenMessage {
                role: ScreenMessageRole::Agent,
                content: "Inspect".to_owned(),
            },
            ScreenMessage {
                role: ScreenMessageRole::Agent,
                content: "Inspect the repo".to_owned(),
            },
            ScreenMessage {
                role: ScreenMessageRole::User,
                content: "continue".to_owned(),
            },
        ];

        merge_screen_messages(&mut existing, &incoming);

        assert_eq!(
            existing,
            vec![
                ScreenMessage {
                    role: ScreenMessageRole::Agent,
                    content: "Inspect the repo".to_owned(),
                },
                ScreenMessage {
                    role: ScreenMessageRole::User,
                    content: "continue".to_owned(),
                },
            ]
        );
    }
}
