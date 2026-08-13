//! Runtime-owned approval broker. Transports send `RuntimeCommand::Approval`;
//! this waiter does not know ACP or Cloud HTTP.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use zene_permission::{ApprovalBroker, ApprovalRequest, PromptChoice};
use zene_runtime::{ApprovalDecision, ApprovalWaiters, RuntimeEventPublisher};
use zene_turn::RuntimeEventKind;

pub fn prompt_choice(decision: ApprovalDecision) -> PromptChoice {
    match decision {
        ApprovalDecision::AllowOnce => PromptChoice::AllowOnce,
        ApprovalDecision::AllowSession => PromptChoice::AllowSession,
        ApprovalDecision::Deny => PromptChoice::Deny,
    }
}

pub struct RuntimeOwnedBroker {
    waiters: Arc<ApprovalWaiters>,
    publisher: RuntimeEventPublisher,
}

impl RuntimeOwnedBroker {
    pub fn new(waiters: Arc<ApprovalWaiters>, publisher: RuntimeEventPublisher) -> Self {
        Self { waiters, publisher }
    }
}

#[async_trait]
impl ApprovalBroker for RuntimeOwnedBroker {
    async fn request(&self, request: ApprovalRequest) -> io::Result<PromptChoice> {
        let request_id = request.request_id.clone();
        let rx = self.waiters.register(request_id.clone());
        self.publisher
            .publish_kind(RuntimeEventKind::ApprovalRequested {
                request_id: request_id.clone(),
                tool_name: request.tool_name,
                arguments: request.arguments,
                tool_call_id: request.tool_call_id,
            });
        match rx.await {
            Ok(decision) => {
                self.publisher
                    .publish_kind(RuntimeEventKind::ApprovalResolved {
                        request_id,
                        allowed: decision.allowed(),
                    });
                Ok(prompt_choice(decision))
            }
            Err(_) => Err(io::Error::other("approval waiter dropped")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{broadcast, watch};
    use zene_runtime::ExecutionState;
    use zene_turn::SessionId;

    #[tokio::test]
    async fn broker_waits_until_runtime_resolves_approval() {
        let waiters = Arc::new(ApprovalWaiters::new());
        let (events, _) = broadcast::channel(8);
        let (state, _) = watch::channel(ExecutionState::Idle);
        let publisher =
            RuntimeEventPublisher::new(events, state, SessionId::from_string("session"));
        let broker = RuntimeOwnedBroker::new(Arc::clone(&waiters), publisher);
        let pending = tokio::spawn(async move {
            broker
                .request(ApprovalRequest {
                    request_id: "req-1".into(),
                    tool_name: "Bash".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                    tool_call_id: Some("call-1".into()),
                })
                .await
        });
        tokio::task::yield_now().await;
        waiters
            .resolve("req-1", ApprovalDecision::AllowSession)
            .unwrap();
        assert_eq!(pending.await.unwrap().unwrap(), PromptChoice::AllowSession);
    }
}
