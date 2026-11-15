MERGE (n:UI {id: $target_id})
SET n.name = $name,
    n.description = $description,
    n.template = $template,
    n.attachedToId = $attached_to_id,
    n.attachedToType = $attached_to_type,
    n.attachedToName = $attached_to_name,
    n.createdAt = datetime()
WITH n
MATCH (bc:BoundedContext {id: $bc_id})
MERGE (bc)-[:HAS_UI]->(n)
RETURN n.id as id
