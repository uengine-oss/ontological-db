UNWIND $items AS item
MATCH (node {id: item.source_id, graph_owner: 'analyzer'})
MATCH (r:RULE {id: item.target_id, graph_owner: 'analyzer'})
MERGE (node)-[rel:HAS_RULE {occurrence_id: item.occurrence_id}]->(r)
SET rel.ordinal = item.ordinal,
    rel.anchor_line = item.anchor_line
REMOVE rel.coupled_domains
