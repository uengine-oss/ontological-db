UNWIND $parent_ids as eid
MATCH (e {id: eid})-[:IMPLEMENTED_IN]->(impl:ImplementationFile)
RETURN eid as elementId,
       collect({
           id: impl.id,
           projectId: impl.projectId,
           path: impl.path,
           role: impl.role,
           lastSeenAt: impl.lastSeenAt
       }) as files
