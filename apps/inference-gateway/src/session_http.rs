//! Publish/delete routes for any [`SessionStore`] (memory or Redis).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, post},
};
use serde::Deserialize;
use unigateway_session::{
    Fingerprint, PublishResult, SessionError, SessionHttpConfig, SessionKey, SessionPrefix,
    SessionStore,
};

pub fn session_routes(store: Arc<dyn SessionStore>, config: SessionHttpConfig) -> Router {
    Router::new()
        .route(
            &format!("{}/sessions/{{session_id}}/publish", config.path_prefix),
            post(publish_session),
        )
        .route(
            &format!("{}/sessions/{{session_id}}", config.path_prefix),
            delete(delete_session),
        )
        .with_state((store, config))
}

type SessionHttpState = (Arc<dyn SessionStore>, SessionHttpConfig);

#[derive(Deserialize)]
struct PublishBody {
    epoch: u64,
    messages: Vec<serde_json::Value>,
    #[serde(default)]
    pinned_boundary: Option<u64>,
    #[serde(default)]
    fingerprint: Option<Fingerprint>,
    #[serde(default)]
    message_count: Option<u64>,
}

async fn publish_session(
    State((store, config)): State<SessionHttpState>,
    Path(session_id): Path<String>,
    Json(body): Json<PublishBody>,
) -> Result<StatusCode, StatusCode> {
    let key = SessionKey::new(config.namespace, session_id);
    let prefix = SessionPrefix {
        epoch: body.epoch,
        messages: body.messages,
        pinned_boundary: body.pinned_boundary,
        fingerprint: body.fingerprint,
        message_count: body.message_count,
    };

    match store.publish_key(&key, prefix) {
        Ok(PublishResult::Created)
        | Ok(PublishResult::Replaced)
        | Ok(PublishResult::AlreadyCurrent) => Ok(StatusCode::NO_CONTENT),
        Err(SessionError::StaleEpoch { .. }) | Err(SessionError::EpochConflict { .. }) => {
            Err(StatusCode::CONFLICT)
        }
        Err(SessionError::Expired(_)) => Err(StatusCode::NOT_FOUND),
        Err(
            SessionError::PrefixTooLarge { .. }
            | SessionError::TailTooLarge { .. }
            | SessionError::AssembledTooLarge { .. },
        ) => Err(StatusCode::PAYLOAD_TOO_LARGE),
        Err(SessionError::Unavailable(_)) => Err(StatusCode::SERVICE_UNAVAILABLE),
        Err(_) => Err(StatusCode::BAD_REQUEST),
    }
}

async fn delete_session(
    State((store, config)): State<SessionHttpState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let key = SessionKey::new(config.namespace, session_id);
    store.delete_key(&key).map_err(|error| match error {
        SessionError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;
    Ok(StatusCode::NO_CONTENT)
}
