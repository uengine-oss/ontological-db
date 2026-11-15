MATCH (u:UI {id: $uid})
SET u.sceneGraph = $sg,
    u.designSource = 'backend',
    u.updatedAt = datetime()
