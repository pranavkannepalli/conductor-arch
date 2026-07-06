use anyhow::Result;

use crate::github_pr::PullRequestReadiness;
use crate::workspace::{
    Checkpoint, PullRequest, PullRequestPanelState, ReviewComment, WorkspaceStore,
};

pub struct GitReviewService<'a> {
    store: &'a WorkspaceStore,
}

impl<'a> GitReviewService<'a> {
    pub fn new(store: &'a WorkspaceStore) -> Self {
        Self { store }
    }

    pub fn pull_request(&self, workspace_name: &str) -> Result<Option<PullRequest>> {
        self.store.pull_request(workspace_name)
    }

    pub fn pull_request_panel_state(&self, workspace_name: &str) -> Result<PullRequestPanelState> {
        self.store.pull_request_panel_state(workspace_name)
    }

    pub fn pull_request_readiness(&self, workspace_name: &str) -> Result<PullRequestReadiness> {
        self.store.pull_request_readiness(workspace_name)
    }

    pub fn refresh_pull_request_state(&self, workspace_name: &str) -> Result<Option<PullRequest>> {
        self.store.refresh_pull_request_state(workspace_name)
    }

    pub fn merge_pull_request(&self, workspace_name: &str, method: Option<&str>) -> Result<String> {
        self.store.merge_pull_request(workspace_name, method)
    }

    pub fn add_review_comment(
        &self,
        workspace_name: &str,
        file_path: &str,
        line_number: Option<i64>,
        body: &str,
    ) -> Result<ReviewComment> {
        self.store
            .add_review_comment(workspace_name, file_path, line_number, body)
    }

    pub fn list_review_comments(&self, workspace_name: &str) -> Result<Vec<ReviewComment>> {
        self.store.list_review_comments(workspace_name)
    }

    pub fn review_comments_agent_prompt(&self, workspace_name: &str) -> Result<String> {
        self.store.review_comments_agent_prompt(workspace_name)
    }

    pub fn resolve_review_comment(&self, id: i64) -> Result<ReviewComment> {
        self.store.resolve_review_comment(id)
    }

    pub fn checkpoint_create(
        &self,
        workspace_name: &str,
        message: &str,
        session_id: Option<i64>,
    ) -> Result<Checkpoint> {
        self.store
            .checkpoint_create(workspace_name, message, session_id)
    }

    pub fn checkpoint_list(&self, workspace_name: &str) -> Result<Vec<Checkpoint>> {
        self.store.checkpoint_list(workspace_name)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn git_review_service_exposes_review_and_pr_boundary_without_gtk_dependency() {
        let source = include_str!("git_review_service.rs");

        assert!(source.contains("pub struct GitReviewService"));
        assert!(source.contains("pull_request_panel_state"));
        assert!(source.contains("list_review_comments"));
        assert!(source.contains("checkpoint_create"));
        assert!(!source.contains(concat!("use ", "gtk")));
        assert!(!source.contains(concat!("gtk", "::")));
        assert!(!source.contains(concat!("gtk", "4")));
    }
}
