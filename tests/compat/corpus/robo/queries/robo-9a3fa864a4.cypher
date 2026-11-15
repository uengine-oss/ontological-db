MATCH (bc:BoundedContext {id: $id})
RETURN bc.id AS id, coalesce(bc.displayName, bc.name) AS name,
       bc.purpose AS purpose, bc.classification AS classification,
       bc.domainRoles AS domainRoles, bc.ubiquitousLanguage AS ubiquitousLanguage,
       bc.businessDecisions AS businessDecisions, bc.assumptions AS assumptions,
       coalesce(bc.version, 0) AS version
