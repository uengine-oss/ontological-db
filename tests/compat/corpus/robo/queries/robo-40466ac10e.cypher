MATCH (r:SyncRun)
RETURN r
ORDER BY r.startedAt DESC
LIMIT $lim
