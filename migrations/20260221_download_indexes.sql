-- Speed up recent downloads query (ORDER BY finished_at DESC NULLS LAST, started_at DESC)
CREATE INDEX IF NOT EXISTS download_finished_at_started_at ON download(finished_at DESC, started_at DESC);

-- Speed up status-filtered aggregates (COUNT/SUM WHERE status = ...)
CREATE INDEX IF NOT EXISTS download_status ON download(status);
