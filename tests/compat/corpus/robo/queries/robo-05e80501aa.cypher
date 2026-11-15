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
WITH gwt, $refs as refs
UNWIND refs as ref
WITH gwt, ref
WHERE ref.id IS NOT NULL AND ref.type IS NOT NULL
MATCH (n {id: ref.id})
WHERE ref.type IN labels(n)
MERGE (gwt)-[:REFERENCES]->(n)
RETURN gwt.id as id
