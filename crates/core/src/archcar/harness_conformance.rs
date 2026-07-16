use super::harness::{managed_harness_for_kind, validate_managed_harness};
use super::harness_contract::{
    DesiredHarnessControls, HarnessAdapterContext, HarnessCapability, HarnessEffect, HarnessInput,
    NativeRecord, SupportMode, REQUIRED_HARNESS_FEATURES,
};
use super::protocol::{ArchcarInputDelivery, ArchcarInputKind};
use crate::workspace::SessionKind;
use serde_json::Value;

#[test]
fn codex_and_claude_implement_contract_v1() {
    for kind in [SessionKind::Codex, SessionKind::Claude] {
        let harness = managed_harness_for_kind(kind).expect("managed harness");
        assert_eq!(harness.descriptor().contract_version, 1);
        assert_eq!(
            harness.descriptor().required_features,
            REQUIRED_HARNESS_FEATURES,
        );
        validate_managed_harness(harness.as_ref()).expect("valid descriptor");
    }
}

#[test]
fn optional_goal_support_is_explicit() {
    let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
    assert_eq!(
        codex.descriptor().optional(HarnessCapability::Goals),
        SupportMode::Native
    );
    assert!(matches!(
        claude.descriptor().optional(HarnessCapability::Goals),
        SupportMode::Unsupported { .. }
    ));
}

#[test]
fn shell_stays_outside_the_managed_chat_contract() {
    assert!(managed_harness_for_kind(SessionKind::Shell).is_none());
}

#[test]
fn managed_adapters_wrap_existing_native_input_formats() {
    let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
    let mut codex_adapter = codex
        .create_adapter(adapter_context(Some("codex-thread-1")))
        .unwrap();
    let codex_write = codex_adapter
        .encode_input(input("codex-input", "run tests"))
        .unwrap();
    let codex_payload: Value = serde_json::from_slice(&codex_write.payload).unwrap();
    assert_eq!(codex_write.provider_key, "codex");
    assert_eq!(codex_write.local_input_id.as_deref(), Some("codex-input"));
    assert_eq!(codex_payload["method"], "turn/start");
    assert_eq!(codex_payload["params"]["threadId"], "codex-thread-1");
    assert_eq!(codex_payload["params"]["input"][0]["text"], "run tests");

    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
    let mut claude_adapter = claude.create_adapter(adapter_context(None)).unwrap();
    let claude_write = claude_adapter
        .encode_input(input("claude-input", "review changes"))
        .unwrap();
    let claude_payload: Value = serde_json::from_slice(&claude_write.payload).unwrap();
    assert_eq!(claude_write.provider_key, "claude");
    assert_eq!(claude_write.local_input_id.as_deref(), Some("claude-input"));
    assert_eq!(claude_payload["type"], "user");
    assert_eq!(claude_payload["message"]["role"], "user");
    assert_eq!(
        claude_payload["message"]["content"][0]["text"],
        "review changes"
    );
}

#[test]
fn claude_does_not_fake_native_input_acknowledgement() {
    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
    let mut adapter = claude.create_adapter(adapter_context(None)).unwrap();
    adapter
        .encode_input(input("claude-input", "review changes"))
        .unwrap();

    let effects = adapter
        .observe_native(NativeRecord {
            provider_key: "claude",
            payload: br#"{"type":"stream_event","session_id":"claude-session-1","event":{"type":"message_start","message":{"id":"message-1"}}}
"#
            .to_vec(),
        })
        .unwrap();

    assert!(effects.iter().any(|effect| matches!(
        effect,
        HarnessEffect::TurnStarted { local_input_id } if local_input_id == "claude-input"
    )));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, HarnessEffect::InputAcknowledged { .. })));
}

#[test]
fn codex_error_response_does_not_acknowledge_or_drop_steer_input() {
    let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
    let mut adapter = codex
        .create_adapter(adapter_context(Some("codex-thread-1")))
        .unwrap();
    adapter
        .encode_input(input("turn-input", "run tests"))
        .unwrap();
    adapter
        .observe_native(codex_record(
            r#"{"method":"turn/started","params":{"threadId":"codex-thread-1","turn":{"id":"turn-1"}}}"#,
        ))
        .unwrap();
    adapter
        .encode_input(immediate_input("steer-input", "also run clippy"))
        .unwrap();

    let error_effects = adapter
        .observe_native(codex_record(
            r#"{"id":2,"error":{"code":-32000,"message":"turn already completed"}}"#,
        ))
        .unwrap();
    assert!(!error_effects.iter().any(|effect| matches!(
        effect,
        HarnessEffect::InputAcknowledged { local_input_id } if local_input_id == "steer-input"
    )));

    let retry_effects = adapter
        .observe_native(codex_record(r#"{"id":2,"result":{}}"#))
        .unwrap();
    assert!(retry_effects.iter().any(|effect| matches!(
        effect,
        HarnessEffect::InputAcknowledged { local_input_id } if local_input_id == "steer-input"
    )));
}

#[test]
fn codex_steer_preserves_turn_start_input_for_exactly_once_completion() {
    let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
    let mut adapter = codex
        .create_adapter(adapter_context(Some("codex-thread-1")))
        .unwrap();
    adapter
        .encode_input(input("turn-input", "run tests"))
        .unwrap();
    adapter
        .observe_native(codex_record(
            r#"{"method":"turn/started","params":{"threadId":"codex-thread-1","turn":{"id":"turn-1"}}}"#,
        ))
        .unwrap();
    adapter
        .encode_input(immediate_input("steer-input", "also run clippy"))
        .unwrap();

    let completion = codex_record(
        r#"{"method":"turn/completed","params":{"threadId":"codex-thread-1","turn":{"id":"turn-1","status":"completed"}}}"#,
    );
    let effects = adapter.observe_native(completion.clone()).unwrap();
    assert!(effects.iter().any(|effect| matches!(
        effect,
        HarnessEffect::TurnCompleted { local_input_id, .. } if local_input_id == "turn-input"
    )));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        HarnessEffect::TurnCompleted { local_input_id, .. } if local_input_id == "steer-input"
    )));

    let duplicate_effects = adapter.observe_native(completion).unwrap();
    assert!(!duplicate_effects
        .iter()
        .any(|effect| matches!(effect, HarnessEffect::TurnCompleted { .. })));
}

fn adapter_context(native_session_id: Option<&str>) -> HarnessAdapterContext {
    HarnessAdapterContext {
        session_id: 7,
        thread_id: 11,
        workspace: "berlin".to_owned(),
        native_session_id: native_session_id.map(ToOwned::to_owned),
        controls: DesiredHarnessControls::default(),
    }
}

fn input(local_input_id: &str, content: &str) -> HarnessInput {
    HarnessInput {
        local_input_id: local_input_id.to_owned(),
        content: content.to_owned(),
        visible_content: None,
        kind: ArchcarInputKind::User,
        delivery: ArchcarInputDelivery::Auto,
    }
}

fn immediate_input(local_input_id: &str, content: &str) -> HarnessInput {
    HarnessInput {
        delivery: ArchcarInputDelivery::Immediate,
        ..input(local_input_id, content)
    }
}

fn codex_record(payload: &str) -> NativeRecord {
    NativeRecord {
        provider_key: "codex",
        payload: format!("{payload}\n").into_bytes(),
    }
}
