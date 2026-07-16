use std::path::Path;

use serde_json::{json, Map, Value};

use crate::archcar::harness_contract::ProviderInteractionResolution;

const ARCHCAR_CLAUDE_HOOK_FLAG: &str = "--archcar-claude-hook";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeHookRequest {
    PreToolUse { event_name: String },
    PermissionRequest,
    AskUserQuestion,
    ExitPlanMode,
    Unknown { event_name: String },
}

impl ClaudeHookRequest {
    pub fn event_name(&self) -> &str {
        match self {
            Self::PreToolUse { event_name } => event_name,
            Self::PermissionRequest => "PermissionRequest",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::ExitPlanMode => "ExitPlanMode",
            Self::Unknown { event_name } => event_name,
        }
    }
}

pub fn build_claude_hook_settings(executable: &Path, thread_id: i64) -> Value {
    let hook = json!({
        "type": "command",
        "command": executable.to_string_lossy(),
        "args": [ARCHCAR_CLAUDE_HOOK_FLAG, thread_id.to_string()],
    });
    json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": ".*",
                "hooks": [hook.clone()]
            }],
            "PermissionRequest": [{
                "matcher": ".*",
                "hooks": [hook.clone()]
            }],
            "AskUserQuestion": [{
                "matcher": "AskUserQuestion|ExitPlanMode",
                "hooks": [hook.clone()]
            }],
            "ExitPlanMode": [{
                "matcher": "AskUserQuestion|ExitPlanMode",
                "hooks": [hook]
            }]
        }
    })
}

pub fn classify_claude_hook_request(input: &Value) -> ClaudeHookRequest {
    let event_name = input
        .get("hook_event_name")
        .or_else(|| input.get("hookEventName"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tool_name = input
        .get("tool_name")
        .or_else(|| input.get("toolName"))
        .and_then(Value::as_str);

    match event_name {
        "PermissionRequest" => ClaudeHookRequest::PermissionRequest,
        "AskUserQuestion" => ClaudeHookRequest::AskUserQuestion,
        "ExitPlanMode" => ClaudeHookRequest::ExitPlanMode,
        "PreToolUse" if tool_name == Some("AskUserQuestion") => ClaudeHookRequest::AskUserQuestion,
        "PreToolUse" if tool_name == Some("ExitPlanMode") => ClaudeHookRequest::ExitPlanMode,
        "PreToolUse" => ClaudeHookRequest::PreToolUse {
            event_name: "PreToolUse".to_owned(),
        },
        "" => ClaudeHookRequest::Unknown {
            event_name: "Unknown".to_owned(),
        },
        other => ClaudeHookRequest::Unknown {
            event_name: other.to_owned(),
        },
    }
}

pub fn encode_claude_hook_defer(request: &ClaudeHookRequest) -> Value {
    let hook_event_name = match request {
        ClaudeHookRequest::PermissionRequest => "PermissionRequest",
        ClaudeHookRequest::AskUserQuestion | ClaudeHookRequest::ExitPlanMode => "PreToolUse",
        other => other.event_name(),
    };
    json!({
        "hookSpecificOutput": {
            "hookEventName": hook_event_name,
            "permissionDecision": "defer"
        }
    })
}

pub fn encode_claude_hook_resolution(
    request: &Value,
    resolution: &ProviderInteractionResolution,
) -> Value {
    match (classify_claude_hook_request(request), resolution) {
        (ClaudeHookRequest::PermissionRequest, ProviderInteractionResolution::Approve) => {
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedPermissions": permission_suggestions(request)
                    }
                }
            })
        }
        (ClaudeHookRequest::PermissionRequest, ProviderInteractionResolution::Deny { reason }) => {
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "deny",
                        "message": reason.clone().unwrap_or_else(|| "Denied by Archductor.".to_owned())
                    }
                }
            })
        }
        (ClaudeHookRequest::AskUserQuestion, ProviderInteractionResolution::Answer { answers }) => {
            encode_updated_input("AskUserQuestion", request, answers_object(answers))
        }
        (ClaudeHookRequest::ExitPlanMode, ProviderInteractionResolution::Approve) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow"
            }
        }),
        (ClaudeHookRequest::ExitPlanMode, ProviderInteractionResolution::Deny { reason }) => {
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason.clone().unwrap_or_else(|| "Keep planning.".to_owned())
                }
            })
        }
        (_, ProviderInteractionResolution::Deny { reason }) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason.clone().unwrap_or_else(|| "Denied by Archductor.".to_owned())
            }
        }),
        _ => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow"
            }
        }),
    }
}

pub fn handle_claude_hook_json(_thread_id: i64, stdin: &str) -> Value {
    let request = serde_json::from_str::<Value>(stdin).unwrap_or(Value::Null);
    encode_claude_hook_defer(&classify_claude_hook_request(&request))
}

fn encode_updated_input(event_name: &str, request: &Value, answers: Value) -> Value {
    let mut updated_input = request
        .get("tool_input")
        .or_else(|| request.get("toolInput"))
        .cloned()
        .unwrap_or_else(|| request.clone());
    if let Value::Object(ref mut object) = updated_input {
        object.insert("answers".to_owned(), answers);
    }
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": updated_input,
            "toolName": event_name
        }
    })
}

fn permission_suggestions(request: &Value) -> Value {
    request
        .get("permission_suggestions")
        .or_else(|| request.get("permissionSuggestions"))
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn answers_object(answers: &[(String, String)]) -> Value {
    let mut object = Map::new();
    for (key, value) in answers {
        object.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn claude_hooks_permission_request_defers_pretooluse() {
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"}
        });

        assert_eq!(
            encode_claude_hook_defer(&classify_claude_hook_request(&input)),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "defer"
                }
            })
        );
    }

    #[test]
    fn claude_hooks_permission_resolution_allows_with_updated_permissions() {
        let suggestions = json!([{"tool": "Bash", "rule": "cargo test"}]);
        let input = json!({
            "hook_event_name": "PermissionRequest",
            "permission_suggestions": suggestions
        });

        assert_eq!(
            encode_claude_hook_resolution(&input, &ProviderInteractionResolution::Approve),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedPermissions": suggestions
                    }
                }
            })
        );
    }

    #[test]
    fn claude_hooks_question_resolution_echoes_questions_and_answers() {
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "AskUserQuestion",
            "tool_input": {
                "questions": [{"id": "scope", "question": "Ship it?"}]
            }
        });

        assert_eq!(
            encode_claude_hook_resolution(
                &input,
                &ProviderInteractionResolution::Answer {
                    answers: vec![("scope".to_owned(), "yes".to_owned())]
                }
            ),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": {
                        "questions": [{"id": "scope", "question": "Ship it?"}],
                        "answers": {"scope": "yes"}
                    },
                    "toolName": "AskUserQuestion"
                }
            })
        );
    }

    #[test]
    fn claude_hooks_plan_resolution_approves_and_denies() {
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "ExitPlanMode",
            "tool_input": {"plan": "Do work"}
        });

        assert_eq!(
            encode_claude_hook_resolution(&input, &ProviderInteractionResolution::Approve),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow"
                }
            })
        );
        assert_eq!(
            encode_claude_hook_resolution(
                &input,
                &ProviderInteractionResolution::Deny {
                    reason: Some("Keep planning".to_owned())
                }
            ),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": "Keep planning"
                }
            })
        );
    }

    #[test]
    fn claude_hook_settings_builds_exec_form_hooks() {
        let executable = PathBuf::from("/usr/local/bin/archductor");
        let thread_id = 42;
        let settings = build_claude_hook_settings(&executable, thread_id);
        let permission_hook = &settings["hooks"]["PermissionRequest"][0];
        let question_hook = &settings["hooks"]["AskUserQuestion"][0];
        let hook = &permission_hook["hooks"][0];

        assert_eq!(permission_hook["matcher"], ".*");
        assert_eq!(question_hook["matcher"], "AskUserQuestion|ExitPlanMode");
        assert_eq!(hook["command"], executable.to_string_lossy().as_ref());
        assert!(hook["args"]
            .as_array()
            .unwrap()
            .contains(&json!(thread_id.to_string())));
        assert!(settings.get("disableAllHooks").is_none());
    }
}
