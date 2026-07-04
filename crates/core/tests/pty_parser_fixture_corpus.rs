use std::fs;
use std::path::Path;

use linux_archductor_core::codex_tui::{
    parse_codex_screen_delta, CodexFileChangeAction, CodexParsedItem, CodexTranscriptEvent,
    ScreenMessageRole,
};
use serde::Deserialize;
use vt100::Parser;

const CORPUS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pty_corpus");

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    fixtures: Vec<CorpusFixture>,
}

#[derive(Debug, Deserialize)]
struct CorpusFixture {
    id: String,
    outcome: FixtureOutcome,
    features: Vec<FixtureFeature>,
    raw_log: String,
    expected_events: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureOutcome {
    Success,
    Failure,
    Interruption,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureFeature {
    Ansi,
    PartialLines,
    DelayedChunks,
    DuplicatedChunks,
    ToolOutput,
    Prompt,
    Error,
}

#[derive(Debug, Deserialize)]
struct RawPtyLog {
    rows: u16,
    cols: u16,
    chunks: Vec<RawPtyChunk>,
}

#[derive(Debug, Deserialize)]
struct RawPtyChunk {
    data: String,
    #[serde(default)]
    delay_ms: Option<u64>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExpectedParsedEvent {
    Message { role: String, content: String },
    Tool { title: String, body: String },
    Skill { title: String, body: String },
    FileChange { action: String, path: String },
}

#[test]
fn pty_parser_fixture_corpus_has_required_session_coverage() {
    let manifest = read_manifest();

    assert!(
        manifest
            .fixtures
            .iter()
            .filter(|fixture| fixture.outcome == FixtureOutcome::Success)
            .count()
            >= 5
    );
    assert!(
        manifest
            .fixtures
            .iter()
            .filter(|fixture| matches!(
                fixture.outcome,
                FixtureOutcome::Failure | FixtureOutcome::Interruption
            ))
            .count()
            >= 3
    );

    for feature in [
        FixtureFeature::Ansi,
        FixtureFeature::PartialLines,
        FixtureFeature::DelayedChunks,
        FixtureFeature::DuplicatedChunks,
        FixtureFeature::ToolOutput,
        FixtureFeature::Prompt,
        FixtureFeature::Error,
    ] {
        assert!(
            manifest
                .fixtures
                .iter()
                .any(|fixture| fixture.features.contains(&feature)),
            "corpus is missing required feature {feature:?}"
        );
    }

    for fixture in manifest.fixtures {
        let raw = read_raw_log(&fixture.raw_log);
        let expected = read_expected_events(&fixture.expected_events);
        for feature in &fixture.features {
            assert_fixture_feature(&fixture.id, &raw, &expected, *feature);
        }
    }
}

#[test]
fn pty_parser_fixture_corpus_replays_to_expected_events_without_codex() {
    let manifest = read_manifest();

    for fixture in manifest.fixtures {
        let raw = read_raw_log(&fixture.raw_log);
        assert!(
            !raw.chunks.is_empty(),
            "fixture {} has no raw PTY chunks",
            fixture.id
        );
        if fixture.features.contains(&FixtureFeature::DelayedChunks) {
            assert!(
                raw.chunks
                    .iter()
                    .any(|chunk| chunk.delay_ms.unwrap_or(0) > 0),
                "fixture {} is marked delayed but has no delayed chunk",
                fixture.id
            );
        }

        let screen = replay_raw_log(&raw);
        let parsed = parse_codex_screen_delta(&screen, &Default::default(), None)
            .items
            .into_iter()
            .map(expected_event_from_parsed_item)
            .collect::<Vec<_>>();
        let expected = read_expected_events(&fixture.expected_events);

        assert_eq!(parsed, expected, "fixture {} parsed mismatch", fixture.id);
    }
}

fn read_manifest() -> CorpusManifest {
    let path = Path::new(CORPUS_DIR).join("manifest.json");
    let json = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"));
    serde_json::from_str(&json).unwrap_or_else(|err| panic!("parse {path:?}: {err}"))
}

fn read_raw_log(path: &str) -> RawPtyLog {
    let path = Path::new(CORPUS_DIR).join(path);
    let json = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"));
    serde_json::from_str(&json).unwrap_or_else(|err| panic!("parse {path:?}: {err}"))
}

fn read_expected_events(path: &str) -> Vec<ExpectedParsedEvent> {
    let path = Path::new(CORPUS_DIR).join(path);
    let json = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"));
    serde_json::from_str(&json).unwrap_or_else(|err| panic!("parse {path:?}: {err}"))
}

fn replay_raw_log(raw: &RawPtyLog) -> String {
    let mut parser = Parser::new(raw.rows, raw.cols, 0);
    for chunk in &raw.chunks {
        parser.process(chunk.data.as_bytes());
    }
    parser.screen().contents()
}

fn assert_fixture_feature(
    fixture_id: &str,
    raw: &RawPtyLog,
    expected: &[ExpectedParsedEvent],
    feature: FixtureFeature,
) {
    let raw_text = raw
        .chunks
        .iter()
        .map(|chunk| chunk.data.as_str())
        .collect::<String>();
    match feature {
        FixtureFeature::Ansi => assert!(
            raw_text.contains('\u{1b}'),
            "fixture {fixture_id} is marked ansi but contains no escape bytes"
        ),
        FixtureFeature::PartialLines => assert!(
            raw.chunks
                .iter()
                .any(|chunk| !chunk.data.ends_with('\n') && !chunk.data.ends_with("\r\n")),
            "fixture {fixture_id} is marked partial_lines but every chunk ends on a line boundary"
        ),
        FixtureFeature::DelayedChunks => assert!(
            raw.chunks
                .iter()
                .any(|chunk| chunk.delay_ms.unwrap_or(0) > 0),
            "fixture {fixture_id} is marked delayed_chunks but has no delayed chunk"
        ),
        FixtureFeature::DuplicatedChunks => assert!(
            raw.chunks
                .windows(2)
                .any(|window| window[0].data == window[1].data),
            "fixture {fixture_id} is marked duplicated_chunks but has no repeated adjacent chunk"
        ),
        FixtureFeature::ToolOutput => assert!(
            expected
                .iter()
                .any(|event| matches!(event, ExpectedParsedEvent::Tool { .. })),
            "fixture {fixture_id} is marked tool_output but expects no tool event"
        ),
        FixtureFeature::Prompt => assert!(
            raw_text.contains("Do you trust the contents of this directory?"),
            "fixture {fixture_id} is marked prompt but has no trust prompt text"
        ),
        FixtureFeature::Error => {
            let lower = raw_text.to_lowercase();
            assert!(
                lower.contains("error") || lower.contains("^c") || lower.contains("stopped"),
                "fixture {fixture_id} is marked error but has no error/interruption marker"
            );
        }
    }
}

fn expected_event_from_parsed_item(item: CodexParsedItem) -> ExpectedParsedEvent {
    match item {
        CodexParsedItem::Message(message) => ExpectedParsedEvent::Message {
            role: match message.role {
                ScreenMessageRole::User => "user",
                ScreenMessageRole::Agent => "agent",
            }
            .to_owned(),
            content: message.content,
        },
        CodexParsedItem::Event(CodexTranscriptEvent::Tool { title, body }) => {
            ExpectedParsedEvent::Tool { title, body }
        }
        CodexParsedItem::Event(CodexTranscriptEvent::Skill { title, body }) => {
            ExpectedParsedEvent::Skill { title, body }
        }
        CodexParsedItem::Event(CodexTranscriptEvent::FileChange(change)) => {
            ExpectedParsedEvent::FileChange {
                action: match change.action {
                    CodexFileChangeAction::Added => "added",
                    CodexFileChangeAction::Edited => "edited",
                    CodexFileChangeAction::Deleted => "deleted",
                }
                .to_owned(),
                path: change.path,
            }
        }
    }
}
