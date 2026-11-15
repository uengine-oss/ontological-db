MATCH (u:UI)
WHERE u.figmaSyncStatus = 'failed'
RETURN u.id AS id,
       coalesce(u.displayName, u.name, '') AS displayName,
       u.figmaSyncLastError AS lastError,
       toString(u.figmaSyncLastAttemptAt) AS lastAttemptAt,
       u.figmaSyncBindingFileKey AS bindingFileKey
ORDER BY u.figmaSyncLastAttemptAt DESC
