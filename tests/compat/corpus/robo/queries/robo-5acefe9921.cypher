MATCH (n {id: $id})
CREATE (h:EditHistory {
    id: randomUUID(), timestamp: datetime(),
    userName: $user_name, userEmail: $user_email,
    source: $source, feedback: $feedback, rationale: $rationale,
    changes: $changes_json
})
CREATE (n)-[:HAS_HISTORY]->(h)
RETURN h.id AS id
