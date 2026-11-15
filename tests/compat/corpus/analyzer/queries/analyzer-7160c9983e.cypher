UNWIND $items AS item
MATCH (from:TABLE {id: item.from_id, graph_owner: 'analyzer'})
MATCH (to:TABLE {id: item.to_id, graph_owner: 'analyzer'})
MERGE (from)-[r:FK {from_column: item.from_column, to_column: item.to_column}]->(to)
SET r.fk_type = item.fk_type,
    r.is_candidate = item.is_candidate
