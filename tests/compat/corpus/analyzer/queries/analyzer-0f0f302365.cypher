UNWIND $items AS item
MATCH (source {id: item.source_id, graph_owner: 'analyzer'})
MATCH (target {id: item.target_id, graph_owner: 'analyzer'})
MERGE (source)-[r:READS]->(target)
