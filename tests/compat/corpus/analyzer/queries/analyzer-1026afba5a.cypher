UNWIND $items AS item
MATCH (f {id: item.from_id, graph_owner: 'analyzer'})
MATCH (s {id: item.source_id, graph_owner: 'analyzer'})
MERGE (f)-[:INIT_BY]->(s)
