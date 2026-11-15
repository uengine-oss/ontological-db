MATCH (bc:BoundedContext {id: $bc_id})
MERGE (rm:ReadModel {key: $key})
ON CREATE SET rm.id = randomUUID(),
              rm.createdAt = datetime()
SET rm.key = $key,
    rm.name = $name,
    rm.displayName = $display_name,
    rm.description = $description,
    rm.provisioningType = $provisioning_type,
    rm.actor = $actor,
    rm.isMultipleResult = $is_multiple_result,
    rm.updatedAt = datetime()
MERGE (bc)-[:HAS_READMODEL]->(rm)
RETURN rm {.id, .key, .name, .displayName, .description, .provisioningType, .actor, .isMultipleResult} as readmodel
