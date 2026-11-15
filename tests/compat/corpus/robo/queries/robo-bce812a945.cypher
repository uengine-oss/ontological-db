CREATE (ui:UI {
    id: randomUUID(),
    name: $name,
    displayName: $displayName,
    sequence: $sequence,
    actor: $actor,
    attachedToId: $attachedToId,
    attachedToType: $attachedToType,
    createdAt: datetime(),
    updatedAt: datetime()
})
RETURN ui.id AS id, ui.name AS name, ui.displayName AS displayName
