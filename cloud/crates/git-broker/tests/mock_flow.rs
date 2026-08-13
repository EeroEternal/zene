use zene_cloud_db::Db;
use zene_cloud_domain::{
    CreatePullRequestBody, CreateRunRequest, GithubRepoSummary, PermissionMode, RegisterRequest,
};
use zene_cloud_git_broker::GitBroker;

#[tokio::test]
async fn mock_clone_push_and_draft_pr() {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let auth = db
        .register(RegisterRequest {
            email: "broker@example.com".into(),
            password: "password123".into(),
            display_name: "Broker".into(),
        })
        .await
        .unwrap();
    db.upsert_installation(auth.organization.id, "1", "acme", "Organization", "active")
        .await
        .unwrap();
    let repos = db
        .sync_repos_from_github(
            auth.organization.id,
            &[GithubRepoSummary {
                provider_repo_id: "55".into(),
                owner: "acme".into(),
                name: "demo".into(),
                default_branch: "main".into(),
                clone_url: "https://github.com/acme/demo.git".into(),
                private: false,
                installation_id: "1".into(),
            }],
        )
        .await
        .unwrap();
    let repo = &repos[0];

    let run = db
        .create_run(
            auth.organization.id,
            auth.user.id,
            CreateRunRequest {
                repository_id: repo.id,
                prompt: "add feature".into(),
                base_ref: Some("main".into()),
                model: "default".into(),
                permission_mode: PermissionMode::Default,
                max_turns: 50,
            },
        )
        .await
        .unwrap();

    let broker = GitBroker::mock(db.clone());
    let token = broker.issue_read_clone_token(&run).await.unwrap();
    assert_eq!(token.mode, "mock");
    assert!(token.token.starts_with("mock_clone_"));

    let pushed = broker
        .accept_bundle_and_push(&run, b"fake-bundle-bytes", Some("base"), "push-1")
        .await
        .unwrap();
    assert_eq!(pushed.head_sha.len(), 40);
    assert!(!pushed.push_url.is_empty());

    // Idempotent
    let again = broker
        .accept_bundle_and_push(&run, b"fake-bundle-bytes", Some("base"), "push-1")
        .await
        .unwrap();
    assert_eq!(again.operation_id, pushed.operation_id);
    assert_eq!(again.head_sha, pushed.head_sha);

    let pr = broker
        .create_draft_pr(
            &run,
            CreatePullRequestBody {
                title: "Agent changes".into(),
                body: Some("auto".into()),
                draft: true,
                base_ref: None,
                head_ref: None,
            },
        )
        .await
        .unwrap();
    assert!(pr.draft);
    assert!(pr.url.unwrap().contains("github.com"));
}
