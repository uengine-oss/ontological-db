UNWIND $items AS item
MATCH (e:EXAMPLE {id: item.source_id, graph_owner: 'analyzer'})
MATCH (t:TABLE {id: item.target_id, graph_owner: 'analyzer'})
MERGE (e)-[:AFFECTS_TABLE {op: item.op}]->(t)
