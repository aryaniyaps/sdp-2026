use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{Value, json};

type GraphError = Box<dyn std::error::Error + Send + Sync>;
/// Relationship rows read per requested node before a projection is reported as truncated.
const RELATIONSHIPS_PER_NODE: usize = 20;
/// Characters of an assertion statement used as a node name.
const NAME_CHARS: usize = 60;

/// One node of the Neo4j projection: an Assertion (fact) or a KnowledgeEntity.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProjectionNode {
    pub id: String,
    /// "assertion" or "entity".
    pub kind: String,
    pub name: String,
    pub statement: Option<String>,
    pub status: Option<String>,
    pub assertion_kind: Option<String>,
    pub entity_type: Option<String>,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub revision: Option<i64>,
}
/// A MENTIONS or SUPPORTS relationship between two returned nodes.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProjectionRelationship {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub relationship_type: String,
    pub relation: Option<String>,
    pub explanation: Option<String>,
}
#[derive(Debug, Clone)]
pub struct GraphProjection {
    pub nodes: Vec<ProjectionNode>,
    pub relationships: Vec<ProjectionRelationship>,
    /// True when the namespace holds more nodes or relationships than were returned.
    pub truncated: bool,
}

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
        self.execute_batch(&[(statement, params)]).await
    }

    /// Runs several statements in one transaction request. Results are in statement order.
    pub async fn execute_batch(
        &self,
        statements: &[(&str, Value)],
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let endpoint = format!("{}/db/{}/tx/commit", self.uri, self.database);
        let statements: Vec<Value> = statements
            .iter()
            .map(|(statement, params)| json!({"statement": statement, "parameters": params}))
            .collect();
        let response = self
            .client
            .post(endpoint)
            .basic_auth(&self.user, Some(&self.password))
            .json(&json!({ "statements": statements }))
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

impl GraphStore {
    /// Reads one namespace of the projection directly from Neo4j. `limit` caps the number
    /// of nodes. Assertions come newest first and a share of the limit is kept for the
    /// entities they mention, most mentioned first, so a truncated graph still has its
    /// MENTIONS relationships. Relationships are returned only when both endpoints are
    /// returned. The MemoryNamespace gate node is never read.
    pub async fn projection(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<GraphProjection, GraphError> {
        let totals = self
            .execute_batch(&[
                (
                    "MATCH (a:Assertion {namespace:$namespace}) RETURN count(a)",
                    json!({"namespace":namespace}),
                ),
                (
                    "MATCH (e:KnowledgeEntity {namespace:$namespace}) RETURN count(e)",
                    json!({"namespace":namespace}),
                ),
            ])
            .await?;
        let assertion_total = result_count(&totals, 0)?;
        let entity_total = result_count(&totals, 1)?;
        let (assertion_limit, entity_limit) = node_budget(assertion_total, entity_total, limit);
        let first = self
            .execute_batch(&[
                (
                    "MATCH (a:Assertion {namespace:$namespace}) RETURN a.id,a.statement,a.status,a.kind,a.valid_from,a.valid_to,a.revision ORDER BY coalesce(a.revision,0) DESC,a.id LIMIT $assertion_limit",
                    json!({"namespace":namespace,"assertion_limit":assertion_limit}),
                ),
                (
                    "CALL { MATCH (a:Assertion {namespace:$namespace}) WITH a ORDER BY coalesce(a.revision,0) DESC,a.id LIMIT $assertion_limit RETURN collect(a.id) AS ids } MATCH (e:KnowledgeEntity {namespace:$namespace}) OPTIONAL MATCH (e)<-[m:MENTIONS]-(x:Assertion {namespace:$namespace}) WHERE x.id IN ids WITH e,count(m) AS degree RETURN e.id,e.name,e.entity_type ORDER BY degree DESC,e.name,e.id LIMIT $entity_limit",
                    json!({"namespace":namespace,"assertion_limit":assertion_limit,"entity_limit":entity_limit}),
                ),
            ])
            .await?;
        let mut nodes = Vec::new();
        for row in result_rows(&first, 0)? {
            let id = cell_required(row, 0, "Assertion.id")?;
            let statement = cell_text(row, 1, "Assertion.statement")?;
            nodes.push(ProjectionNode {
                name: assertion_name(&id, statement.as_deref()),
                id,
                kind: "assertion".into(),
                statement,
                status: cell_text(row, 2, "Assertion.status")?,
                assertion_kind: cell_text(row, 3, "Assertion.kind")?,
                entity_type: None,
                valid_from: cell_text(row, 4, "Assertion.valid_from")?,
                valid_to: cell_text(row, 5, "Assertion.valid_to")?,
                revision: cell_integer(row, 6, "Assertion.revision")?,
            });
        }
        for row in result_rows(&first, 1)? {
            let id = cell_required(row, 0, "KnowledgeEntity.id")?;
            nodes.push(ProjectionNode {
                name: cell_text(row, 1, "KnowledgeEntity.name")?.unwrap_or_else(|| id.clone()),
                id,
                kind: "entity".into(),
                statement: None,
                status: None,
                assertion_kind: None,
                entity_type: cell_text(row, 2, "KnowledgeEntity.entity_type")?,
                valid_from: None,
                valid_to: None,
                revision: None,
            });
        }
        let mut truncated = assertion_total + entity_total > nodes.len();
        let mut relationships = Vec::new();
        if !nodes.is_empty() {
            let ids: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
            let relationship_cap = limit * RELATIONSHIPS_PER_NODE;
            let second = self
                .execute_cypher(
                    "MATCH (a:Assertion {namespace:$namespace})-[r:MENTIONS|SUPPORTS]->(b) WHERE a.id IN $ids AND b.namespace=$namespace AND b.id IN $ids AND (b:Assertion OR b:KnowledgeEntity) RETURN a.id,b.id,type(r),r.relation,r.explanation ORDER BY a.id,b.id,type(r),r.relation LIMIT $limit",
                    json!({"namespace":namespace,"ids":ids,"limit":relationship_cap + 1}),
                )
                .await?;
            for row in result_rows(&second, 0)? {
                if relationships.len() == relationship_cap {
                    truncated = true;
                    break;
                }
                relationships.push(ProjectionRelationship {
                    from: cell_required(row, 0, "relationship source id")?,
                    to: cell_required(row, 1, "relationship target id")?,
                    relationship_type: cell_required(row, 2, "relationship type")?,
                    relation: cell_text(row, 3, "relationship relation")?,
                    explanation: cell_text(row, 4, "relationship explanation")?,
                });
            }
        }
        Ok(GraphProjection {
            nodes,
            relationships,
            truncated,
        })
    }
}

/// How many assertions and how many entities to read for a node limit. Everything is read
/// when it fits. Otherwise a quarter of the limit (at least one node, never all of it) is
/// held back for entities, and entities also take whatever the assertions leave unused.
fn node_budget(assertion_total: usize, entity_total: usize, limit: usize) -> (usize, usize) {
    let reserve = entity_total
        .min((limit / 4).max(1))
        .min(limit.saturating_sub(1));
    let assertions = assertion_total.min(limit - reserve);
    (assertions, limit - assertions)
}
/// An assertion node is named by a short single-line statement. A node that was created
/// only as the target of a SUPPORTS relationship has no statement yet and is named by id.
fn assertion_name(id: &str, statement: Option<&str>) -> String {
    let Some(statement) = statement else {
        return format!("assertion {}", id.chars().take(8).collect::<String>());
    };
    let line = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > NAME_CHARS {
        format!("{}...", line.chars().take(NAME_CHARS).collect::<String>())
    } else {
        line
    }
}
fn result_rows(payload: &Value, statement: usize) -> Result<Vec<&[Value]>, GraphError> {
    let data = payload["results"][statement]["data"]
        .as_array()
        .ok_or_else(|| GraphError::from(format!("neo4j result {statement} has no data array")))?;
    data.iter()
        .map(|entry| {
            entry["row"].as_array().map(Vec::as_slice).ok_or_else(|| {
                GraphError::from(format!(
                    "neo4j result {statement} has a row that is not a list"
                ))
            })
        })
        .collect()
}
fn result_count(payload: &Value, statement: usize) -> Result<usize, GraphError> {
    result_rows(payload, statement)?
        .first()
        .and_then(|row| row.first())
        .and_then(Value::as_u64)
        .map(|count| count as usize)
        .ok_or_else(|| GraphError::from(format!("neo4j result {statement} is not a count")))
}
fn cell_text(row: &[Value], index: usize, field: &str) -> Result<Option<String>, GraphError> {
    match row.get(index) {
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(other) => Err(format!("neo4j field {field} is not text: {other}").into()),
    }
}
fn cell_required(row: &[Value], index: usize, field: &str) -> Result<String, GraphError> {
    cell_text(row, index, field)?
        .ok_or_else(|| GraphError::from(format!("neo4j field {field} is missing")))
}
fn cell_integer(row: &[Value], index: usize, field: &str) -> Result<Option<i64>, GraphError> {
    match row.get(index) {
        Some(Value::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| GraphError::from(format!("neo4j field {field} is not an integer"))),
        Some(Value::Null) | None => Ok(None),
        Some(other) => Err(format!("neo4j field {field} is not an integer: {other}").into()),
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
    fn assertion_names_are_short_single_lines() {
        assert_eq!(
            assertion_name("id", Some("Ada\n  uses   Rust")),
            "Ada uses Rust"
        );
        let long = "\u{e9}".repeat(NAME_CHARS + 5);
        assert_eq!(
            assertion_name("id", Some(&long)),
            format!("{}...", "\u{e9}".repeat(NAME_CHARS))
        );
        // A SUPPORTS target that was never projected has no statement.
        assert_eq!(
            assertion_name("0123456789abcdef", None),
            "assertion 01234567"
        );
    }

    #[test]
    fn node_budget_keeps_entities_when_the_limit_cuts() {
        // Everything fits: nothing is held back and nothing is cut.
        assert_eq!(node_budget(310, 46, 400), (310, 90));
        assert_eq!(node_budget(0, 0, 400), (0, 400));
        // The assertions alone fill the limit: a quarter of it still goes to entities.
        assert_eq!(node_budget(310, 46, 310), (264, 46));
        assert_eq!(node_budget(2500, 900, 2000), (1500, 500));
        // Few entities leave their unused share to the assertions.
        assert_eq!(node_budget(500, 3, 100), (97, 3));
        // Assertions that do not fill their share leave the rest to entities.
        assert_eq!(node_budget(50, 400, 100), (50, 50));
        // Tiny limits keep one entity when anything has to be cut, and never go above the limit.
        assert_eq!(node_budget(4, 1, 3), (2, 1));
        assert_eq!(node_budget(4, 1, 1), (1, 0));
        assert_eq!(node_budget(0, 5, 1), (0, 1));
        for (a, e, l) in [(10, 10, 5), (1, 1, 1), (3000, 3000, 2000), (7, 0, 4)] {
            let (assertions, entities) = node_budget(a, e, l);
            assert!(assertions <= a && assertions <= l && assertions + entities == l);
        }
    }

    #[test]
    fn projection_rows_with_unexpected_types_are_errors() {
        let row = [json!(7), json!("text"), Value::Null];
        assert!(cell_text(&row, 0, "field").is_err());
        assert_eq!(
            cell_text(&row, 1, "field").unwrap().as_deref(),
            Some("text")
        );
        assert_eq!(cell_text(&row, 2, "field").unwrap(), None);
        assert_eq!(cell_integer(&row, 0, "field").unwrap(), Some(7));
        assert!(cell_integer(&row, 1, "field").is_err());
        assert!(cell_required(&row, 2, "field").is_err());
        assert!(result_count(&json!({"results":[{"data":[]}]}), 0).is_err());
        assert_eq!(
            result_count(&json!({"results":[{"data":[{"row":[3]}]}]}), 0).unwrap(),
            3
        );
    }

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
