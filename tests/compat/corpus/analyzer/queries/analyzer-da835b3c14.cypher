UNWIND $items AS item
MATCH (prev {id: item.prev_id, graph_owner: 'analyzer'})
MATCH (next {id: item.next_id, graph_owner: 'analyzer'})
MERGE (prev)-[:NEXT]->(next)
