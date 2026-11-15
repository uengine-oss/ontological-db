MATCH (r:_OntologyRelationship) RETURN r.name AS name, r.from_class AS f, r.to_class AS t, r.description AS d
