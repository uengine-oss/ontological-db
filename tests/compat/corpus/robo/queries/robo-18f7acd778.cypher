MATCH (bc:BoundedContext {id: $bid}), (ui:UI {id: $uid}) MERGE (bc)-[r:HAS_UI]->(ui) ON CREATE SET r.createdAt = datetime()
