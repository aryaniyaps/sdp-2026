use super::*;

impl Store {
    pub async fn job(&self, namespace: &str, id: Uuid) -> Result<Job, AppError> {
        Ok(job_from_row(
            sqlx::query("SELECT * FROM memory_jobs WHERE namespace=$1 AND id=$2")
                .bind(namespace)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(AppError::NotFound)?,
        ))
    }
    pub async fn jobs(&self, namespace: &str) -> Result<Vec<Job>, AppError> {
        Ok(sqlx::query(
            "SELECT * FROM memory_jobs WHERE namespace=$1 ORDER BY created_at DESC LIMIT 100",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(job_from_row)
        .collect())
    }
    pub async fn claim_job(&self, kinds: &[&str]) -> Result<Option<Job>, AppError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
UPDATE memory_jobs
SET status = 'failed',
    error = 'worker lease expired after final attempt',
    lease_token = NULL,
    lease_until = NULL
WHERE status = 'running'
  AND lease_until < now()
  AND attempts >= max_attempts
"#,
        )
        .execute(&mut *tx)
        .await?;
        for _ in 0..16 {
            let candidate = sqlx::query(
                r#"
SELECT j.*
FROM memory_jobs j
WHERE j.kind = ANY($1)
  AND j.attempts < j.max_attempts
  AND ((j.status = 'pending'
        AND j.available_at <= now())
       OR (j.status = 'running'
           AND j.lease_until < now()))
  AND (j.kind NOT IN ('extract',
                      'consolidate')
       OR NOT EXISTS
         (SELECT 1
          FROM memory_namespace_leases l
          WHERE l.namespace = j.namespace
            AND l.lease_until > now()))
  AND (j.kind <> 'extract'
       OR NOT EXISTS
         (SELECT 1
          FROM memory_jobs
          PRIOR
          WHERE prior.namespace = j.namespace
            AND prior.kind = 'extract'
            AND prior.status IN ('pending', 'running', 'failed')
            AND (prior.created_at, prior.id) < (j.created_at, j.id)))
  AND (j.kind <> 'consolidate'
       OR NOT EXISTS
         (SELECT 1
          FROM memory_jobs pending
          WHERE pending.namespace = j.namespace
            AND pending.kind = 'extract'
            AND pending.status IN ('pending', 'running', 'failed')))
ORDER BY j.created_at,
         j.id
FOR
UPDATE OF j SKIP LOCKED
LIMIT 1
"#,
            )
            .bind(kinds)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(candidate) = candidate else {
                tx.commit().await?;
                return Ok(None);
            };
            let id: Uuid = candidate.get("id");
            let namespace: String = candidate.get("namespace");
            let kind: String = candidate.get("kind");
            let token = Uuid::new_v4();
            if ["extract", "consolidate"].contains(&kind.as_str()) {
                let leased = sqlx::query(
                    r#"
INSERT INTO memory_namespace_leases(namespace, job_id, lease_token, lease_until)
VALUES($1,$2,$3,now() + interval '15 minutes') ON CONFLICT(namespace) DO
UPDATE
SET job_id = excluded.job_id,
    lease_token = excluded.lease_token,
    lease_until = excluded.lease_until
WHERE memory_namespace_leases.lease_until <= now()
"#,
                )
                .bind(&namespace)
                .bind(id)
                .bind(token)
                .execute(&mut *tx)
                .await?;
                if leased.rows_affected() == 0 {
                    continue;
                }
            }
            let row=sqlx::query("UPDATE memory_jobs SET status='running',attempts=attempts+1,lease_token=$2,lease_until=now()+interval '15 minutes' WHERE id=$1 RETURNING *").bind(id).bind(token).fetch_one(&mut *tx).await?;
            tx.commit().await?;
            return Ok(Some(job_from_row(row)));
        }
        tx.commit().await?;
        Ok(None)
    }

    pub async fn finish_job(
        &self,
        job: &Job,
        result: Result<Value, String>,
    ) -> Result<(), AppError> {
        match result {
            Ok(v) => {
                sqlx::query("UPDATE memory_jobs SET status='succeeded',result=$3,error=NULL,completed_at=now(),lease_until=NULL,lease_token=NULL WHERE id=$1 AND lease_token=$2 AND status='running'").bind(job.id).bind(job.lease_token).bind(v).execute(&self.pool).await?;
            }
            Err(e) => {
                sqlx::query(
                    r#"
UPDATE memory_jobs
SET status = CASE
                 WHEN attempts >= max_attempts THEN 'failed'
                 ELSE 'pending'
             END,
             error = $3,
             available_at = now() + make_interval(secs => LEAST(300, power(2, attempts)::int)),
             lease_token = NULL,
             lease_until = NULL
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
"#,
                )
                .bind(job.id)
                .bind(job.lease_token)
                .bind(e)
                .execute(&self.pool)
                .await?;
            }
        }
        sqlx::query("DELETE FROM memory_namespace_leases WHERE namespace=$1 AND lease_token=$2")
            .bind(&job.namespace)
            .bind(job.lease_token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn retry_job(&self, namespace: &str, id: Uuid) -> Result<(), AppError> {
        let n=sqlx::query("UPDATE memory_jobs SET status='pending',attempts=0,available_at=now(),error=NULL WHERE namespace=$1 AND id=$2 AND status='failed'").bind(namespace).bind(id).execute(&self.pool).await?.rows_affected();
        if n == 0 {
            return Err(AppError::Conflict("only failed jobs can be retried".into()));
        }
        Ok(())
    }
}
