-- Inference may run concurrently across banks, but never within one bank.
CREATE TABLE memory_namespace_leases (
 namespace text PRIMARY KEY,
 job_id uuid NOT NULL REFERENCES memory_jobs(id) ON DELETE CASCADE,
 lease_token uuid NOT NULL,
 lease_until timestamptz NOT NULL
);
INSERT INTO memory_namespace_leases(namespace,job_id,lease_token,lease_until)
 SELECT DISTINCT ON(namespace) namespace,id,lease_token,lease_until FROM memory_jobs
 WHERE kind IN ('extract','consolidate') AND status='running' AND lease_token IS NOT NULL
 ORDER BY namespace,created_at,id;
CREATE INDEX memory_jobs_namespace_order ON memory_jobs(namespace,kind,created_at,id)
 WHERE status IN ('pending','running','failed');
