UNWIND $node_ids as sourceId
UNWIND $node_ids as targetId
MATCH (source {id: sourceId})-[r]->(target {id: targetId})
WHERE sourceId <> targetId
RETURN DISTINCT {
    source: source.id,
    target: target.id,
    type: type(r)
} as relationship
