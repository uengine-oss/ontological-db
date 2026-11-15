MATCH (u:UI)
RETURN u.id AS id,
       coalesce(u.displayName, u.name, '') AS displayName,
       u.sceneGraph AS sceneGraph,
       u.figmaSyncStatus AS figmaSyncStatus
ORDER BY coalesce(u.displayName, u.name), u.id
