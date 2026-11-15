MERGE (p:Property {parentType: $parent_type, parentId: $parent_id, name: $name})
ON CREATE SET p.id = randomUUID()
SET p.type = $type,
    p.description = $description,
    p.isKey = $is_key,
    p.isForeignKey = $is_fk,
    p.isRequired = $is_required,
    p.parentType = $parent_type,
    p.parentId = $parent_id
WITH p
MATCH (parent {id: $parent_id})
WHERE $parent_type IN labels(parent)
MERGE (parent)-[:HAS_PROPERTY]->(p)
RETURN p.id as id
