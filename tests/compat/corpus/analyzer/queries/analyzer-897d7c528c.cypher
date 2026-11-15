UNWIND $items AS item
MATCH (caller {id: item.caller_id, graph_owner: 'analyzer'})
MERGE (callee:ANALYZER_OWNED:FUNCTION {id: item.callee_id, graph_owner: 'analyzer'})
  SET callee.external = true,
      callee.name = coalesce(callee.name, item.callee_name),
      callee.type = coalesce(callee.type, 'FUNCTION'),
      callee.graph_owner = 'analyzer'
MERGE (m:ANALYZER_OWNED:MODULE {id: item.callee_module, graph_owner: 'analyzer'})
  SET m.external = true,
      m.name = coalesce(m.name, item.callee_name),
      m.type = coalesce(m.type, 'MODULE'),
      m.graph_owner = 'analyzer'
MERGE (m)-[:HAS_MEMBER]->(callee)
MERGE (caller)-[r:CALLS]->(callee)
SET r.scope = item.scope,
    r.recursive = item.recursive
