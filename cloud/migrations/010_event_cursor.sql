-- Optional provider/runtime cursor for stable reconnect and replay diagnostics.
-- The server-generated run_events.seq remains the canonical global ordering.
ALTER TABLE run_events ADD COLUMN cursor INTEGER;

-- A provider event is unique for a run, not for one worker attempt. Keep the
-- earliest canonical event when upgrading databases that already contain
-- duplicates from replacement attempts before this migration.
DROP INDEX IF EXISTS idx_run_events_source;
DELETE FROM run_events
 WHERE source_event_id IS NOT NULL
   AND seq > (
       SELECT MIN(previous.seq)
         FROM run_events AS previous
        WHERE previous.run_id = run_events.run_id
          AND previous.source_event_id = run_events.source_event_id
   );
CREATE UNIQUE INDEX IF NOT EXISTS idx_run_events_source
    ON run_events(run_id, source_event_id)
    WHERE source_event_id IS NOT NULL;
