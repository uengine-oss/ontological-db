MATCH (bc:BoundedContext {id: $context_id})
RETURN bc {.id, .name, .displayName, .description, .owner, .domainType, .classification, .userStoryIds} as bc
