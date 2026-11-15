MATCH (parent {id: $parent_id})
WHERE $parent_type IN labels(parent)
MERGE (gwt:GWT {parentType: $parent_type, parentId: $parent_id})
  ON CREATE SET gwt.id = randomUUID(),
                gwt.createdAt = datetime(),
                gwt.testCases = '[]'
SET gwt.updatedAt = datetime(),
    gwt.givenRef = coalesce($given_ref, gwt.givenRef),
    gwt.whenRef  = coalesce($when_ref, gwt.whenRef),
    gwt.thenRef  = coalesce($then_ref, gwt.thenRef),
    gwt.testCases = CASE WHEN $test_cases = '[]'
                         THEN coalesce(gwt.testCases, '[]')
                         ELSE $test_cases END
MERGE (parent)-[:HAS_GWT]->(gwt)
RETURN gwt.id AS id
