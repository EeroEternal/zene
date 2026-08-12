-- ACP session identity used to reconnect a run after worker replacement.
ALTER TABLE run_attempts ADD COLUMN acp_session_id TEXT;
