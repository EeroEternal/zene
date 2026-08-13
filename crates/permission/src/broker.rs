//! Async approval port. Policy (`evaluate`) stays in [`PermissionGate`];
//! user interaction lives here so ACP / Cloud / tests can inject their own
//! waiter without blocking a sync mutex.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;

use crate::{default_prompter, PromptChoice};

/// One request for a human (or test double) to approve a gated tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: String,
    pub tool_call_id: Option<String>,
}

impl ApprovalRequest {
    pub fn preview(&self) -> String {
        crate::truncate(&self.arguments, 120)
    }
}

/// Transport-neutral async approval. Core must not know ACP or Cloud HTTP.
#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    async fn request(&self, request: ApprovalRequest) -> io::Result<PromptChoice>;
}

pub type SharedApprovalBroker = Arc<dyn ApprovalBroker>;

/// Always returns a configured choice. Used by tests and yolo-adjacent fakes.
#[derive(Debug, Clone, Copy)]
pub struct AutoApprovalBroker {
    pub choice: PromptChoice,
}

impl AutoApprovalBroker {
    pub fn allow_once() -> Self {
        Self {
            choice: PromptChoice::AllowOnce,
        }
    }

    pub fn deny() -> Self {
        Self {
            choice: PromptChoice::Deny,
        }
    }
}

#[async_trait]
impl ApprovalBroker for AutoApprovalBroker {
    async fn request(&self, _request: ApprovalRequest) -> io::Result<PromptChoice> {
        Ok(self.choice)
    }
}

/// Local stdin prompt, moved off the async worker via `spawn_blocking`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TerminalApprovalBroker;

#[async_trait]
impl ApprovalBroker for TerminalApprovalBroker {
    async fn request(&self, request: ApprovalRequest) -> io::Result<PromptChoice> {
        let preview = request.preview();
        let tool_name = request.tool_name;
        tokio::task::spawn_blocking(move || default_prompter(&tool_name, &preview))
            .await
            .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auto_broker_returns_configured_choice() {
        let allow = AutoApprovalBroker::allow_once();
        let deny = AutoApprovalBroker::deny();
        let request = ApprovalRequest {
            request_id: "req-1".into(),
            tool_name: "Write".into(),
            arguments: r#"{"path":"a.txt"}"#.into(),
            tool_call_id: Some("call-1".into()),
        };
        assert_eq!(
            allow.request(request.clone()).await.unwrap(),
            PromptChoice::AllowOnce
        );
        assert_eq!(deny.request(request).await.unwrap(), PromptChoice::Deny);
    }
}
