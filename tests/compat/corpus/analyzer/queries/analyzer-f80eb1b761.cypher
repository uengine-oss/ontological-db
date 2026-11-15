UNWIND $items AS item
MATCH (m {id: item.callee_module, graph_owner: 'analyzer'})
MERGE (p:ANALYZER_OWNED:PACKAGE {id: item.callee_package, graph_owner: 'analyzer'})
  SET p.graph_owner = 'analyzer'
MERGE (m)-[:BELONGS_TO]->(p)
