MATCH (t:BpmTask {session_id: $sid})-[pt:PROMOTED_TO]->(us:UserStory {session_id: $sid})
WITH count(pt) AS promoted_to
MATCH (us:UserStory {session_id: $sid})-[sf:SOURCED_FROM]->(:Rule {session_id: $sid})
WITH promoted_to, count(sf) AS sourced_from
MATCH (us2:UserStory {session_id: $sid})-[imp:IMPLEMENTS]->(:BoundedContext {session_id: $sid})
RETURN promoted_to, sourced_from, count(imp) AS implements_bc
