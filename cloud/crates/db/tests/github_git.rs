use zene_cloud_db::Db;
use zene_cloud_domain::{
    CreateRepositoryRequest, CreateRunRequest, GitOperationKind, GitOperationStatus,
    GithubRepoSummary, RegisterRequest,
};

#[tokio::test]
async fn github_crud_and_migrations() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();

    let auth = db
        .register(RegisterRequest {
            email: "gh@example.com".into(),
            password: "password123".into(),
            display_name: "Gh".into(),
        })
        .await
        .unwrap();

    let state = db
        .save_oauth_state("state-1", Some(auth.user.id), Some("/repos"), 600)
        .await
        .unwrap();
    assert_eq!(state.state, "state-1");
    let taken = db.take_oauth_state("state-1").await.unwrap().unwrap();
    assert_eq!(taken.user_id, Some(auth.user.id));
    assert!(db.take_oauth_state("state-1").await.unwrap().is_none());

    let account = db
        .upsert_github_account(
            auth.user.id,
            "42",
            "octocat",
            "enc-token",
            "bearer",
            Some("read:user"),
        )
        .await
        .unwrap();
    assert_eq!(account.login, "octocat");

    let inst = db
        .upsert_installation(auth.organization.id, "999", "acme", "Organization", "active")
        .await
        .unwrap();
    assert_eq!(inst.installation_id, "999");
    assert_eq!(db.list_installations(auth.organization.id).await.unwrap().len(), 1);

    let synced = db
        .sync_repos_from_github(
            auth.organization.id,
            &[GithubRepoSummary {
                provider_repo_id: "100".into(),
                owner: "acme".into(),
                name: "app".into(),
                default_branch: "main".into(),
                clone_url: "https://github.com/acme/app.git".into(),
                private: true,
                installation_id: "999".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].provider_repo_id.as_deref(), Some("100"));
    assert!(synced[0].private);

    let run = db
        .create_run(
            auth.organization.id,
            auth.user.id,
            CreateRunRequest {
                repository_id: synced[0].id,
                prompt: "ship it".into(),
                base_ref: Some("main".into()),
                model: "default".into(),
                permission_mode: "default".into(),
            },
        )
        .await
        .unwrap();

    let op = db
        .create_git_operation(
            auth.organization.id,
            synced[0].id,
            run.id,
            GitOperationKind::PushBundle,
            Some("abc"),
            None,
            "idem-1",
        )
        .await
        .unwrap();
    let finished = db
        .finish_git_operation(
            op.id,
            GitOperationStatus::Succeeded,
            Some("def456"),
            None,
            Some(serde_json::json!({ "ok": true })),
        )
        .await
        .unwrap();
    assert_eq!(finished.result_head_sha.as_deref(), Some("def456"));

    let pr = db
        .create_pull_request(
            synced[0].id,
            run.id,
            "Draft",
            Some("body"),
            Some(7),
            Some("https://github.com/acme/app/pull/7"),
            None,
            Some("def456"),
            "draft",
            true,
        )
        .await
        .unwrap();
    let prs = db.list_pull_requests_for_run(run.id).await.unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].id, pr.id);

    db.append_audit(
        Some(auth.organization.id),
        "user",
        Some(&auth.user.id.to_string()),
        "test.audit",
        Some("run"),
        Some(&run.id.to_string()),
        Some(serde_json::json!({ "n": 1 })),
    )
    .await
    .unwrap();

    // Manual repo create still works after schema alter columns.
    let _ = db
        .create_repository(
            auth.organization.id,
            CreateRepositoryRequest {
                owner: "manual".into(),
                name: "repo".into(),
                default_branch: "main".into(),
                clone_url: None,
            },
        )
        .await
        .unwrap();
}
