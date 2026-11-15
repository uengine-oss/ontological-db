MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_UI]->(ui:UI)
RETURN ui
