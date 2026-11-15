MATCH (e { id: $element_id })-[r:IMPLEMENTED_IN]->()
WHERE any(lbl IN $element_labels WHERE lbl IN labels(e))
DELETE r
