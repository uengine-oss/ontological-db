UNWIND $parent_ids as pid
MATCH (parent {id: pid})
OPTIONAL MATCH (parent)-[:HAS_PROPERTY]->(p:Property)
OPTIONAL MATCH (p)-[:REFERENCES]->(tgt:Property)
WITH pid, p, collect(DISTINCT {
  id: tgt.id,
  name: tgt.name,
  type: tgt.type,
  isKey: tgt.isKey,
  parentType: tgt.parentType,
  parentId: tgt.parentId
}) as tgtProps
WITH pid, collect(DISTINCT {
  id: p.id,
  name: p.name,
  type: p.type,
  description: p.description,
  isKey: p.isKey,
  isForeignKey: p.isForeignKey,
  isRequired: p.isRequired,
  parentType: p.parentType,
  parentId: p.parentId,
  references: [t in tgtProps WHERE t.id IS NOT NULL]
}) as props
RETURN pid as parentId, [x in props WHERE x.id IS NOT NULL] as props
