MATCH (e { id: $element_id })
WHERE any(lbl IN $element_labels WHERE lbl IN labels(e))
CALL {
    WITH e
    UNWIND $files as f
      MERGE (impl:ImplementationFile { projectId: $project_id, path: f.path })
      ON CREATE SET impl.id = randomUUID(),
                    impl.role = f.role,
                    impl.createdAt = $now,
                    impl.lastSeenAt = $now
      ON MATCH  SET impl.role = f.role,
                    impl.lastSeenAt = $now
      MERGE (e)-[:IMPLEMENTED_IN]->(impl)
      RETURN count(*) as merged
}
OPTIONAL MATCH (e)-[r:IMPLEMENTED_IN]->()
RETURN count(r) as filesNow
