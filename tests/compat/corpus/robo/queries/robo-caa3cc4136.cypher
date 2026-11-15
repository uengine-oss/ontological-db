MATCH (bc:BoundedContext { id: $bc_id })
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (bc)-[:HAS_READMODEL]->(rm:ReadModel)
WITH collect(DISTINCT agg) + collect(DISTINCT cmd)
     + collect(DISTINCT evt) + collect(DISTINCT rm) as elements
UNWIND elements as e
WITH e WHERE e IS NOT NULL
OPTIONAL MATCH (e)-[:IMPLEMENTED_IN]->(impl:ImplementationFile {projectId: $project_id})
WITH e,
     collect({ path: impl.path, role: impl.role, lastSeenAt: impl.lastSeenAt }) as files
RETURN
    e.id as elementId,
    head([lbl IN labels(e) WHERE lbl IN $element_labels]) as kind,
    e.name as name,
    [f IN files WHERE f.path IS NOT NULL] as files
