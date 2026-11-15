MATCH (cmd:Command {id: $command_id})
OPTIONAL MATCH (evt1:Event {name: $event_name})
OPTIONAL MATCH (evt2:Event {displayName: $event_name})
OPTIONAL MATCH (evt3:Event) WHERE toLower(evt3.name) = toLower($event_name)
   OR toLower(evt3.displayName) = toLower($event_name)
WITH cmd, coalesce(evt1, evt2, evt3) AS evt
WHERE evt IS NOT NULL

// Resolve BC ownership for both sides
OPTIONAL MATCH (cmd_bc:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(cmd)
OPTIONAL MATCH (evt_bc:BoundedContext)-[:HAS_EVENT]->(evt)

RETURN evt.id AS id, evt.name AS evt_name,
       cmd_bc.id AS cmd_bc_id, cmd_bc.name AS cmd_bc_name,
       evt_bc.id AS evt_bc_id, evt_bc.name AS evt_bc_name
LIMIT 1
