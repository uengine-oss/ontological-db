MATCH (cmd:Command {id: $cmd})
CREATE (g:Given {
    id: $id, name: $name, description: $desc,
    parentType: 'Command', parentId: $cmd,
    fieldValues: $fv, scenario: $scenario,
    proposalSource: $pid, createdAt: datetime()
})
MERGE (cmd)-[:HAS_GIVEN]->(g)
