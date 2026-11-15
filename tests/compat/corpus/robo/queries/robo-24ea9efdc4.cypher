MATCH (evt:Event {id: $evt_id})
MATCH (us:UserStory {id: $us_id})
MERGE (us)-[:HAS_EVENT]->(evt)
