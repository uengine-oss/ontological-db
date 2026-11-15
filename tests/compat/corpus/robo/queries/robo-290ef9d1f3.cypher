UNWIND $rows AS r
MATCH (bc:BoundedContext {id: r.bc_id})
MERGE (rm:ReadModel {key: r.key})
  ON CREATE SET rm.id = randomUUID(),
                rm.createdAt = datetime()
SET rm.key = r.key,
    rm.name = r.name,
    rm.displayName = r.display_name,
    rm.description = r.description,
    rm.provisioningType = r.provisioning_type,
    rm.actor = r.actor,
    rm.isMultipleResult = r.is_multiple_result,
    rm.updatedAt = datetime()
MERGE (bc)-[:HAS_READMODEL]->(rm)
RETURN rm {.id, .key, .name, .displayName, .description, .provisioningType, .actor, .isMultipleResult} AS result
