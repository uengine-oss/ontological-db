MATCH (b:FigmaBinding {id: $id})
WHERE b.currentRunId IS NULL AND b.status = 'active'
SET b.currentRunId = $rid,
    b.currentRunHolder = $actor
RETURN b.currentRunId AS rid
