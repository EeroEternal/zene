use std::net::SocketAddr;
use zene_cloud_db::Db;
use zene_cloud_domain::{
    CreatePullRequestBody, CreateRunRequest, GithubAccountType, GithubInstallationStatus,
    GithubMode, GithubRepoSummary, PermissionMode, RegisterRequest,
};
use zene_cloud_git_broker::GitBroker;
use zene_cloud_github::{GithubClient, GithubConfig};

const TEST_RSA_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7Gf5qJlbnHSR4\n\
0QHT7w2VpmJ6jtaiezduvdUnYA4uUEq5vAl0spbcKJq8xVTWi9gWUH16ztFzPF+Q\n\
1cB9MFGCPrUqwVMjy3VWEmDQK3mReSjn2fLGGVTjB82soGdLQ7/naxmToc1fuIri\n\
KzG3fC1Lnz7RxsPn4PNBlaeL8PWg4UYYA6hq38mLv6rku+6q2XAVodvx9SBVK/5p\n\
MZFAi/siPIEqafLxfEKMgU+j3DfJdx2IG9UfpWpRQP7BIVdTrSSk1/lCv7hCRoFd\n\
gaSDW/WgJjUNuz0fY+qlkPadgzQM/0A1nKc6fDjuF/jeD9BbL80RdNKiiIp3G87M\n\
guni4qIFAgMBAAECggEAGJ3KZc/mhiDE8CpbibU9fc92zHYnkhgRCn5qYXRXWUuS\n\
EU7GlbZ7d7rV5Pk3eMTMaN8tKy+zyewLDMS6vx3Q04iJkHcAB8kYhnsDhs/5fiTJ\n\
N3vq35psmzQnIMu322SuBnYGVvCmUy42A5y4PVJWqUjp3HLAyqzDhID6msRYpNJV\n\
LMYvpUTIfSGz+c7oc5ogUwsqM7cECjQNCMgZeqDhTZ9TfKzNUd4mROjeQx9ktIv8\n\
HZ3CD5IVn2h11JehBl1Er9Nm1dvVYLLvqOBHd7AiuCamK48JOVjpeU5Hne6Y+Xf2\n\
Zy4XgR6HVDAvqxw5VwqKvHHmBUZuFdKzwuZCeMgSXwKBgQDd5UMLVGBzep/Wv5vv\n\
LQZr0sOlAlX5EMfBObCs6DxWRvnfFLnSgI5epsmm4dtI/seTzH2l569WuAQsrUrV\n\
saQK+HqYOGj7PVFA5j5U3WEPsaim3UqBX5uOs0Lr1GFLaVvIzvKhYUTV4Ru1RV5R\n\
l/RXF7NBdxNZMEmbGJPjbW2jcwKBgQDX27iR8NK98GaIG/RNQ3d8O4UnIToRKBgy\n\
S10E3QOBsFrP7PmI6hRKNN3Cy8gkMMh8yP80kV4eXIE0M1rUhue+CHW0fZbn4CXy\n\
fcRNjmN0E+N8VumEjzD58u+G7Vj8bOwTmDs64gxPAPIdSbEriMqd3Twv06MNbv9m\n\
8cMSx2J2pwKBgH0jrovVKg/2N+6EYQyh990XH/8PMi0kqYLvZhQdZOnDXWfR6Hou\n\
xhvbNB5JgcHI7gUMblACOYBOhwwrLukVJc6KE5mFNq96BTj0oHJ75yFSsCpq4nnT\n\
0YbI0hTt0XEWGg1FqNAaaxezvEyesnKRn9r+IrnozaCe+uPdGIpKTGrBAoGAf+q4\n\
TNvutxpgWGZgduz1QMyw0ohxNbuR4zQf8oLa0h7lIfSnx4gX8AW2KPrEJxY1qSUf\n\
f1Jp+QoOkxWfzPQJHuc6gXQvWkfNlQ8Mpn0r2Jz0oTmL9r84YdaiNU4v/p65o78B\n\
0pokeyjvUYXbFRZiI/z37su3A330olfApz86zV8CgYEAp9ht0x4HdxDBdrUbzCsy\n\
fCTC/EVo/bPuvNiAS6aOjA73+eu+8JBPfFYRCsuRzy8nvW3FFVEvRt1p6Qres7Rp\n\
kWDufPe7licMqMdgVvzCfXV5gMa07ejPOziPjTLdqJqSso+J0SNT+bd9gtsspDAm\n\
h6MGk70dY9y6rTcjiP1XXuw=\n\
-----END PRIVATE KEY-----";

#[tokio::test]
async fn live_clone_and_draft_pr_flow() {
    let app = axum::Router::new()
        .route(
            "/app/installations/1/access_tokens",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "token": "ghs_test_token_123",
                    "expires_at": "2030-01-01T00:00:00Z"
                }))
            }),
        )
        .route(
            "/repos/acme/demo/pulls",
            axum::routing::post(|axum::Json(body): axum::Json<serde_json::Value>| async move {
                axum::Json(serde_json::json!({
                    "number": 101,
                    "html_url": "https://github.com/acme/demo/pull/101",
                    "state": "open",
                    "draft": body.get("draft").and_then(|v| v.as_bool()).unwrap_or(true),
                    "title": body.get("title").and_then(|v| v.as_str()).unwrap_or("PR"),
                    "body": body.get("body").and_then(|v| v.as_str()),
                }))
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

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
    db.upsert_installation(
        auth.organization.id,
        "1",
        "acme",
        GithubAccountType::Organization,
        GithubInstallationStatus::Active,
    )
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
                mode_id: None,
            },
        )
        .await
        .unwrap();

    let github_config = GithubConfig {
        mode: GithubMode::Live,
        client_id: Some("client-123".into()),
        client_secret: Some("secret-123".into()),
        app_id: Some("123456".into()),
        app_private_key_pem: Some(TEST_RSA_PEM.into()),
        app_slug: Some("test-app".into()),
        api_base: format!("http://{addr}"),
        oauth_authorize_url: format!("http://{addr}/login/oauth/authorize"),
        oauth_token_url: format!("http://{addr}/login/oauth/access_token"),
    };
    let github_client = GithubClient::new(github_config);
    let broker = GitBroker::new(db.clone(), github_client);

    let token = broker.issue_read_clone_token(&run).await.unwrap();
    assert_eq!(token.mode, GithubMode::Live);
    assert_eq!(token.token, "ghs_test_token_123");

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
    assert_eq!(
        pr.url.as_deref(),
        Some("https://github.com/acme/demo/pull/101")
    );
}
