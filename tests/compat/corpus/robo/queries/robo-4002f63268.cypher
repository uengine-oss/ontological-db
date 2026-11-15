MATCH (bc:BoundedContext)
WHERE ($session_id IS NULL OR coalesce(bc.session_id, '') = $session_id)
RETURN collect(bc.id) as bc_ids
