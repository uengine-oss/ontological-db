MATCH (r:SyncRun)
WHERE r.bindingFileKey = $fk
RETURN r
ORDER BY r.startedAt DESC
LIMIT $lim
