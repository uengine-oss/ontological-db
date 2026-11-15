MATCH (r:SyncRun {id: $rid})
WHERE r.status = 'running'
SET r.status = $status,
    r.finishedAt = $now,
    r.summary = $summary
