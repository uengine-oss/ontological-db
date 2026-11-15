MATCH (u:UI)
WHERE u.figmaSyncStatus IS NOT NULL
SET u.figmaSyncStatus = null,
    u.figmaSyncLastError = null,
    u.figmaSyncLastAttemptAt = null
RETURN count(u) AS n
