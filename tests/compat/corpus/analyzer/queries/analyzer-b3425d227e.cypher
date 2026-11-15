UNWIND $items AS item
MERGE (g:ANALYZER_OWNED:GLOSSARY {id: "glossary", graph_owner: 'analyzer'})
SET g.terms_json = item.terms_json,
    g.n_terms = item.n_terms
