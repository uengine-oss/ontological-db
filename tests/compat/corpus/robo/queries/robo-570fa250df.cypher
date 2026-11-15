UNWIND $rows AS r
MERGE (bc:BoundedContext {key: r.key})
  ON CREATE SET bc.id = randomUUID(),
                bc.createdAt = datetime()
SET bc.name = r.name,
    bc.key = r.key,
    bc.displayName = r.display_name,
    bc.description = r.description,
    bc.owner = r.owner,
    bc.domainType = r.domain_type,
    bc.userStoryIds = r.user_story_ids,
    bc.updatedAt = datetime()
RETURN bc {.id, .key, .name, .displayName, .description, .owner, .domainType, .userStoryIds} AS result
