UNWIND $items AS item
MATCH (node {id: item.source_id, graph_owner: 'analyzer'})
MATCH (t {id: item.target_id, graph_owner: 'analyzer'})
MERGE (node)-[r:REFERENCES]->(t)
SET r.evidence_line = item.evidence_line
