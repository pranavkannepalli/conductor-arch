use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct LinearIssue {
    pub identifier: String,
    pub title: String,
    pub branch_name: Option<String>,
    pub url: Option<String>,
}

struct LinearCurlRequest {
    args: Vec<String>,
    stdin: String,
}

pub fn fetch_linear_issue(issue_id: &str) -> Result<LinearIssue> {
    let api_key = std::env::var("LINEAR_API_KEY")
        .context("LINEAR_API_KEY is required to create a workspace from a Linear issue")?;
    fetch_linear_issue_with_api_key(issue_id, &api_key)
}

fn fetch_linear_issue_with_api_key(issue_id: &str, api_key: &str) -> Result<LinearIssue> {
    let request = build_linear_curl_request(api_key, &linear_issue_query_payload(issue_id));
    let mut child = Command::new("curl")
        .args(&request.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run curl for Linear API")?;
    child
        .stdin
        .as_mut()
        .context("open curl config stdin")?
        .write_all(request.stdin.as_bytes())
        .context("write curl config")?;
    let output = child
        .wait_with_output()
        .context("wait for Linear API curl")?;
    anyhow::ensure!(
        output.status.success(),
        "Linear API request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = String::from_utf8_lossy(&output.stdout);
    parse_linear_issue_response(issue_id, &body)
}

fn linear_issue_query_payload(issue_id: &str) -> String {
    json!({
        "query": "query Issue($id: String!) { issue(id: $id) { identifier title branchName url } }",
        "variables": { "id": issue_id },
    })
    .to_string()
}

fn build_linear_curl_request(api_key: &str, payload: &str) -> LinearCurlRequest {
    LinearCurlRequest {
        args: vec!["-fsS".to_owned(), "--config".to_owned(), "-".to_owned()],
        stdin: format!(
            "url = \"https://api.linear.app/graphql\"\nheader = \"Content-Type: application/json\"\nheader = \"Authorization: {}\"\ndata = \"{}\"\n",
            curl_config_escape(api_key),
            curl_config_escape(payload)
        ),
    }
}

fn parse_linear_issue_response(issue_id: &str, body: &str) -> Result<LinearIssue> {
    let value: Value = serde_json::from_str(body).context("parse Linear API response")?;
    if let Some(errors) = value.get("errors") {
        anyhow::bail!("Linear API returned errors: {errors}");
    }
    let issue = value
        .get("data")
        .and_then(|data| data.get("issue"))
        .context("Linear API response did not include issue data")?;
    let identifier = issue
        .get("identifier")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| issue_id.to_ascii_uppercase());
    let title = issue
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("Linear issue {issue_id} did not include a title"))?;
    let branch_name = issue
        .get("branchName")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let url = issue.get("url").and_then(Value::as_str).map(str::to_owned);
    Ok(LinearIssue {
        identifier,
        title,
        branch_name,
        url,
    })
}

fn curl_config_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            _ => vec![ch],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_curl_request_keeps_api_key_out_of_argv() {
        let request = build_linear_curl_request("lin_api_secret", "{}");

        assert!(!request
            .args
            .iter()
            .any(|arg| arg.contains("lin_api_secret")));
        assert!(request.stdin.contains("lin_api_secret"));
    }

    #[test]
    fn linear_curl_request_keeps_payload_out_of_argv() {
        let payload = linear_issue_query_payload("ENG-123");
        let request = build_linear_curl_request("lin_api_secret", &payload);

        assert!(!request.args.iter().any(|arg| arg.contains("ENG-123")));
        assert!(request.stdin.contains("ENG-123"));
    }

    #[test]
    fn parses_linear_issue_response_from_graphql_data() {
        let response = r#"{"data":{"issue":{"identifier":"ENG-123","title":"Fix launch","branchName":"eng-123-fix-launch","url":"https://linear.app/acme/issue/ENG-123"}}}"#;

        let issue = parse_linear_issue_response("ENG-123", response).unwrap();

        assert_eq!(issue.identifier, "ENG-123");
        assert_eq!(issue.title, "Fix launch");
        assert_eq!(issue.branch_name.as_deref(), Some("eng-123-fix-launch"));
        assert_eq!(
            issue.url.as_deref(),
            Some("https://linear.app/acme/issue/ENG-123")
        );
    }

    #[test]
    fn linear_issue_query_payload_escapes_issue_id_as_json() {
        let payload = linear_issue_query_payload("ENG-\"123");

        assert!(payload.contains(r#"ENG-\"123"#));
    }
}
