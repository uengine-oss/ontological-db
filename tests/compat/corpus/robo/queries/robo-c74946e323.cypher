MATCH (e { id: $element_id })-[:IMPLEMENTED_IN]->(impl:ImplementationFile)
WHERE impl.projectId = $project_id
RETURN impl { .path, .role, .lastSeenAt } as file
ORDER BY impl.role, impl.path
