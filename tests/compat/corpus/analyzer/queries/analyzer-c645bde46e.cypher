UNWIND $items AS item
MATCH (node {id: item.source_id, graph_owner: 'analyzer'})
MATCH (q:QUESTION {id: item.target_id, graph_owner: 'analyzer'})
MERGE (node)-[:HAS_QUESTION]->(q)
