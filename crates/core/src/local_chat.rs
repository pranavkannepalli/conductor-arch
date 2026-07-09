use std::path::PathBuf;

use crate::codex_tui::{merge_screen_messages, parse_codex_screen_messages, ScreenMessage};
use crate::session_event::{SessionEvent, SessionEventPayload};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChatHistorySummary {
    pub process_id: i64,
    pub chat_thread_id: Option<i64>,
    pub repository_name: String,
    pub workspace_name: String,
    pub workspace_path: PathBuf,
    pub agent_type: String,
    pub status: String,
    pub started_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub preview: String,
    pub harness: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChatThreadSummary {
    pub thread_id: i64,
    pub repository_name: String,
    pub workspace_name: String,
    pub workspace_path: PathBuf,
    pub provider: String,
    pub title: String,
    pub status: String,
    pub updated_at: String,
    pub message_count: usize,
    pub preview: String,
    pub native_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChatHistoryMessage {
    pub role: String,
    pub content: String,
}

pub(crate) fn parse_local_chat_transcript(transcript: &str) -> Vec<LocalChatHistoryMessage> {
    let lines = transcript.lines().collect::<Vec<_>>();
    let mut messages = Vec::new();
    let mut codex_messages = Vec::<ScreenMessage>::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index].trim_end();
        if line.trim().is_empty() {
            index += 1;
            continue;
        }

        if line == "[codex screen]" {
            let (screen, next) = collect_codex_screen_block(&lines, index + 1);
            let parsed = parse_codex_screen_messages(&screen);
            merge_screen_messages(&mut codex_messages, &parsed);
            index = next;
            continue;
        }

        if line == "[codex raw]" {
            let (_, next) = collect_codex_raw_block(&lines, index + 1);
            index = next;
            continue;
        }

        if is_local_user_marker(line) {
            let (content, next) = collect_local_user_input(&lines, index + 1);
            push_local_chat_message(&mut messages, "user", content);
            index = next;
            continue;
        }

        if line == "[staged review prompt]" {
            let (content, next) = collect_staged_review_prompt(&lines, index + 1);
            push_local_chat_message(&mut messages, "review", content);
            index = next;
            continue;
        }

        if is_local_system_marker(line) {
            push_local_chat_message(&mut messages, "system", line.to_owned());
            index += 1;
            continue;
        }

        let (content, next) = collect_until_local_marker(&lines, index);
        push_local_chat_message(&mut messages, "agent", content);
        index = next;
    }

    for message in codex_messages {
        push_local_chat_message(&mut messages, message.role.as_str(), message.content);
    }
    messages
}

pub(crate) fn session_events_to_local_chat_messages(
    events: &[SessionEvent],
) -> Vec<LocalChatHistoryMessage> {
    let mut messages = Vec::new();
    for event in events {
        match &event.payload {
            SessionEventPayload::UserInput { text, kind } => {
                let role = match kind {
                    crate::session_event::SessionInputKind::ReviewPrompt => "review",
                    crate::session_event::SessionInputKind::ControlCommand => "system",
                    crate::session_event::SessionInputKind::User => "user",
                };
                push_local_chat_message(&mut messages, role, text.clone());
            }
            SessionEventPayload::AssistantText { text } => {
                push_local_chat_message(&mut messages, "agent", text.clone());
            }
            SessionEventPayload::CommandOutput { title, output, .. } => {
                let content = if output.is_empty() {
                    title.clone()
                } else {
                    format!("{title}\n{output}")
                };
                push_local_chat_message(&mut messages, "system", content);
            }
            SessionEventPayload::StatusChange { message, .. } => {
                if let Some(message) = message {
                    push_local_chat_message(&mut messages, "system", message.clone());
                }
            }
            SessionEventPayload::Error { message, .. } => {
                push_local_chat_message(&mut messages, "system", message.clone());
            }
            SessionEventPayload::Prompt { text, .. } => {
                push_local_chat_message(&mut messages, "system", text.clone());
            }
            SessionEventPayload::Metadata { .. } => {}
        }
    }
    messages
}

pub(crate) fn local_chat_agent_type(command: &str) -> String {
    let lower = command.to_ascii_lowercase();
    if lower.contains("codex") {
        "Codex".to_owned()
    } else if lower.contains("claude") {
        "Claude".to_owned()
    } else if lower.contains("cursor") {
        "Cursor".to_owned()
    } else {
        "Shell".to_owned()
    }
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn push_local_chat_message(
    messages: &mut Vec<LocalChatHistoryMessage>,
    role: &str,
    content: String,
) {
    let content = content.trim().to_owned();
    if !content.is_empty() {
        messages.push(LocalChatHistoryMessage {
            role: role.to_owned(),
            content,
        });
    }
}

fn collect_until_local_marker(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut content = Vec::new();
    while index < lines.len() {
        let line = lines[index].trim_end();
        if !content.is_empty() && is_local_chat_marker(line) {
            break;
        }
        content.push(lines[index]);
        index += 1;
    }
    (content.join("\n"), index)
}

fn collect_local_user_input(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut content = Vec::new();
    while index < lines.len() {
        let line = lines[index].trim_end();
        if line == "[/user input]" {
            return (content.join("\n"), index + 1);
        }
        if content.is_empty() && is_local_chat_marker(line) {
            return (String::new(), index);
        }
        content.push(lines[index]);
        index += 1;
    }
    (content.join("\n"), index)
}

fn collect_staged_review_prompt(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut content = Vec::new();
    while index < lines.len() {
        let line = lines[index].trim_end();
        if line == "[/staged review prompt]" {
            return (content.join("\n"), index + 1);
        }
        content.push(lines[index]);
        index += 1;
    }
    (content.join("\n"), index)
}

fn collect_codex_screen_block(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut content = Vec::new();
    while index < lines.len() {
        let line = lines[index].trim_end();
        if line == "[/codex screen]" {
            return (content.join("\n"), index + 1);
        }
        content.push(lines[index]);
        index += 1;
    }
    (content.join("\n"), index)
}

fn collect_codex_raw_block(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut content = Vec::new();
    while index < lines.len() {
        let line = lines[index].trim_end();
        if line == "[/codex raw]" {
            return (content.join("\n"), index + 1);
        }
        content.push(lines[index]);
        index += 1;
    }
    (content.join("\n"), index)
}

fn is_local_chat_marker(line: &str) -> bool {
    is_local_user_marker(line)
        || line == "[/user input]"
        || line == "[staged review prompt]"
        || line == "[codex raw]"
        || line == "[/codex raw]"
        || line == "[codex screen]"
        || line == "[/codex screen]"
        || is_local_system_marker(line)
}

fn is_local_user_marker(line: &str) -> bool {
    line.starts_with("[user input ") && line.ends_with(']')
}

fn is_local_system_marker(line: &str) -> bool {
    line.starts_with("[session ")
        || line.starts_with("[provider ")
        || line.starts_with("[mcp ")
        || line.starts_with("[harness ")
        || line.starts_with("[tool ")
        || line.starts_with("[skill ")
        || line.starts_with("[archductor bootstrap")
}
