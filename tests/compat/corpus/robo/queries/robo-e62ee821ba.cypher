MATCH (p:Property {parentType: $pt, parentId: $pid, name: $name})
RETURN p.id as id, p.name as name, p.description as description,
       p.type as type, p.isKey as isKey, p.isForeignKey as isForeignKey,
       p.isRequired as isRequired, p.parentType as parentType, p.parentId as parentId
