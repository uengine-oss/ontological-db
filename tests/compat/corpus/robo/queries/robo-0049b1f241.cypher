MATCH (u:UI {id: $uid})
RETURN u.figmaNodeId AS fnid, u.sceneGraph AS sg, coalesce(u.displayName, u.name) AS name
