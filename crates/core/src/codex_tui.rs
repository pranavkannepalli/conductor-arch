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
        merge_screen_messages, parse_codex_screen_messages, ScreenMessage, ScreenMessageRole,
    };

    #[test]
    fn encode_send_line_returns_line_bytes_plus_carriage_return() {
        assert_eq!(encode_send_line("status"), b"status\r");
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
