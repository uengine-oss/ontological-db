UNWIND $items AS item
MATCH (child {id: item.child_id, graph_owner: 'analyzer'})
MATCH (parent {id: item.parent_id, graph_owner: 'analyzer'})
MERGE (child)-[:BELONGS_TO]->(parent)
