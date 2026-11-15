MATCH (bc:BoundedContext)-[:HAS_UI]->(ui:UI)
RETURN ui.id AS id,
       coalesce(ui.displayName, ui.name) AS displayName,
       bc.id AS bc_id,
       bc.name AS bc_name,
       bc.key AS bc_key
ORDER BY bc.name, ui.displayName
