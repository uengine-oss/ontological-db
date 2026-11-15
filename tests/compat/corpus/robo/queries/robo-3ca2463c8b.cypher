MATCH (p {id: $pid})-[:HAS_GWT]->(g:GWT)
WHERE g.parentType = $ptype
RETURN g.givenRef AS givenRef, g.whenRef AS whenRef,
       g.thenRef AS thenRef, g.testCases AS testCases
