MATCH (p:Proposal {id: $id})
RETURN p.strategicDiff AS sd, p.tacticalDiff AS td, p.journeys AS jny,
       p.statusHistory AS history, p.status AS status
