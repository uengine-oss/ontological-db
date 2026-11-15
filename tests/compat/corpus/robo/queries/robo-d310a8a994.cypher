MATCH (gwt:GWT {parentType: $parent_type, parentId: $parent_id})
WITH gwt, $refs as refs
UNWIND refs as ref
WITH gwt, ref
WHERE ref.id IS NOT NULL AND ref.type IS NOT NULL
MATCH (n {id: ref.id})
WHERE ref.type IN labels(n)
MERGE (gwt)-[:REFERENCES]->(n)
RETURN count(*) as linked
