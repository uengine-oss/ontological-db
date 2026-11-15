UNWIND $rows AS r
MATCH (bc:BoundedContext {id: r.bc_id})
MATCH (evt:Event {id: r.trigger_event_id})
MATCH (cmd:Command {id: r.invoke_command_id})
MERGE (pol:Policy {key: r.key})
  ON CREATE SET pol.id = randomUUID(),
                pol.createdAt = datetime()
SET pol.key = r.key,
    pol.name = r.name,
    pol.displayName = r.display_name,
    pol.description = r.description,
    pol.updatedAt = datetime()
MERGE (bc)-[:HAS_POLICY]->(pol)
MERGE (evt)-[:TRIGGERS {priority: 1, isEnabled: true}]->(pol)
MERGE (pol)-[:INVOKES {isAsync: true}]->(cmd)
RETURN pol {.id, .key, .name, .displayName, .description} AS result
