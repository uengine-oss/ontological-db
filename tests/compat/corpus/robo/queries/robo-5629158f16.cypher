MATCH (bc:BoundedContext)-[:HAS_UI]->(ui:UI)
WHERE NOT (ui)-[:ATTACHED_TO]->()
RETURN ui.id AS uiId, ui.name AS uiName, ui.displayName AS uiDisplayName,
       ui.template AS uiTemplate, ui.actor AS uiActor, ui.sequence AS uiStoredSequence,
       bc.id AS bcId
