use zene_cloud_db::Db;
use zene_cloud_domain::{CreateRepositoryRequest, CreateRunRequest, RegisterRequest, WorkerFence};

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
    db.append_event_fenced(
        run.id,
        &fence,
        Some("fenced-event-1"),
        "runtime",
        serde_json::json!({"ok": true}),
    )
    .await
    .unwrap();

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

    let events = db.events_after(run.id, 0).await.unwrap();
    assert!(events.iter().any(|event| event.event_type == "runtime"));
}
