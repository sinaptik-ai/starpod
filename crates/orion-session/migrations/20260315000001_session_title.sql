-- Add a title column for session display (set from first user message).
ALTER TABLE session_metadata ADD COLUMN title TEXT;
