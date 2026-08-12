-- Optional provider/runtime cursor for stable reconnect and replay diagnostics.
-- The server-generated run_events.seq remains the canonical global ordering.
ALTER TABLE run_events ADD COLUMN cursor INTEGER;
