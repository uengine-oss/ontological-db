MATCH (p:Proposal {id: $id})
SET p.status = 'ACCEPTED',
    p.acceptedAt = datetime($at),
    p.statusHistory = $history,
    p.sandboxStatus = 'DESTROYED'
