-- OpenSCAP results (spec openscap §5). Append-only like the audit log: reports are evidence.
CREATE TABLE compliance_reports (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  host_id UUID NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  task_id UUID REFERENCES tasks(id) ON DELETE SET NULL,
  profile TEXT NOT NULL,
  datastream TEXT NOT NULL DEFAULT '',
  passed INT NOT NULL,
  failed INT NOT NULL,
  not_applicable INT NOT NULL,
  other INT NOT NULL DEFAULT 0,
  score DOUBLE PRECISION NOT NULL,
  failed_rules JSONB NOT NULL DEFAULT '[]',
  results_sha256 TEXT NOT NULL,
  collected_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX compliance_host_idx ON compliance_reports(host_id, created_at DESC);
CREATE FUNCTION compliance_no_update() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'compliance reports are append-only'; END $$ LANGUAGE plpgsql;
CREATE TRIGGER compliance_reports_immutable BEFORE UPDATE ON compliance_reports FOR EACH ROW EXECUTE FUNCTION compliance_no_update();
