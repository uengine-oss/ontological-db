MATCH (bc:BoundedContext)
RETURN bc.id AS id, coalesce(bc.displayName, bc.name) AS name,
       bc.purpose AS purpose, bc.classification AS classification,
       bc.ubiquitousLanguage AS ul, bc.businessDecisions AS bd
ORDER BY name
