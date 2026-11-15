MATCH (b:FigmaBinding {id: $id})
WHERE b.currentRunId IS NOT NULL
RETURN b.currentRunId AS rid, b.currentRunHolder AS holder
