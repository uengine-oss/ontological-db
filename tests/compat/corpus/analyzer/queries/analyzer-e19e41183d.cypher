UNWIND $items AS item
MATCH (node {id: item.source_id, graph_owner: 'analyzer'})-[r:READS]->(t:TABLE {id: item.target_id, graph_owner: 'analyzer'})
RETURN item.source_id AS source_id, item.target_id AS target_id,
       r.evidence_lines AS evidence_lines, r.evidence_files AS evidence_files,
       r.dml_types AS dml_types, r.evidence_locations AS evidence_locations
