MATCH (p:Property {parentType: $pt, parentId: $pid})
RETURN p.id as id, p.name as name, p.type as type
