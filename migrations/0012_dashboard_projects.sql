CREATE TABLE dashboard_projects (
 id text PRIMARY KEY,
 created_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO dashboard_projects(id) VALUES ('payments-api'), ('mobile-app'), ('scratch');
CREATE TABLE dashboard_sessions (
 id uuid PRIMARY KEY,
 project text NOT NULL REFERENCES dashboard_projects(id),
 namespace text NOT NULL,
 revision bigint NOT NULL DEFAULT 0,
 applied_revision bigint NOT NULL DEFAULT -1,
 updated_at timestamptz NOT NULL DEFAULT now()
);
