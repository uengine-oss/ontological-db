MERGE (g:GlossaryTerm {term: $term, session_id: $sid}) SET g.aliases = $aliases, g.code_candidates = $candidates, g.source = $source
