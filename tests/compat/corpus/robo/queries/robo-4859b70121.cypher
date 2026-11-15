UNWIND $node_ids as evtId
UNWIND $node_ids as polId
MATCH (evt:Event {id: evtId})-[r:TRIGGERS]->(pol:Policy {id: polId})
RETURN DISTINCT {
    source: evt.id,
    target: pol.id,
    type: 'TRIGGERS'
} as relationship

UNION

// Policy → INVOKES → Command (cross-BC)
UNWIND $node_ids as polId
UNWIND $node_ids as cmdId
MATCH (pol:Policy {id: polId})-[r:INVOKES]->(cmd:Command {id: cmdId})
RETURN DISTINCT {
    source: pol.id,
    target: cmd.id,
    type: 'INVOKES'
} as relationship
