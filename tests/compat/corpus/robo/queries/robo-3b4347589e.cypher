MATCH (bc:BoundedContext {id: $bc_id})
MATCH (evt:Event {id: $trigger_event_id})
MATCH (cmd:Command {id: $invoke_command_id})
MERGE (pol:Policy {key: $key})
ON CREATE SET pol.id = randomUUID(),
              pol.createdAt = datetime()
SET pol.key = $key,
    pol.name = $name,
    pol.displayName = $display_name,
    pol.description = $description,
    pol.updatedAt = datetime()
MERGE (bc)-[:HAS_POLICY]->(pol)
MERGE (evt)-[:TRIGGERS {priority: 1, isEnabled: true}]->(pol)
MERGE (pol)-[:INVOKES {isAsync: true}]->(cmd)
RETURN pol {.id, .key, .name, .displayName, .description} as policy
