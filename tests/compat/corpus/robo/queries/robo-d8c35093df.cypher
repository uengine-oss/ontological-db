UNWIND $ids as eid
MATCH (e {id: eid})-[:IMPLEMENTED_IN]->(impl:ImplementationFile)
WHERE $pid IS NULL OR impl.projectId = $pid
RETURN eid as elementId,
       collect({path: impl.path, role: impl.role, lastSeenAt: impl.lastSeenAt}) as files
