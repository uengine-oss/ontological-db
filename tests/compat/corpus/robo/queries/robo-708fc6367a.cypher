MATCH (u:UI {id: $uid})
SET u.sceneGraph = $sg, u.updatedAt = datetime()
