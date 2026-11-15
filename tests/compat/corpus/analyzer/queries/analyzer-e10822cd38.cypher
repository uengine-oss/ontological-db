UNWIND $items AS item
MATCH (t:TABLE {id: item.parent_id, graph_owner: 'analyzer'})
MATCH (c:COLUMN {id: item.child_id, graph_owner: 'analyzer'})
MERGE (t)-[:HAS_COLUMN]->(c)
