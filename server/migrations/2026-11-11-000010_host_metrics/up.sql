-- Resource use of each host: the latest reading (sent with the heartbeat) and a short history for the charts.
ALTER TABLE hosts ADD COLUMN metrics JSONB NOT NULL DEFAULT '{}';
ALTER TABLE hosts ADD COLUMN metrics_at TIMESTAMPTZ;

CREATE TABLE host_metrics (
  host_id UUID NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  at TIMESTAMPTZ NOT NULL,
  cpu_pct REAL,
  mem_pct REAL,
  load_1m REAL,
  PRIMARY KEY (host_id, at)
);
CREATE INDEX host_metrics_at_idx ON host_metrics(at);
