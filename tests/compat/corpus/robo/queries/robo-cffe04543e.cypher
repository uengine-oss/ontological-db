MATCH (r:SyncRun {status:'running'})
WHERE datetime(r.startedAt) < datetime() - duration({minutes: $m})
WITH r
OPTIONAL MATCH (b:FigmaBinding {currentRunId: r.id})
SET r.status = 'aborted-binding-unreachable',
    r.finishedAt = datetime(),
    r.summary = coalesce(r.summary, '{}')
FOREACH (bb IN CASE WHEN b IS NULL THEN [] ELSE [b] END |
    SET bb.currentRunId = null, bb.currentRunHolder = null
)
RETURN count(r) AS n
