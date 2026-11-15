MERGE (evt:Event {name: $name})
ON CREATE SET evt.id = randomUUID(),
              evt.createdAt = datetime()
SET evt.name = $name,
    evt.key = $key,
    evt.displayName = $display_name,
    evt.description = $description,
    evt.userStoryId = $user_story_id,
    evt.sequence = $sequence,
    evt.version = '1.0.0',
    evt.isBreaking = false,
    evt.updatedAt = datetime()
RETURN evt {.id, .name, .key, .displayName, .description, .userStoryId, .sequence} as event
