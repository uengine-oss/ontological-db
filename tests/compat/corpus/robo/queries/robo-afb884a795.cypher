MATCH (p:Proposal {id: $id})
SET p.status = 'PENDING_ACCEPTANCE',
    p.statusHistory = $history
REMOVE p.acceptedAt
