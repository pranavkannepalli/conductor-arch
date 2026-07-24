use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestCheckRun {
    pub name: String,
    pub status: String,
    pub detail: Option<String>,
}

impl PullRequestCheckRun {
    pub fn is_failure(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "fail" | "failed" | "failure" | "error" | "cancelled" | "timed_out"
        )
    }

    pub fn is_pending(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "pending" | "queued" | "requested" | "waiting" | "in_progress" | "in progress"
        )
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "pass" | "passed" | "success" | "successful" | "completed"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestReviewEntry {
    pub author: String,
    pub state: String,
    pub body: Option<String>,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestCommentEntry {
    pub author: String,
    pub body: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestDeployment {
    pub environment: String,
    pub status: String,
    pub url: Option<String>,
}

impl PullRequestDeployment {
    pub fn is_failure(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "fail" | "failed" | "failure" | "error" | "inactive" | "cancelled" | "timed_out"
        )
    }

    pub fn is_pending(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "pending" | "queued" | "requested" | "waiting" | "in_progress" | "in progress"
        )
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "pass" | "passed" | "success" | "successful" | "active"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestThreadComment {
    pub author: String,
    pub body: String,
    pub url: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestReviewThread {
    pub id: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub resolved: bool,
    pub comments: Vec<PullRequestThreadComment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestReadiness {
    pub state: Option<String>,
    pub merge_state_status: Option<String>,
    pub mergeable: Option<String>,
    pub review_decision: Option<String>,
    pub latest_reviews: Vec<PullRequestReviewEntry>,
    pub comments: Vec<PullRequestCommentEntry>,
    pub review_threads: Vec<PullRequestReviewThread>,
    pub checks: Vec<PullRequestCheckRun>,
    pub deployments: Vec<PullRequestDeployment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubNumberedChoice {
    pub number: u64,
    pub state: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitHubDeploymentEntry {
    pub(crate) id: i64,
    pub(crate) environment: String,
    pub(crate) status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitHubDeploymentStatus {
    pub(crate) state: String,
    pub(crate) url: Option<String>,
}

pub(crate) fn extract_pull_request_url(output: &str) -> Option<String> {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(str::to_owned)
}

pub(crate) fn parse_pull_request_number(url: &str) -> Option<i64> {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .and_then(|segment| segment.parse::<i64>().ok())
}

pub fn parse_github_numbered_stateful_choices(raw: &str) -> Vec<GitHubNumberedChoice> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let number = parts.next()?.trim().parse().ok()?;
            let state = parts.next()?.trim().to_owned();
            let title = parts.next()?.trim().to_owned();
            Some(GitHubNumberedChoice {
                number,
                state,
                title,
            })
        })
        .collect()
}

pub(crate) fn parse_pull_request_check_runs(output: &str) -> Vec<PullRequestCheckRun> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let parts = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if parts.len() >= 2 {
                return Some(PullRequestCheckRun {
                    name: parts[0].to_owned(),
                    status: parts[1].to_owned(),
                    detail: parts
                        .iter()
                        .skip(2)
                        .rev()
                        .find(|part| !part.is_empty())
                        .map(|part| (*part).to_owned()),
                });
            }
            let lower = line.to_ascii_lowercase();
            let status = [
                "fail",
                "failed",
                "failure",
                "error",
                "cancelled",
                "timed_out",
                "pass",
                "pending",
            ]
            .iter()
            .find(|status| lower.contains(**status))?;
            Some(PullRequestCheckRun {
                name: line.to_owned(),
                status: (*status).to_owned(),
                detail: None,
            })
        })
        .collect()
}

pub(crate) fn parse_pull_request_readiness(output: &str) -> Result<PullRequestReadiness> {
    let value: Value = serde_json::from_str(output).context("parse gh pull request JSON")?;
    let latest_reviews = value
        .get("latestReviews")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_pull_request_review_entry)
        .collect();
    let comments = value
        .get("comments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_pull_request_comment_entry)
        .collect();
    let mut checks = Vec::new();
    let mut deployments = Vec::new();
    for item in json_array_or_nodes(value.get("statusCheckRollup")) {
        if is_deployment_rollup_item(item) {
            if let Some(deployment) = parse_pull_request_deployment(item) {
                deployments.push(deployment);
            }
        } else if let Some(check) = parse_pull_request_rollup_check(item) {
            checks.push(check);
        }
    }
    Ok(PullRequestReadiness {
        state: json_string(&value, "state"),
        merge_state_status: json_string(&value, "mergeStateStatus"),
        mergeable: json_string(&value, "mergeable"),
        review_decision: json_string(&value, "reviewDecision"),
        latest_reviews,
        comments,
        review_threads: Vec::new(),
        checks,
        deployments,
    })
}

pub(crate) fn parse_pull_request_review_threads(
    output: &str,
) -> Result<Vec<PullRequestReviewThread>> {
    let value: Value = serde_json::from_str(output).context("parse GitHub review thread JSON")?;
    let threads = value
        .get("data")
        .and_then(|data| data.get("node"))
        .and_then(|node| node.get("reviewThreads"))
        .and_then(|review_threads| review_threads.get("nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_pull_request_review_thread)
        .collect();
    Ok(threads)
}

pub(crate) fn parse_pull_request_review_thread_mutation(
    output: &str,
    mutation_name: &str,
) -> Result<PullRequestReviewThread> {
    let value: Value = serde_json::from_str(output).context("parse GitHub review thread JSON")?;
    let thread = value
        .get("data")
        .and_then(|data| data.get(mutation_name))
        .and_then(|mutation| mutation.get("thread"))
        .and_then(parse_pull_request_review_thread)
        .with_context(|| format!("parse GitHub {mutation_name} response"))?;
    Ok(thread)
}

fn parse_pull_request_review_thread(value: &Value) -> Option<PullRequestReviewThread> {
    let comments = value
        .get("comments")
        .and_then(|comments| comments.get("nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_pull_request_thread_comment)
        .collect();
    Some(PullRequestReviewThread {
        id: json_string(value, "id"),
        path: json_string(value, "path"),
        line: json_i64(value, "line").or_else(|| json_i64(value, "startLine")),
        resolved: value
            .get("isResolved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        comments,
    })
}

fn parse_pull_request_thread_comment(value: &Value) -> Option<PullRequestThreadComment> {
    Some(PullRequestThreadComment {
        author: json_author_login(value).unwrap_or_else(|| "unknown".to_owned()),
        body: json_string(value, "body")?,
        url: json_string(value, "url"),
        created_at: json_string(value, "createdAt"),
    })
}

fn parse_pull_request_review_entry(value: &Value) -> Option<PullRequestReviewEntry> {
    Some(PullRequestReviewEntry {
        author: json_author_login(value).unwrap_or_else(|| "unknown".to_owned()),
        state: json_string(value, "state")?,
        body: json_string(value, "body"),
        submitted_at: json_string(value, "submittedAt"),
    })
}

fn parse_pull_request_comment_entry(value: &Value) -> Option<PullRequestCommentEntry> {
    Some(PullRequestCommentEntry {
        author: json_author_login(value).unwrap_or_else(|| "unknown".to_owned()),
        body: json_string(value, "body")?,
        created_at: json_string(value, "createdAt"),
    })
}

fn parse_pull_request_rollup_check(value: &Value) -> Option<PullRequestCheckRun> {
    let name = json_string(value, "name")
        .or_else(|| json_string(value, "context"))
        .or_else(|| json_string(value, "workflowName"))?;
    let status = json_string(value, "conclusion")
        .or_else(|| json_string(value, "state"))
        .or_else(|| json_string(value, "status"))
        .unwrap_or_else(|| "UNKNOWN".to_owned());
    Some(PullRequestCheckRun {
        name,
        status,
        detail: json_string(value, "detailsUrl")
            .or_else(|| json_string(value, "targetUrl"))
            .or_else(|| json_string(value, "url")),
    })
}

pub(crate) fn parse_github_commit_status_checks(output: &str) -> Result<Vec<PullRequestCheckRun>> {
    let value: Value = serde_json::from_str(output).context("parse GitHub commit status JSON")?;
    Ok(value
        .get("statuses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_github_commit_status_check)
        .collect())
}

fn parse_github_commit_status_check(value: &Value) -> Option<PullRequestCheckRun> {
    Some(PullRequestCheckRun {
        name: json_string(value, "context")
            .or_else(|| json_string(value, "name"))
            .unwrap_or_else(|| "status".to_owned()),
        status: json_string(value, "state")
            .or_else(|| json_string(value, "status"))
            .unwrap_or_else(|| "UNKNOWN".to_owned()),
        detail: json_string(value, "target_url").or_else(|| json_string(value, "url")),
    })
}

fn parse_pull_request_deployment(value: &Value) -> Option<PullRequestDeployment> {
    let status = value
        .get("latestStatus")
        .and_then(|latest| json_string(latest, "state"))
        .or_else(|| json_nested_string(value, "status", "state"))
        .or_else(|| json_string(value, "conclusion"))
        .or_else(|| json_string(value, "state"))
        .or_else(|| json_string(value, "status"))
        .unwrap_or_else(|| "UNKNOWN".to_owned());
    Some(PullRequestDeployment {
        environment: json_string(value, "environment")
            .or_else(|| json_nested_string(value, "environment", "name"))
            .or_else(|| json_string(value, "name"))
            .unwrap_or_else(|| "deployment".to_owned()),
        status,
        url: json_string(value, "url")
            .or_else(|| json_string(value, "latestEnvironmentUrl"))
            .or_else(|| json_string(value, "targetUrl"))
            .or_else(|| json_nested_string(value, "latestStatus", "environmentUrl"))
            .or_else(|| json_nested_string(value, "latestStatus", "logUrl"))
            .or_else(|| json_nested_string(value, "status", "targetUrl")),
    })
}

pub(crate) fn parse_github_deployment_entries(output: &str) -> Result<Vec<GitHubDeploymentEntry>> {
    let value: Value = serde_json::from_str(output).context("parse GitHub deployment JSON")?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_github_deployment_entry)
        .collect())
}

fn parse_github_deployment_entry(value: &Value) -> Option<GitHubDeploymentEntry> {
    Some(GitHubDeploymentEntry {
        id: json_i64(value, "id")?,
        environment: json_string(value, "environment")
            .or_else(|| json_nested_string(value, "environment", "name"))
            .unwrap_or_else(|| "deployment".to_owned()),
        status: json_string(value, "state").or_else(|| json_string(value, "status")),
    })
}

pub(crate) fn parse_github_deployment_latest_status(
    output: &str,
) -> Result<Option<GitHubDeploymentStatus>> {
    let value: Value =
        serde_json::from_str(output).context("parse GitHub deployment status JSON")?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .find_map(parse_github_deployment_status))
}

fn parse_github_deployment_status(value: &Value) -> Option<GitHubDeploymentStatus> {
    Some(GitHubDeploymentStatus {
        state: json_string(value, "state")?,
        url: json_string(value, "environment_url")
            .or_else(|| json_string(value, "log_url"))
            .or_else(|| json_string(value, "target_url")),
    })
}

fn is_deployment_rollup_item(value: &Value) -> bool {
    json_string(value, "__typename")
        .map(|name| name.eq_ignore_ascii_case("deployment"))
        .unwrap_or(false)
        || value.get("environment").is_some()
}

pub(crate) fn append_unique_checks(
    checks: &mut Vec<PullRequestCheckRun>,
    additional: Vec<PullRequestCheckRun>,
) {
    for check in additional {
        if !checks.iter().any(|existing| {
            existing.name.eq_ignore_ascii_case(&check.name)
                && existing.status.eq_ignore_ascii_case(&check.status)
                && normalized_optional_url(existing.detail.as_deref())
                    == normalized_optional_url(check.detail.as_deref())
        }) {
            checks.push(check);
        }
    }
}

pub(crate) fn append_unique_deployments(
    deployments: &mut Vec<PullRequestDeployment>,
    additional: Vec<PullRequestDeployment>,
) {
    for deployment in additional {
        if !deployments.iter().any(|existing| {
            existing.environment == deployment.environment
                && existing.status == deployment.status
                && existing.url == deployment.url
        }) {
            deployments.push(deployment);
        }
    }
}

fn json_author_login(value: &Value) -> Option<String> {
    value
        .get("author")
        .and_then(|author| json_string(author, "login"))
}

fn normalized_optional_url(value: Option<&str>) -> Option<String> {
    value.map(|url| url.trim_end_matches('/').to_owned())
}

fn json_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn json_nested_string(value: &Value, parent: &str, field: &str) -> Option<String> {
    value
        .get(parent)
        .and_then(|nested| json_string(nested, field))
}

fn json_array_or_nodes(value: Option<&Value>) -> Vec<&Value> {
    if let Some(items) = value.and_then(Value::as_array) {
        return items.iter().collect();
    }
    if let Some(items) = value
        .and_then(|item| item.get("nodes"))
        .and_then(Value::as_array)
    {
        return items.iter().collect();
    }
    if let Some(items) = value
        .and_then(|item| item.get("contexts"))
        .and_then(|contexts| contexts.get("nodes"))
        .and_then(Value::as_array)
    {
        return items.iter().collect();
    }
    Vec::new()
}

fn json_i64(value: &Value, field: &str) -> Option<i64> {
    value.get(field).and_then(Value::as_i64)
}

pub(crate) fn json_root_string(input: &str, field: &str) -> Result<Option<String>> {
    let value: Value = serde_json::from_str(input).context("parse JSON")?;
    Ok(json_string(&value, field))
}

pub(crate) fn format_pull_request_checks_agent_prompt(
    name: &str,
    checks: &[PullRequestCheckRun],
) -> String {
    let failures = checks
        .iter()
        .filter(|check| check.is_failure())
        .collect::<Vec<_>>();
    let mut prompt = format!("Fix these failing PR checks for workspace {name}.\n");
    if failures.is_empty() {
        prompt.push_str("No failing PR checks.\n");
        return prompt;
    }
    prompt.push_str("Make the smallest safe changes, then run relevant tests.\n\n");
    for check in failures {
        match check.detail.as_deref() {
            Some(detail) => prompt.push_str(&format!(
                "- {}: {} - {}\n",
                check.name, check.status, detail
            )),
            None => prompt.push_str(&format!("- {}: {}\n", check.name, check.status)),
        }
    }
    prompt
}

pub(crate) fn format_pull_request_review_agent_prompt(name: &str, review_state: &str) -> String {
    let mut prompt = format!("Address this GitHub PR review/comment state for workspace {name}.\n");
    let review_state = review_state.trim();
    if review_state.is_empty() {
        prompt.push_str("No GitHub PR review/comment output.\n");
        return prompt;
    }
    prompt.push_str("Make the smallest safe changes, then run relevant tests.\n\n");
    prompt.push_str(review_state);
    prompt.push('\n');
    prompt
}

pub(crate) fn format_pull_request_readiness(
    name: &str,
    readiness: &PullRequestReadiness,
) -> String {
    let mut out = format!("PR readiness for workspace {name}.\n");
    if let Some(state) = readiness.state.as_deref() {
        out.push_str(&format!("State: {state}\n"));
    }
    if let Some(merge_state) = readiness.merge_state_status.as_deref() {
        out.push_str(&format!("Merge state: {merge_state}\n"));
    }
    if let Some(mergeable) = readiness.mergeable.as_deref() {
        out.push_str(&format!("Mergeable: {mergeable}\n"));
    }
    out.push_str(&format!(
        "Review decision: {}\n",
        readiness.review_decision.as_deref().unwrap_or("UNKNOWN")
    ));
    append_rollup_entries(&mut out, readiness);
    append_attention_entries(&mut out, readiness);
    append_review_entries(&mut out, &readiness.latest_reviews);
    append_comment_entries(&mut out, &readiness.comments);
    append_review_thread_entries(&mut out, &readiness.review_threads);
    append_check_entries(&mut out, &readiness.checks);
    append_deployment_entries(&mut out, &readiness.deployments);
    out
}

pub(crate) fn format_pull_request_readiness_agent_prompt(
    name: &str,
    readiness: &PullRequestReadiness,
) -> String {
    let mut prompt = format!("Address this PR readiness state for workspace {name}.\n");
    prompt.push_str("Prioritize failing checks, failed deployments, and requested changes. Make the smallest safe changes, then run relevant tests.\n\n");
    prompt.push_str(&format_pull_request_readiness(name, readiness));
    prompt
}

fn append_rollup_entries(out: &mut String, readiness: &PullRequestReadiness) {
    let unresolved_threads = readiness
        .review_threads
        .iter()
        .filter(|thread| !thread.resolved)
        .count();
    let check_counts = gate_counts(
        readiness.checks.iter(),
        PullRequestCheckRun::is_success,
        PullRequestCheckRun::is_failure,
        PullRequestCheckRun::is_pending,
    );
    let deployment_counts = gate_counts(
        readiness.deployments.iter(),
        PullRequestDeployment::is_success,
        PullRequestDeployment::is_failure,
        PullRequestDeployment::is_pending,
    );

    out.push_str("\nRollup:\n");
    out.push_str(&format!(
        "- Reviews: {} latest, {} top-level {}\n",
        readiness.latest_reviews.len(),
        readiness.comments.len(),
        plural(readiness.comments.len(), "comment", "comments")
    ));
    out.push_str(&format!(
        "- Review threads: {} unresolved / {} total\n",
        unresolved_threads,
        readiness.review_threads.len()
    ));
    out.push_str(&format!(
        "- Checks: {} passing, {} failing, {} pending, {} other\n",
        check_counts.passing, check_counts.failing, check_counts.pending, check_counts.other
    ));
    out.push_str(&format!(
        "- Deployments: {} passing, {} failing, {} pending, {} other\n",
        deployment_counts.passing,
        deployment_counts.failing,
        deployment_counts.pending,
        deployment_counts.other
    ));
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GateCounts {
    passing: usize,
    failing: usize,
    pending: usize,
    other: usize,
}

fn gate_counts<'a, T: 'a>(
    items: impl Iterator<Item = &'a T>,
    is_success: impl Fn(&T) -> bool,
    is_failure: impl Fn(&T) -> bool,
    is_pending: impl Fn(&T) -> bool,
) -> GateCounts {
    let mut counts = GateCounts::default();
    for item in items {
        if is_failure(item) {
            counts.failing += 1;
        } else if is_pending(item) {
            counts.pending += 1;
        } else if is_success(item) {
            counts.passing += 1;
        } else {
            counts.other += 1;
        }
    }
    counts
}

fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        singular.to_owned()
    } else {
        plural.to_owned()
    }
}

fn append_attention_entries(out: &mut String, readiness: &PullRequestReadiness) {
    let mut lines = Vec::new();
    if matches!(
        readiness
            .review_decision
            .as_deref()
            .map(|decision| decision.to_ascii_uppercase())
            .as_deref(),
        Some("CHANGES_REQUESTED" | "REVIEW_REQUIRED")
    ) {
        lines.push(format!(
            "- Review decision: {}",
            readiness.review_decision.as_deref().unwrap_or("UNKNOWN")
        ));
    }
    for thread in readiness
        .review_threads
        .iter()
        .filter(|thread| !thread.resolved)
    {
        let id = thread.id.as_deref().unwrap_or("unknown thread");
        lines.push(format!(
            "- Unresolved review thread {id} at {}",
            review_thread_location(thread)
        ));
    }
    for check in readiness.checks.iter().filter(|check| check.is_failure()) {
        lines.push(format_gate_attention(
            "Failing check",
            &check.name,
            &check.status,
            check.detail.as_deref(),
        ));
    }
    for check in readiness.checks.iter().filter(|check| check.is_pending()) {
        lines.push(format_gate_attention(
            "Pending check",
            &check.name,
            &check.status,
            check.detail.as_deref(),
        ));
    }
    for deployment in readiness
        .deployments
        .iter()
        .filter(|deployment| deployment.is_failure())
    {
        lines.push(format_gate_attention(
            "Failing deployment",
            &deployment.environment,
            &deployment.status,
            deployment.url.as_deref(),
        ));
    }
    for deployment in readiness
        .deployments
        .iter()
        .filter(|deployment| deployment.is_pending())
    {
        lines.push(format_gate_attention(
            "Pending deployment",
            &deployment.environment,
            &deployment.status,
            deployment.url.as_deref(),
        ));
    }

    out.push_str("\nAttention needed:\n");
    if lines.is_empty() {
        out.push_str("- none\n");
    } else {
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
    }
}

fn append_review_entries(out: &mut String, reviews: &[PullRequestReviewEntry]) {
    out.push_str("\nLatest reviews:\n");
    if reviews.is_empty() {
        out.push_str("- none\n");
        return;
    }
    for review in reviews {
        match review.body.as_deref() {
            Some(body) => out.push_str(&format!(
                "- {}: {} - {}\n",
                review.author, review.state, body
            )),
            None => out.push_str(&format!("- {}: {}\n", review.author, review.state)),
        }
    }
}

fn append_comment_entries(out: &mut String, comments: &[PullRequestCommentEntry]) {
    out.push_str("\nComments:\n");
    if comments.is_empty() {
        out.push_str("- none\n");
        return;
    }
    for comment in comments {
        out.push_str(&format!("- {}: {}\n", comment.author, comment.body));
    }
}

fn review_thread_location(thread: &PullRequestReviewThread) -> String {
    match (thread.path.as_deref(), thread.line) {
        (Some(path), Some(line)) => format!("{path}:{line}"),
        (Some(path), None) => path.to_owned(),
        (None, Some(line)) => format!("line {line}"),
        (None, None) => "unknown location".to_owned(),
    }
}

fn format_gate_attention(prefix: &str, name: &str, status: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("- {prefix} {name}: {status} - {detail}"),
        None => format!("- {prefix} {name}: {status}"),
    }
}

fn append_review_thread_entries(out: &mut String, threads: &[PullRequestReviewThread]) {
    out.push_str("\nReview threads:\n");
    if threads.is_empty() {
        out.push_str("- none\n");
        return;
    }
    for thread in threads {
        let location = review_thread_location(thread);
        let state = if thread.resolved {
            "resolved"
        } else {
            "unresolved"
        };
        match thread.id.as_deref() {
            Some(id) => out.push_str(&format!("- {location} ({state}, {id})\n")),
            None => out.push_str(&format!("- {location} ({state})\n")),
        }
        if thread.comments.is_empty() {
            out.push_str("  - no comments\n");
        }
        for comment in &thread.comments {
            match comment.url.as_deref() {
                Some(url) => out.push_str(&format!(
                    "  - {}: {} - {}\n",
                    comment.author, comment.body, url
                )),
                None => out.push_str(&format!("  - {}: {}\n", comment.author, comment.body)),
            }
        }
    }
}

fn append_check_entries(out: &mut String, checks: &[PullRequestCheckRun]) {
    out.push_str("\nChecks:\n");
    if checks.is_empty() {
        out.push_str("- none\n");
        return;
    }
    for check in checks {
        match check.detail.as_deref() {
            Some(detail) => out.push_str(&format!(
                "- {}: {} - {}\n",
                check.name, check.status, detail
            )),
            None => out.push_str(&format!("- {}: {}\n", check.name, check.status)),
        }
    }
}

fn append_deployment_entries(out: &mut String, deployments: &[PullRequestDeployment]) {
    out.push_str("\nDeployments:\n");
    if deployments.is_empty() {
        out.push_str("- none\n");
        return;
    }
    for deployment in deployments {
        match deployment.url.as_deref() {
            Some(url) => out.push_str(&format!(
                "- {}: {} - {}\n",
                deployment.environment, deployment.status, url
            )),
            None => out.push_str(&format!(
                "- {}: {}\n",
                deployment.environment, deployment.status
            )),
        }
    }
}

pub(crate) fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let field_start = json.find(&needle)? + needle.len();
    let after_colon = json[field_start..].trim_start();
    let after_colon = after_colon.strip_prefix(':')?.trim_start();
    let after_quote = after_colon.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    Some(after_quote[..end].to_owned())
}
