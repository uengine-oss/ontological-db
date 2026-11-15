UNWIND $items AS item
MATCH (caller {id: item.caller_id, graph_owner: 'analyzer'})
MATCH (callee {id: item.callee_id, graph_owner: 'analyzer'})
MERGE (caller)-[r:CALLS]->(callee)
SET r.scope = item.scope,
    r.recursive = item.recursive
