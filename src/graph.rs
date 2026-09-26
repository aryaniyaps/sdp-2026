use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct GraphStore {
    client: Client,
    uri: String,
    user: String,
    password: String,
    database: String,
}

impl GraphStore {
    pub fn new(uri: String, user: String, password: String, database: Option<String>) -> Self {
        Self {
            client: Client::builder().build().expect("reqwest client should build"),
            uri: uri.trim_end_matches('/').to_string(),
            user,
            password,
            database: database.unwrap_or_else(|| "neo4j".to_string()),
        }
    }

    pub async fn ensure_constraints(&self) -> Result<(), Box<dyn std::error::Error>> {
        let queries = [
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        let query = build_memory_graph_statement(namespace, subject, predicate, value, memory_id, version_id);
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

    async fn execute_cypher(&self, statement: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
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
        if let Some(errors) = payload.get("errors").and_then(|v| v.as_array()) {
            if !errors.is_empty() {
                return Err(format!("neo4j graph query failed: {}", errors[0]).into());
            }
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
