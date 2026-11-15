MATCH (bc:BoundedContext {id: $bc_id})
MERGE (rm:ReadModel {key: $key})
ON CREATE SET rm.id = $id,
              rm.createdAt = datetime()
SET rm.key = $key,
    rm.name = $name,
    rm.description = $description,
    rm.provisioningType = $provisioning_type,
    rm.actor = $actor,
    rm.isMultipleResult = $is_multiple_result,
    rm.updatedAt = datetime()
WITH bc, rm
MERGE (bc)-[:HAS_READMODEL]->(rm)
