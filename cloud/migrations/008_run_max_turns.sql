-- Per-run agent step budget (0 = unlimited).
ALTER TABLE runs ADD COLUMN max_turns INTEGER NOT NULL DEFAULT 50;
