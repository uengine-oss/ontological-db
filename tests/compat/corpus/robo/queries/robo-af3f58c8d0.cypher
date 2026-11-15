MATCH (u:UI {id: $uid})
RETURN coalesce(u.displayName, u.name) AS name,
       u.sceneGraph AS sceneGraph
