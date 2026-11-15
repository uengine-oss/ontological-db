MATCH (tgt:ReadModel)
WHERE NOT EXISTS {
    MATCH (tgt)-[:HAS_CQRS]->(:CQRSConfig)-[:HAS_OPERATION]->(:CQRSOperation)-[:TRIGGERED_BY]->(:Event {id: $srcId})
}
RETURN tgt.id AS id, tgt.name AS name, tgt.displayName AS displayName
