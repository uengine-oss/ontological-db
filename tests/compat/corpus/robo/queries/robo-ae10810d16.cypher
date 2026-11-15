MATCH (ui:UI {id: $id})
OPTIONAL MATCH (ui)-[:ATTACHED_TO]->(target)
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_UI]->(ui)
RETURN ui.name as name, ui.displayName as displayName,
       ui.description as description,
       ui.attachedToType as attachedToType,
       ui.attachedToName as attachedToName,
       labels(target)[0] as targetLabel,
       target.name as targetName,
       bc.name as bcName
