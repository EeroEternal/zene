-- Purge legacy mock GitHub demo data (mock-org, mock-user, etc.)
DELETE FROM pull_requests
WHERE repository_id IN (
    SELECT id FROM repositories
    WHERE provider = 'github'
      AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
);

DELETE FROM git_operations
WHERE repository_id IN (
    SELECT id FROM repositories
    WHERE provider = 'github'
      AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
);

DELETE FROM runs
WHERE repository_id IN (
    SELECT id FROM repositories
    WHERE provider = 'github'
      AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
);

DELETE FROM repositories
WHERE provider = 'github'
  AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'));

DELETE FROM github_installations
WHERE installation_id = '10001' OR account_login = 'mock-org';

DELETE FROM github_accounts
WHERE login = 'mock-user' OR github_user_id = '1001'
   OR access_token_enc LIKE 'mock_oauth%';

DELETE FROM run_clone_credentials WHERE mock = 1;
