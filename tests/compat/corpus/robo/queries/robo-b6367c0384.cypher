MATCH (p:Proposal {id:$id})
SET p.strategicDiff=$sd,
    p.tacticalDiff=$td
