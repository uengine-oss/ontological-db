MATCH (cmd:Command)-[:HAS_GWT]->(g:GWT)
RETURN cmd.id AS cmdId,
       g.givenRef AS givenRef, g.whenRef AS whenRef,
       g.thenRef AS thenRef, g.testCases AS testCases
