MATCH (p:Proposal {id: $id})
SET p.status = 'DESTROYED',
    p.destroyedAt = datetime($at),
    p.statusHistory = $history,
    p.sandboxStatus = 'DESTROYED'
