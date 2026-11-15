MATCH (parent {id: $parent})
CREATE (pr:Property {
    id: $id, name: $name, type: $type, description: $desc, displayName: $dn,
    isKey: $isKey, isForeignKey: $isFk, isRequired: $isReq, fkTargetHint: $fkHint,
    parentType: $ptype, parentId: $parent,
    proposalSource: $pid, createdAt: datetime(), updatedAt: datetime()
})
MERGE (parent)-[:HAS_PROPERTY]->(pr)
