MATCH (g:GlossaryTerm {session_id: $sid}) RETURN g ORDER BY g.term
