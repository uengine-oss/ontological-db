UNWIND $items AS item
MATCH (r:RULE {id: item.source_id, graph_owner: 'analyzer'})
MATCH (e:EXAMPLE {id: item.target_id, graph_owner: 'analyzer'})
MERGE (r)-[rel:HAS_EXAMPLE {occurrence_id: item.occurrence_id}]->(e)
SET rel.ordinal = item.ordinal
