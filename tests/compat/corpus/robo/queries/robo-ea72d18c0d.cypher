MATCH (b:FigmaBinding {id: $bid})
CREATE (r:SyncRun {
    id: $rid,
    kind: $kind,
    bindingFileKey: $fk,
    actor: $actor,
    startedAt: $now,
    status: 'running',
    summary: null,
    finishedAt: null
})
CREATE (r)-[:RUN_OF]->(b)
RETURN r
