MATCH (rm:ReadModel) WHERE rm.id IN $ids RETURN rm.id AS id, rm.actor AS actor
