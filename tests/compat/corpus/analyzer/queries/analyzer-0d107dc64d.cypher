UNWIND $items AS item
MATCH (parent {id: item.parent_id, graph_owner: 'analyzer'})
MATCH (child {id: item.child_id, graph_owner: 'analyzer'})
MERGE (parent)-[:PARENT_OF]->(child)
