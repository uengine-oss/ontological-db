MATCH (b:FigmaBinding {id: $id, currentRunId: $rid})
SET b.currentRunId = null,
    b.currentRunHolder = null
