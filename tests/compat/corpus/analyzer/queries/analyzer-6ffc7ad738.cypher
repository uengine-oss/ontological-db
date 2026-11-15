UNWIND $items AS item
MATCH (node {id: item.source_id, graph_owner: 'analyzer'})
MATCH (t:TABLE {id: item.target_id, graph_owner: 'analyzer'})
MERGE (node)-[r:READS]->(t)
SET r.dml_type = item.dml_type,
    r.evidence_file = item.evidence_file,
    r.evidence_line = item.evidence_line,
    r.evidence_lines = item.evidence_lines,
    r.evidence_files = item.evidence_files,
    r.dml_types = item.dml_types,
    r.evidence_locations = item.evidence_locations
