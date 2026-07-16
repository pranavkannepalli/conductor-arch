use super::harness::{managed_harness_for_kind, validate_managed_harness};
use super::harness_contract::{
    DesiredHarnessControls, HarnessAdapterContext, HarnessCapability, HarnessControl,
    HarnessControlPlan, HarnessEffect, HarnessInput, HarnessRecoveryCause, HarnessRecoveryPlan,
    HarnessSignal, NativeRecord, ProviderInteractionResolution, SupportMode,
    REQUIRED_HARNESS_FEATURES,
};
use super::protocol::{
    session_harness_capabilities_for_descriptor, ArchcarInputDelivery, ArchcarInputKind,
};
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
fn capability_snapshots_include_required_baseline_for_managed_providers() {
    for kind in [SessionKind::Codex, SessionKind::Claude] {
        let harness = managed_harness_for_kind(kind).expect("managed harness");
        let capabilities = session_harness_capabilities_for_descriptor(
            harness.descriptor(),
            vec!["native-extra".to_owned()],
        );
        let required = REQUIRED_HARNESS_FEATURES
            .iter()
            .map(|feature| feature.as_str().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(capabilities.contract_version, 1);
        assert_eq!(capabilities.required, required);
        assert_eq!(capabilities.observed_native, vec!["native-extra"]);
        assert_eq!(
            capabilities.optional.len(),
            harness.descriptor().optional_capabilities.len()
        );
    }
}

#[test]
fn claude_reconfigure_controls_require_resume_with_desired_controls() {
    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
    let mut adapter = claude
        .create_adapter(adapter_context(Some("claude-session-1")))
        .unwrap();

    assert!(matches!(
        adapter.plan_control(HarnessControl::SetEffort(Some("high".to_owned()))),
        HarnessControlPlan::RestartRequired(DesiredHarnessControls {
            effort: Some(ref effort),
            ..
        }) if effort == "high"
    ));
}

#[test]
fn claude_interaction_resolution_requires_restart_with_desired_controls() {
    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
    let mut adapter = claude
        .create_adapter(adapter_context(Some("claude-session-1")))
        .unwrap();
    adapter.plan_control(HarnessControl::SetModel(Some("claude-sonnet-5".to_owned())));

    assert!(matches!(
        adapter.plan_control(HarnessControl::ResolveInteraction(
            ProviderInteractionResolution::Approve
        )),
        HarnessControlPlan::RestartRequired(DesiredHarnessControls {
            model: Some(ref model),
            ..
        }) if model == "claude-sonnet-5"
    ));
}

#[test]
fn claude_interrupt_uses_process_group_and_resume_recovery() {
    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
    let mut adapter = claude
        .create_adapter(adapter_context(Some("claude-session-1")))
        .unwrap();

    assert_eq!(
        adapter.plan_control(HarnessControl::Interrupt),
        HarnessControlPlan::Signal(HarnessSignal::InterruptProcessGroup)
    );
    assert!(matches!(
        adapter.recovery_plan(HarnessRecoveryCause::InterruptDeadline),
        HarnessRecoveryPlan::RestartAndResume {
            native_session_id,
            ..
        } if native_session_id == "claude-session-1"
    ));
}

#[test]
fn codex_interrupt_uses_native_turn_interrupt_when_active() {
    let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
    let mut adapter = codex
        .create_adapter(adapter_context(Some("codex-thread-1")))
        .unwrap();
    adapter
        .observe_native(NativeRecord {
            provider_key: "codex",
            payload: br#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#.to_vec(),
        })
        .unwrap();

    assert!(matches!(
        adapter.plan_control(HarnessControl::Interrupt),
        HarnessControlPlan::NativeWrite(_)
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
