CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT NOT NULL,
    repository_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (organization_id, repository_id)
);

CREATE INDEX IF NOT EXISTS idx_workspaces_repo
    ON workspaces(repository_id);

ALTER TABLE runs ADD COLUMN workspace_id TEXT;

INSERT OR IGNORE INTO workspaces (id, organization_id, repository_id, created_at)
SELECT lower(hex(randomblob(16))), organization_id, repository_id, MIN(created_at)
FROM runs
GROUP BY organization_id, repository_id;

UPDATE runs
SET workspace_id = (
    SELECT w.id FROM workspaces w
    WHERE w.organization_id = runs.organization_id
      AND w.repository_id = runs.repository_id
)
WHERE workspace_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_runs_workspace
    ON runs(workspace_id);
