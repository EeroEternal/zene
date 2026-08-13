use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;
use zene_cloud_db::Db;
use zene_cloud_domain::{
    ApprovalStatus, CreateApprovalRequest, CreateRepositoryRequest, CreateRunRequest,
    RegisterRequest, RunStatus, WorkerFence,
};

#[tokio::test]
async fn register_create_run_and_claim() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    db.ensure_dev_worker_token("dev-worker-token").await.unwrap();

    let auth = db
        .register(RegisterRequest {
            email: "a@example.com".into(),
            password: "password123".into(),
            display_name: "Ada".into(),
        })
        .await
        .unwrap();

    let repo = db
        .create_repository(
            auth.organization.id,
            CreateRepositoryRequest {
                owner: "ada".into(),
                name: "demo".into(),
                default_branch: "main".into(),
                clone_url: None,
            },
        )
        .await
        .unwrap();

    let run = db
        .create_run(
            auth.organization.id,
            auth.user.id,
            CreateRunRequest {
                repository_id: repo.id,
                prompt: "hello cloud".into(),
                base_ref: Some("main".into()),
                model: "default".into(),
                permission_mode: "default".into(),
                max_turns: 50,
            },
        )
        .await
        .unwrap();

    let stats_before = db.queue_stats().await.unwrap();
    assert_eq!(stats_before.queued, 1);
    assert_eq!(stats_before.active, 0);

    let claimed = db
        .claim_next_run("worker-1", std::path::Path::new("/tmp/zc-workspaces"))
        .await
        .unwrap()
        .expect("queued run should be claimable");
    assert_eq!(claimed.0.id, run.id);

    let stats_after = db.queue_stats().await.unwrap();
    assert_eq!(stats_after.queued, 0);
    assert_eq!(stats_after.active, 1);
    assert_eq!(stats_after.actives[0].worker_id, "worker-1");

    let fence = WorkerFence {
        attempt_id: claimed.1,
        generation: claimed.2,
        worker_id: "worker-1".into(),
    };
    db.heartbeat_fenced(run.id, &fence).await.unwrap();
    db.set_acp_session_id_fenced(run.id, &fence, "acp-session-1")
        .await
        .unwrap();
    let event = db
        .append_event_fenced_with_cursor(
            run.id,
            &fence,
            Some("fenced-event-1"),
            Some(17),
            "runtime",
            serde_json::json!({"ok": true}),
        )
        .await
        .unwrap();
    assert_eq!(event.cursor, Some(17));
    let platform_event = db
        .append_event_fenced(
            run.id,
            &fence,
            Some("platform-event-1"),
            "platform",
            serde_json::json!({"platform": true}),
        )
        .await
        .unwrap();
    assert_eq!(platform_event.cursor, None);
    let duplicate = db
        .append_event_fenced_with_cursor(
            run.id,
            &fence,
            Some("fenced-event-1"),
            Some(99),
            "runtime-retry",
            serde_json::json!({"retry": true}),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.seq, event.seq);
    assert_eq!(duplicate.cursor, Some(17));
    assert_eq!(duplicate.event_type, "runtime");

    let mut stale = fence.clone();
    stale.generation += 1;
    assert!(db.heartbeat_fenced(run.id, &stale).await.unwrap_err().to_string().contains("stale_attempt"));
    assert!(db.append_event_fenced(
        run.id,
        &stale,
        Some("stale-event"),
        "runtime",
        serde_json::json!({}),
    ).await.unwrap_err().to_string().contains("stale_attempt"));
    assert!(db
        .set_acp_session_id_fenced(run.id, &stale, "stale-session")
        .await
        .unwrap_err()
        .to_string()
        .contains("stale_attempt"));

    db.update_run_status_fenced(
        run.id,
        &fence,
        zene_cloud_domain::RunStatus::Failed,
        None,
        Some("test_retry".into()),
    )
    .await
    .unwrap();
    db.update_run_status(
        run.id,
        zene_cloud_domain::RunStatus::Queued,
        None,
        None,
    )
    .await
    .unwrap();
    let reclaimed = db
        .claim_next_run("worker-2", std::path::Path::new("/tmp/zc-workspaces"))
        .await
        .unwrap()
        .expect("failed run should be re-claimable after queueing");
    assert_eq!(reclaimed.3.as_deref(), Some("acp-session-1"));
    let replacement_fence = WorkerFence {
        attempt_id: reclaimed.1,
        generation: reclaimed.2,
        worker_id: "worker-2".into(),
    };
    let replayed = db
        .append_event_fenced_with_cursor(
            run.id,
            &replacement_fence,
            Some("fenced-event-1"),
            Some(99),
            "runtime-replayed",
            serde_json::json!({"replayed": true}),
        )
        .await
        .unwrap();
    assert_eq!(replayed.seq, event.seq);
    assert_eq!(replayed.cursor, Some(17));
    assert_eq!(replayed.event_type, "runtime");

    let events = db.events_after(run.id, 0).await.unwrap();
    let event = events
        .iter()
        .find(|event| event.event_type == "runtime")
        .expect("runtime event should be persisted");
    assert_eq!(event.cursor, Some(17));
    let replay = db.events_after_cursor(run.id, 0).await.unwrap();
    let replayed_seqs: Vec<i64> = replay.iter().map(|item| item.seq).collect();
    assert!(replayed_seqs.windows(2).all(|window| window[0] < window[1]));
    let replayed_event = replay
        .iter()
        .find(|item| item.seq == event.seq)
        .expect("cursor event should be replayed");
    assert_eq!(replayed_event.cursor, Some(17));
    let replayed_platform = replay
        .iter()
        .find(|item| item.seq == platform_event.seq)
        .expect("platform event without cursor should be replayed");
    assert_eq!(replayed_platform.cursor, None);

    let post_cursor_platform = db
        .append_event_fenced(
            run.id,
            &replacement_fence,
            Some("post-cursor-platform-event"),
            "platform",
            serde_json::json!({"after_cursor": true}),
        )
        .await
        .unwrap();
    let resumed = db.events_after_cursor(run.id, 17).await.unwrap();
    let resumed_seqs: Vec<i64> = resumed.iter().map(|item| item.seq).collect();
    assert!(resumed_seqs.windows(2).all(|window| window[0] < window[1]));
    assert!(resumed_seqs.contains(&post_cursor_platform.seq));
    assert!(resumed.iter().any(|item| {
        item.seq == post_cursor_platform.seq
            && item.event_type == "platform"
            && item.cursor.is_none()
    }));
    assert!(resumed.iter().all(|item| item.seq > event.seq));
}

async fn approval_test_run(permission_mode: &str) -> (Db, Uuid) {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let auth = db
        .register(RegisterRequest {
            email: format!("{}@example.com", Uuid::new_v4()),
            password: "password123".into(),
            display_name: "Approval test".into(),
        })
        .await
        .unwrap();
    let repo = db
        .create_repository(
            auth.organization.id,
            CreateRepositoryRequest {
                owner: "test".into(),
                name: "approval".into(),
                default_branch: "main".into(),
                clone_url: None,
            },
        )
        .await
        .unwrap();
    let run = db
        .create_run(
            auth.organization.id,
            auth.user.id,
            CreateRunRequest {
                repository_id: repo.id,
                prompt: "approval race".into(),
                base_ref: Some("main".into()),
                model: "default".into(),
                permission_mode: permission_mode.into(),
                max_turns: 10,
            },
        )
        .await
        .unwrap();
    (db, run.id)
}

fn approval_request(request_key: &str) -> CreateApprovalRequest {
    CreateApprovalRequest {
        request_key: request_key.into(),
        jsonrpc_id: Some("rpc-1".into()),
        kind: "permission".into(),
        risk: "medium".into(),
        payload: serde_json::json!({"path": "notes.txt"}),
        allowed_decisions: vec!["allow-once".into(), "reject-once".into()],
        expires_at: None,
    }
}

#[tokio::test]
async fn concurrent_approval_creation_has_one_row_and_event() {
    let (db, run_id) = approval_test_run("manual").await;
    let first = approval_request("permission-stable");
    let second = approval_request("permission-stable");

    let (left, right) = tokio::join!(
        db.clone().create_approval(run_id, first),
        db.clone().create_approval(run_id, second),
    );
    let left = left.unwrap();
    let right = right.unwrap();

    assert_eq!(left.id, right.id);
    assert_eq!(left.status, ApprovalStatus::Pending);
    let events = db.events_after(run_id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload["event"] == "approval.created")
            .count(),
        1
    );
}

#[tokio::test]
async fn concurrent_approval_decisions_have_one_winner_event() {
    let (db, run_id) = approval_test_run("manual").await;
    let approval = db
        .create_approval(run_id, approval_request("decision-stable"))
        .await
        .unwrap();

    let (left, right) = tokio::join!(
        db.clone()
            .decide_approval(approval.id, "allow-once", Some("user-a")),
        db.clone()
            .decide_approval(approval.id, "reject-once", Some("user-b")),
    );
    let left = left.unwrap();
    let right = right.unwrap();

    assert_eq!(left.status, right.status);
    assert_eq!(left.decision, right.decision);
    assert!(matches!(left.status, ApprovalStatus::Approved | ApprovalStatus::Denied));
    let events = db.events_after(run_id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload["event"] == "approval.decided")
            .count(),
        1
    );
}

#[tokio::test]
async fn worker_command_is_retried_until_fenced_ack() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let auth = db
        .register(RegisterRequest {
            email: format!("{}@example.com", Uuid::new_v4()),
            password: "password123".into(),
            display_name: "Delivery test".into(),
        })
        .await
        .unwrap();
    let repo = db
        .create_repository(auth.organization.id, CreateRepositoryRequest {
            owner: "test".into(), name: "delivery".into(), default_branch: "main".into(), clone_url: None,
        })
        .await
        .unwrap();
    let run = db.create_run(auth.organization.id, auth.user.id, CreateRunRequest {
        repository_id: repo.id, prompt: "initial".into(), base_ref: None, model: "default".into(),
        permission_mode: "default".into(), max_turns: 10,
    }).await.unwrap();
    let claimed = db.claim_next_run("worker-delivery", std::path::Path::new("/tmp/zc-workspaces"))
        .await.unwrap().unwrap();
    let fence = WorkerFence { attempt_id: claimed.1, generation: claimed.2, worker_id: "worker-delivery".into() };
    let message = db.add_message(run.id, Some(auth.user.id), "user", "follow-up", None).await.unwrap();
    let commands = db.poll_worker_commands_fenced(run.id, &fence).await.unwrap();
    assert_eq!(commands.iter().filter_map(|command| command.message_id).collect::<Vec<_>>(), vec![message.id]);
    assert!(db.poll_worker_commands_fenced(run.id, &fence).await.unwrap().iter().all(|command| command.message_id != Some(message.id)));
    let mut stale_fence = fence.clone();
    stale_fence.generation += 1;
    assert!(db.ack_worker_command_fenced(run.id, &stale_fence, message.id).await.unwrap_err().to_string().contains("stale_attempt"));
    db.ack_worker_command_fenced(run.id, &fence, message.id).await.unwrap();
    assert!(db.poll_worker_commands_fenced(run.id, &fence).await.unwrap().iter().all(|command| command.message_id != Some(message.id)));

    let message = db.add_message(run.id, Some(auth.user.id), "user", "retry me", None).await.unwrap();
    let first_claim = chrono::Utc::now();
    let _ = db.poll_worker_commands_fenced_at(run.id, &fence, first_claim).await.unwrap();
    let retry = db.poll_worker_commands_fenced_at(
        run.id,
        &fence,
        first_claim + chrono::Duration::seconds(61),
    ).await.unwrap();
    assert!(retry.iter().any(|command| command.message_id == Some(message.id)));
}

#[tokio::test]
async fn stale_waiting_for_user_attempt_is_requeued_but_approval_holds_are_not() {
    async fn make_run(db: &Db, suffix: &str) -> (zene_cloud_domain::Run, WorkerFence) {
        let auth = db.register(RegisterRequest {
            email: format!("{}-{}@example.com", suffix, Uuid::new_v4()),
            password: "password123".into(), display_name: suffix.into(),
        }).await.unwrap();
        let repo = db.create_repository(auth.organization.id, CreateRepositoryRequest {
            owner: suffix.into(), name: suffix.into(), default_branch: "main".into(), clone_url: None,
        }).await.unwrap();
        let run = db.create_run(auth.organization.id, auth.user.id, CreateRunRequest {
            repository_id: repo.id, prompt: suffix.into(), base_ref: None, model: "default".into(),
            permission_mode: "default".into(), max_turns: 10,
        }).await.unwrap();
        let claimed = db.claim_next_run("stale-policy-worker", std::path::Path::new("/tmp/zc-workspaces"))
            .await.unwrap().unwrap();
        let fence = WorkerFence { attempt_id: claimed.1, generation: claimed.2, worker_id: "stale-policy-worker".into() };
        (run, fence)
    }

    let db = Db::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let (user_run, user_fence) = make_run(&db, "user-hold").await;
    db.update_run_status_fenced(user_run.id, &user_fence, RunStatus::WaitingForUser, None, None).await.unwrap();
    let (approval_run, approval_fence) = make_run(&db, "approval-hold").await;
    db.update_run_status_fenced(approval_run.id, &approval_fence, RunStatus::WaitingForApproval, None, None).await.unwrap();

    // Both attempts use the normal 60-second claim lease. Reclamation is
    // evaluated in the future to make expiry deterministic without sleeping.
    assert_eq!(
        db.reclaim_stale_runs_at(chrono::Utc::now() + chrono::Duration::seconds(61))
            .await
            .unwrap(),
        1
    );
    assert_eq!(db.get_run(user_run.id).await.unwrap().unwrap().status, RunStatus::Queued);
    assert_eq!(db.get_run(approval_run.id).await.unwrap().unwrap().status, RunStatus::WaitingForApproval);
    // Approval lifecycle remains deliberately outside this durability slice.
}

#[tokio::test]
async fn event_cursor_migration_retries_after_partial_ddl() {
    let url = format!(
        "sqlite:file:event-cursor-retry-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db = Db::connect(&url).await.unwrap();
    db.migrate().await.unwrap();

    // Simulate a process that committed ALTER TABLE but stopped before the
    // migration marker and index were written.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("DELETE FROM schema_migrations WHERE version = '010_event_cursor'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP INDEX IF EXISTS idx_run_events_source")
        .execute(&pool)
        .await
        .unwrap();
    drop(pool);

    db.migrate().await.unwrap();
}
