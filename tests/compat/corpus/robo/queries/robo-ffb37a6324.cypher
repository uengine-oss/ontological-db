MATCH (us:UserStory)-[:IMPLEMENTS]->(bc:BoundedContext)
MATCH (us)-[:HAS_EVENT]->(evt:Event)
MERGE (bc)-[:HAS_EVENT]->(evt)
