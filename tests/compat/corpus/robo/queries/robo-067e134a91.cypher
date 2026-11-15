MATCH (n {id: $target_id})
SET n.description = coalesce($description, n.description),
    n.template = coalesce($template, n.template),
    n.attachedToId = coalesce($attached_to_id, n.attachedToId),
    n.attachedToType = coalesce($attached_to_type, n.attachedToType),
    n.attachedToName = coalesce($attached_to_name, n.attachedToName),
    n.updatedAt = datetime()
RETURN n.id as id
