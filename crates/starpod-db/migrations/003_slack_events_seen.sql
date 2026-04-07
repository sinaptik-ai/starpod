-- Slack Socket Mode dedup table.
--
-- Slack's Socket Mode retries unacked events on the Events API retry
-- schedule (~1s, ~1m, ~5m). Our receive loop acks before handing off to
-- the handler, so duplicates are rare — but they still happen on
-- reconnect races and during container refreshes. This table makes the
-- handler idempotent per Slack event_id.
--
-- A background sweeper in the slack crate deletes rows older than 24h on
-- a ~1h cadence, keeping the table bounded regardless of traffic.

CREATE TABLE IF NOT EXISTS slack_events_seen (
    event_id   TEXT PRIMARY KEY,
    seen_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_slack_events_seen_at
    ON slack_events_seen(seen_at);
