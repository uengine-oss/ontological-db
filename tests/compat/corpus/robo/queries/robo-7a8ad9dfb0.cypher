MATCH (u:UI)
WHERE u.figmaSyncStatus = 'failed'
RETURN u.id AS id,
       coalesce(u.displayName, u.name) AS name,
       u.figmaSyncLastError AS error,
       toString(u.figmaSyncLastAttemptAt) AS lastAttemptAt
ORDER BY u.figmaSyncLastAttemptAt DESC
