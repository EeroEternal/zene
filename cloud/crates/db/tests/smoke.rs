use zene_cloud_db::Db;
use zene_cloud_domain::{CreateRepositoryRequest, CreateRunRequest, RegisterRequest};

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
            },
        )
        .await
        .unwrap();

    let claimed = db
        .claim_next_run("worker-1", std::path::Path::new("/tmp/zc-workspaces"))
        .await
        .unwrap()
        .expect("queued run should be claimable");
    assert_eq!(claimed.0.id, run.id);

    let events = db.events_after(run.id, 0).await.unwrap();
    assert!(!events.is_empty());
}
