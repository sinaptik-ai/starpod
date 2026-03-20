ALTER TABLE usage_stats ADD COLUMN user_id TEXT NOT NULL DEFAULT 'admin';
CREATE INDEX idx_usage_stats_user ON usage_stats(user_id, timestamp);
