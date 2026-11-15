MATCH (bc:BoundedContext {id: $node_id})-[:HAS_UI]->(ui:UI)
RETURN ui
