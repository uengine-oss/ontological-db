UNWIND $items AS item
MATCH (parent {id: item.parent_id, graph_owner: 'analyzer'})
MATCH (member {id: item.child_id, graph_owner: 'analyzer'})
MERGE (parent)-[:HAS_MEMBER]->(member)
