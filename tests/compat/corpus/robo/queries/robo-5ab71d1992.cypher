MATCH (us:UserStory {id: $id})
SET us.clarifications = $payload
