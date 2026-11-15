MATCH (parent {id: $parent_id})
WHERE $parent_type IN labels(parent)
MERGE (gwt:GWT {parentType: $parent_type, parentId: $parent_id})
ON CREATE SET gwt.id = randomUUID(),
              gwt.createdAt = datetime()
SET gwt.updatedAt = datetime(),
    gwt.givenRef = $given_ref_json,
    gwt.whenRef = $when_ref_json,
    gwt.thenRef = $then_ref_json,
    gwt.testCases = $test_cases_json
MERGE (parent)-[:HAS_GWT]->(gwt)
WITH gwt
OPTIONAL MATCH (gwt)-[r:REFERENCES]->()
DELETE r
WITH gwt
RETURN gwt {.id, .parentType, .parentId, .givenRef, .whenRef, .thenRef, .testCases} as gwt
