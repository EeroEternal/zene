-- Pending ACP session mode for RuntimeCommand::SetMode.
-- Consumed by the worker when the runtime is idle.
ALTER TABLE runs ADD COLUMN pending_mode_id TEXT;
