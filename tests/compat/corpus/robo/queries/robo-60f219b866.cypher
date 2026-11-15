MATCH (:Feature)-[r:HAS_USER_STORY]->(us:UserStory {id: $us_id})
DELETE r
