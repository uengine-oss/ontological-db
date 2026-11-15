MATCH (bc:BoundedContext {id: $bcId}), (ui:UI {id: $uiId}) MERGE (bc)-[:HAS_UI]->(ui)
