CREATE (e:BindingHistoryEvent {
    id: $eid,
    eventType: $etype,
    actor: $actor,
    at: $now,
    figmaFileKey: $fk,
    payload: $payload
})
