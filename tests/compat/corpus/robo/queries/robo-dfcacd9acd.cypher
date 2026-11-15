MATCH (tgt:Event)
WHERE NOT EXISTS {
    MATCH (:Command {id: $srcId})-[:EMITS]->(tgt)
}
RETURN tgt.id AS id, tgt.name AS name, tgt.displayName AS displayName
