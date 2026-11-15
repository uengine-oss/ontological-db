UNWIND $node_ids as node_id
MATCH (parent {id: node_id})
OPTIONAL MATCH (parent)-[:HAS_GWT]->(gwt:GWT)
RETURN node_id,
       gwt {.id, .givenRef, .whenRef, .thenRef, .testCases} as gwt
