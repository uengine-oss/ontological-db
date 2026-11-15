MATCH (rm:ReadModel {id: $readmodel_id})-[:HAS_PROPERTY]->(prop:Property)
RETURN prop {.id, .name, .type, .isRequired} as property
ORDER BY prop.name
