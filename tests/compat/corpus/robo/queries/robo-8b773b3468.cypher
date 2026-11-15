MATCH (b:FigmaBinding {id: $bid})
CREATE (e:BindingHistoryEvent {
    id: $eid,
    eventType: $etype,
    actor: $actor,
    at: $now,
    figmaFileKey: $fk,
    payload: $payload
})
CREATE (e)-[:LOGGED]->(b)
