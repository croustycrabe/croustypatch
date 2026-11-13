-- Automatic patches run inside their patch day: a task planned for a window must not start after the window closed
-- (a host that was offline all day shows "missed patch day" instead of patching at a random hour).
ALTER TABLE tasks ADD COLUMN deadline_at TIMESTAMPTZ;
