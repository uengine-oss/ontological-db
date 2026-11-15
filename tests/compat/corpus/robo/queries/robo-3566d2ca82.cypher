MATCH (u:UI {id: $uid})
SET u.figmaSyncStatus = 'failed',
    u.figmaSyncLastError = $err,
    u.figmaSyncLastAttemptAt = datetime()
