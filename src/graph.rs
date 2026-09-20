use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct GraphStore {
    client: Client,
    uri: String,
    user: String,
    password: String,
    database: String,
}

impl GraphStore {
    pub async fn project_assertion(
        &self,
        assertion: &crate::knowledge::AssertionView,
        revision: i64,
        entities: Vec<Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.execute_cypher(r#"
          MERGE (gate:MemoryNamespace {namespace:$namespace})
          SET gate.lock=coalesce(gate.lock,0)+1
          WITH gate WHERE $revision>=coalesce(gate.min_revision,0)
          MERGE (a:Assertion {namespace:$namespace,id:$id})
          WITH a WHERE coalesce(a.revision,0)<=$revision
          SET a.statement=$statement,a.status=$status,a.kind=$kind,a.valid_from=$valid_from,a.valid_to=$valid_to,a.revision=$revision
          WITH a
          OPTIONAL MATCH (a)-[old:MENTIONS|SUPPORTS|SUPERSEDES|EXTENDS|CONTRADICTS|CAUSES]->() DELETE old
          WITH DISTINCT a
          FOREACH (e IN $entities |
            MERGE (n:KnowledgeEntity {namespace:$namespace,id:e.id})
            SET n.name=e.name,n.entity_type=e.entity_type
            MERGE (a)-[:MENTIONS]->(n))
          FOREACH (r IN $relations |
            MERGE (b:Assertion {namespace:$namespace,id:r.assertion_id})
            MERGE (a)-[edge:SUPPORTS {relation:r.relation}]->(b)
            SET edge.explanation=r.explanation)
          RETURN a.id
        "#,json!({"namespace":assertion.namespace,"id":assertion.id,"statement":assertion.statement,"status":assertion.status,"kind":assertion.kind,"valid_from":assertion.valid_from,"valid_to":assertion.valid_to,"revision":revision,"entities":entities,"relations":assertion.relations})).await?;
        Ok(())
    }
    pub async fn clear_namespace(
        &self,
        namespace: &str,
        min_revision: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.execute_cypher(r#"
          MERGE (gate:MemoryNamespace {namespace:$namespace})
          SET gate.lock=coalesce(gate.lock,0)+1,
              gate.min_revision=CASE WHEN coalesce(gate.min_revision,0)>$min_revision THEN gate.min_revision ELSE $min_revision END
          WITH gate
          OPTIONAL MATCH (a:Assertion {namespace:$namespace}) WHERE coalesce(a.revision,0)<gate.min_revision
          DETACH DELETE a
          WITH DISTINCT gate
          OPTIONAL MATCH (e:KnowledgeEntity {namespace:$namespace}) WHERE NOT (e)--(:Assertion)
          DETACH DELETE e
        "#,json!({"namespace":namespace,"min_revision":min_revision})).await?;
        Ok(())
    }
    async fn neighbor_round(
        &self,
        namespace: &str,
        seeds: &[uuid::Uuid],
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.execute_cypher(
            r#"
          UNWIND $seeds AS seed_id
          MATCH (seed:Assertion {namespace:$namespace,id:seed_id})
          CALL {
            WITH seed
            CALL {
              WITH seed
              MATCH (seed)-[:MENTIONS|SUPPORTS]-(via)
              WHERE via.namespace=$namespace AND (via:Assertion OR via:KnowledgeEntity)
              RETURN via ORDER BY via.id LIMIT 20
            }
            CALL {
              WITH seed,via
              WITH seed,via WHERE via:Assertion AND via.id<>seed.id
              RETURN via AS other,[seed.id,via.id] AS path
              UNION ALL
              WITH seed,via
              MATCH (via:KnowledgeEntity)<-[:MENTIONS]-(other:Assertion)
              WHERE other.namespace=$namespace AND other.id<>seed.id
              RETURN other,[seed.id,via.id,other.id] AS path ORDER BY other.id LIMIT 20
            }
            RETURN other,path ORDER BY other.id LIMIT 20
          }
          WITH other,path WHERE NOT other.id IN $seeds
          RETURN other.id,path LIMIT 100
        "#,
            json!({"namespace":namespace,"seeds":seeds}),
        )
        .await
    }
    pub async fn neighbors(
        &self,
        namespace: &str,
        seeds: &[uuid::Uuid],
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let first = self
            .neighbor_round(namespace, &seeds[..seeds.len().min(20)])
            .await?;
        let first_rows = first["results"][0]["data"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut frontier = Vec::new();
        for row in &first_rows {
            if let Some(id) = row["row"][0]
                .as_str()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                && !frontier.contains(&id)
            {
                frontier.push(id);
            }
            if frontier.len() == 20 {
                break;
            }
        }
        let mut rows = first_rows.clone();
        if !frontier.is_empty() {
            let second = self.neighbor_round(namespace, &frontier).await?;
            if let Some(next_rows) = second["results"][0]["data"].as_array() {
                for row in next_rows {
                    let Some(tail) = row["row"][1].as_array() else {
                        continue;
                    };
                    let Some(origin) = tail.first() else {
                        continue;
                    };
                    if let Some(prefix) = first_rows
                        .iter()
                        .find(|r| &r["row"][0] == origin)
                        .and_then(|r| r["row"][1].as_array())
                    {
                        let mut path = prefix.clone();
                        path.extend_from_slice(&tail[1..]);
                        if path
                            .iter()
                            .enumerate()
                            .any(|(i, node)| path[..i].contains(node))
                        {
                            continue;
                        }
                        rows.push(json!({"row":[row["row"][0],path]}));
                    }
                    if rows.len() >= 200 {
                        break;
                    }
                }
            }
        }
        Ok(json!({"results":[{"data":rows}]}))
    }
    pub fn new(uri: String, user: String, password: String, database: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("reqwest client should build"),
            uri: uri.trim_end_matches('/').to_string(),
            user,
            password,
            database: database.unwrap_or_else(|| "neo4j".to_string()),
        }
    }

    pub async fn ensure_constraints(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let queries = [
            "CREATE CONSTRAINT memory_namespace_identity IF NOT EXISTS FOR (n:MemoryNamespace) REQUIRE n.namespace IS UNIQUE",
            "CREATE CONSTRAINT assertion_identity IF NOT EXISTS FOR (a:Assertion) REQUIRE (a.namespace, a.id) IS UNIQUE",
            "CREATE CONSTRAINT knowledge_entity_identity IF NOT EXISTS FOR (e:KnowledgeEntity) REQUIRE (e.namespace, e.id) IS UNIQUE",
            "CREATE CONSTRAINT entity_key IF NOT EXISTS FOR (e:Entity) REQUIRE (e.namespace, e.name) IS UNIQUE",
            "CREATE CONSTRAINT claim_key IF NOT EXISTS FOR (c:Claim) REQUIRE (c.namespace, c.key) IS UNIQUE",
            "CREATE CONSTRAINT version_key IF NOT EXISTS FOR (v:MemoryVersion) REQUIRE (v.namespace, v.version_id) IS UNIQUE",
        ];

        for q in queries {
            self.execute_cypher(q, json!({})).await?;
        }

        Ok(())
    }

    pub async fn upsert_memory(
        &self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        value: &str,
        memory_id: &str,
        version_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = build_memory_graph_statement(
            namespace, subject, predicate, value, memory_id, version_id,
        );
        let params = json!({
            "namespace": namespace,
            "subject": subject,
            "predicate": predicate,
            "value": value,
            "statement": format!("{} {} {}.", subject, predicate, value),
            "memory_id": memory_id,
            "version_id": version_id,
            "claim_key": format!("{}::{}", subject.to_ascii_lowercase(), predicate.to_ascii_lowercase())
        });
        self.execute_cypher(&query, params).await?;
        Ok(())
    }

    pub async fn execute_cypher(
        &self,
        statement: &str,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let endpoint = format!("{}/db/{}/tx/commit", self.uri, self.database);
        let response = self
            .client
            .post(endpoint)
            .basic_auth(&self.user, Some(&self.password))
            .json(&json!({
                "statements": [{
                    "statement": statement,
                    "parameters": params
                }]
            }))
            .send()
            .await?;

        if response.status() != StatusCode::OK {
            let text = response.text().await.unwrap_or_default();
            return Err(format!("neo4j graph request failed: {}", text).into());
        }

        let payload: Value = response.json().await?;
        if let Some(errors) = payload.get("errors").and_then(|v| v.as_array())
            && !errors.is_empty()
        {
            return Err(format!("neo4j graph query failed: {}", errors[0]).into());
        }

        Ok(payload)
    }
}

pub fn build_memory_graph_statement(
    _namespace: &str,
    _subject: &str,
    _predicate: &str,
    _value: &str,
    _memory_id: &str,
    _version_id: &str,
) -> String {
    String::from(
        r#"
        MERGE (entity:Entity {namespace: $namespace, name: $subject})
        MERGE (claim:Claim {namespace: $namespace, key: $claim_key, statement: $statement})
        MERGE (version:MemoryVersion {namespace: $namespace, version_id: $version_id, memory_id: $memory_id})
        MERGE (entity)-[:HAS_CLAIM]->(claim)
        MERGE (claim)-[:VERSION_OF]->(version)
        SET claim.predicate = $predicate,
            claim.value = $value,
            version.subject = $subject,
            version.predicate = $predicate,
            version.value = $value,
            version.memory_id = $memory_id
        RETURN entity, claim, version
        "#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_graph_statement_builds_expected_entities() {
        let cypher = build_memory_graph_statement(
            "review",
            "Aryan",
            "preferred programming language",
            "Rust",
            "memory-123",
            "version-456",
        );

        assert!(cypher.contains("MERGE (entity:Entity"));
        assert!(cypher.contains("MERGE (claim:Claim"));
        assert!(cypher.contains("MERGE (version:MemoryVersion"));
        assert!(cypher.contains("MERGE (entity)-[:HAS_CLAIM]->(claim)"));
        assert!(cypher.contains("version_id: $version_id"));
    }
}
