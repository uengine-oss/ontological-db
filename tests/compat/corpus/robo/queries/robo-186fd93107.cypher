MATCH (n {id: $node_id})-[r]-(related)
RETURN DISTINCT related.id as id, related.name as name,
       labels(related)[0] as type, type(r) as relationship,
       CASE WHEN startNode(r) = n THEN 'outgoing' ELSE 'incoming' END as direction
