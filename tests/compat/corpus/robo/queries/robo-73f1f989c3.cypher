MATCH (ui:UI {id: $id})
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_UI]->(ui)
RETURN ui.name AS name, ui.displayName AS displayName,
       ui.description AS description,
       ui.attachedToType AS attachedToType,
       ui.attachedToName AS attachedToName,
       ui.template AS template,
       bc.name AS bcName
