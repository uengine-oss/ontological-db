MATCH (u:UI {id: $uid})
RETURN u.figmaNodeId AS fnid, coalesce(u.displayName, u.name) AS name
