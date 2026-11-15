MERGE (bc:BoundedContext {key: $key})
ON CREATE SET bc.id = randomUUID(),
              bc.createdAt = datetime()
SET bc.name = $name,
    bc.key = $key,
    bc.displayName = $display_name,
    bc.description = $description,
    bc.owner = $owner,
    bc.domainType = $domain_type,
    bc.userStoryIds = $user_story_ids,
    bc.updatedAt = datetime()
RETURN bc {.id, .key, .name, .displayName, .description, .owner, .domainType, .userStoryIds} as bounded_context
