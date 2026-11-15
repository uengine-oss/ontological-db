MATCH (g:GLOSSARY {id:"glossary", graph_owner:$graph_owner}) RETURN g.terms_json AS tj
