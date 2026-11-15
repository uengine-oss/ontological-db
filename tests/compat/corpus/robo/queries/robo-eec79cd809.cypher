MATCH (us:UserStory {id: $usid})-[sf:SOURCED_FROM]->(r:Rule)
RETURN r.id AS rule_id,
       r.title AS title,
       r.source_function AS source_function,
       r.source_rule_id AS source_rule_id,
       coalesce(r.given, '') AS given,
       coalesce(r.`when`, '') AS when,
       coalesce(r.`then`, '') AS then,
       coalesce(sf.evidence_role, 'primary') AS evidence_role
ORDER BY evidence_role, r.id
