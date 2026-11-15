UNWIND $rows as row
WITH row
WHERE row.parentType IN ['Aggregate','Command','Event','ReadModel']
  AND row.parentId IS NOT NULL AND trim(toString(row.parentId)) <> ''
  AND row.name IS NOT NULL AND trim(toString(row.name)) <> ''
  AND row.type IS NOT NULL AND trim(toString(row.type)) <> ''
MERGE (p:Property {parentType: row.parentType, parentId: row.parentId, name: row.name})
ON CREATE SET p.id = randomUUID(),
              p.createdAt = datetime()
SET p.type = row.type,
    p.description = coalesce(row.description, ''),
    p.isKey = coalesce(row.isKey, false),
    p.isForeignKey = coalesce(row.isForeignKey, false),
    p.isRequired = coalesce(row.isRequired, false),
    p.parentType = row.parentType,
    p.parentId = row.parentId,
    p.fkTargetHint = row.fkTargetHint,
    p.updatedAt = datetime()
WITH row, p
MATCH (parent {id: row.parentId})
WHERE row.parentType IN labels(parent)
MERGE (parent)-[:HAS_PROPERTY]->(p)
RETURN count(p) as upserted
