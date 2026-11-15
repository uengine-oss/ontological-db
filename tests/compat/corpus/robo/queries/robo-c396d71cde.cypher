MATCH (pol:Policy)-[:INVOKES]->(cmd:Command {id: $cmd_id})
MATCH (trigger_evt:Event)-[:TRIGGERS]->(pol)
RETURN trigger_evt.id as trigger_event_id, trigger_evt.name as trigger_event_name
LIMIT 1
