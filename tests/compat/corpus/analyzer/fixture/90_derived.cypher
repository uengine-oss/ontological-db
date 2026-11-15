// graph: analyzer_run
// Derived from the labels, properties and relationship patterns the corpus queries name
// (tests/compat/extract/prepare.py). Graph: analyzer_run. Do not edit — re-run extract.py.

CREATE (:ANALYZER_OWNED:COLUMN {_owner: 'analyzer', analyzed_description: 'COLUMN analyzed_description 1', comment: 'COLUMN comment 1', data_type: 'COLUMN data_type 1', default_value: 'COLUMN default_value 1', description: '', description_source: 'COLUMN description_source 1', dtype: 'COLUMN dtype 1', graph_owner: 'analyzer', id: 'COLUMN-1', is_primary_key: true, is_unique: true, logical_name: 'COLUMN logical_name 1', name: 'COLUMN 1', nullable: true, primary_key: 'COLUMN primary_key 1', summary: 'COLUMN summary 1', unique: 'COLUMN unique 1'});
CREATE (:ANALYZER_OWNED:COLUMN {_owner: 'analyzer', analyzed_description: 'COLUMN analyzed_description 2', comment: 'COLUMN comment 2', data_type: 'COLUMN data_type 2', default_value: 'COLUMN default_value 2', description: 'N/A', description_source: 'COLUMN description_source 2', dtype: 'COLUMN dtype 2', graph_owner: 'analyzer', id: 'COLUMN-2', is_primary_key: false, is_unique: false, logical_name: 'COLUMN logical_name 2', name: 'COLUMN 2', nullable: false, primary_key: 'COLUMN primary_key 2', summary: 'COLUMN summary 2', unique: 'COLUMN unique 2'});
CREATE (:ANALYZER_OWNED:EMBED_META {_owner: 'analyzer', dim: 'EMBED_META dim 1', embedded_count: 10, fingerprint: 'EMBED_META fingerprint 1', graph_owner: 'analyzer', id: 'EMBED_META-1', name: 'EMBED_META 1'});
CREATE (:ANALYZER_OWNED:EMBED_META {_owner: 'analyzer', dim: 'EMBED_META dim 2', embedded_count: 20, fingerprint: 'EMBED_META fingerprint 2', graph_owner: 'analyzer', id: 'EMBED_META-2', name: 'EMBED_META 2'});
CREATE (:ANALYZER_OWNED:EXAMPLE {_owner: 'analyzer', given: 'EXAMPLE given 1', graph_owner: 'analyzer', id: 'EXAMPLE-1', name: 'EXAMPLE 1', then_: 'EXAMPLE then_ 1', when_: 'EXAMPLE when_ 1'});
CREATE (:ANALYZER_OWNED:EXAMPLE {_owner: 'analyzer', given: 'EXAMPLE given 2', graph_owner: 'analyzer', id: 'EXAMPLE-2', name: 'EXAMPLE 2', then_: 'EXAMPLE then_ 2', when_: 'EXAMPLE when_ 2'});
CREATE (:ANALYZER_OWNED:FUNCTION {_owner: 'analyzer', external: true, graph_owner: 'analyzer', id: 'FUNCTION-1', name: 'FUNCTION 1', type: 'FUNCTION type 1'});
CREATE (:ANALYZER_OWNED:FUNCTION {_owner: 'analyzer', external: true, graph_owner: 'analyzer', id: 'FUNCTION-2', name: 'FUNCTION 2', type: 'FUNCTION type 2'});
CREATE (:ANALYZER_OWNED:GLOSSARY {_owner: 'analyzer', graph_owner: 'analyzer', id: 'glossary', n_terms: 'GLOSSARY n_terms 1', name: 'GLOSSARY 1', terms_json: 'GLOSSARY terms_json 1'});
CREATE (:ANALYZER_OWNED:GLOSSARY {_owner: 'analyzer', graph_owner: 'analyzer', id: 'glossary', n_terms: 'GLOSSARY n_terms 2', name: 'GLOSSARY 2', terms_json: 'GLOSSARY terms_json 2'});
CREATE (:ANALYZER_OWNED:MODULE {_owner: 'analyzer', end_line: 10, external: true, file_path: 'MODULE file_path 1', graph_owner: 'analyzer', id: 'MODULE-1', name: 'MODULE 1', start_line: 10, summary: 'MODULE summary 1', type: 'MODULE type 1'});
CREATE (:ANALYZER_OWNED:MODULE {_owner: 'analyzer', end_line: 20, external: true, file_path: 'MODULE file_path 2', graph_owner: 'analyzer', id: 'MODULE-2', name: 'MODULE 2', start_line: 20, summary: 'MODULE summary 2', type: 'MODULE type 2'});
CREATE (:ANALYZER_OWNED:PACKAGE {_owner: 'analyzer', graph_owner: 'analyzer', id: 'PACKAGE-1', name: 'PACKAGE 1'});
CREATE (:ANALYZER_OWNED:PACKAGE {_owner: 'analyzer', graph_owner: 'analyzer', id: 'PACKAGE-2', name: 'PACKAGE 2'});
CREATE (:ANALYZER_OWNED:QUESTION {_owner: 'analyzer', graph_owner: 'analyzer', id: 'QUESTION-1', name: 'QUESTION 1', reason: 'QUESTION reason 1', text: 'QUESTION text 1'});
CREATE (:ANALYZER_OWNED:QUESTION {_owner: 'analyzer', graph_owner: 'analyzer', id: 'QUESTION-2', name: 'QUESTION 2', reason: 'QUESTION reason 2', text: 'QUESTION text 2'});
CREATE (:ANALYZER_OWNED:RULE {_owner: 'analyzer', embedding: [0.7, 1.0, 0.2, 0.5, 0.8, 0.0, 0.3, 0.6], graph_owner: 'analyzer', id: 'RULE-1', name: 'RULE 1', statement: 'RULE statement 1'});
CREATE (:ANALYZER_OWNED:RULE {_owner: 'analyzer', embedding: [0.3, 0.6, 0.9, 0.1, 0.4, 0.7, 1.0, 0.2], graph_owner: 'analyzer', id: 'RULE-2', name: 'RULE 2', statement: 'RULE statement 2'});
CREATE (:ANALYZER_OWNED:SCHEMA {_owner: 'analyzer', db: 'SCHEMA db 1', graph_owner: 'analyzer', id: 'SCHEMA-1', name: 'SCHEMA 1'});
CREATE (:ANALYZER_OWNED:SCHEMA {_owner: 'analyzer', db: 'SCHEMA db 2', graph_owner: 'analyzer', id: 'SCHEMA-2', name: 'SCHEMA 2'});
CREATE (:ANALYZER_OWNED:TABLE {_owner: 'analyzer', analyzed_description: 'TABLE analyzed_description 1', comment: 'TABLE comment 1', datasource: 'TABLE datasource 1', db: 'TABLE db 1', description: '', description_source: 'TABLE description_source 1', graph_owner: 'analyzer', id: 'TABLE-1', logical_name: 'TABLE logical_name 1', name: 'TABLE 1', schema: 'TABLE schema 1', schema_name: 'TABLE schema_name 1', summary: 'TABLE summary 1', table_type: 'TABLE table_type 1'});
CREATE (:ANALYZER_OWNED:TABLE {_owner: 'analyzer', analyzed_description: 'TABLE analyzed_description 2', comment: 'TABLE comment 2', datasource: 'TABLE datasource 2', db: 'TABLE db 2', description: 'N/A', description_source: 'TABLE description_source 2', graph_owner: 'analyzer', id: 'TABLE-2', logical_name: 'TABLE logical_name 2', name: 'TABLE 2', schema: 'TABLE schema 2', schema_name: 'TABLE schema_name 2', summary: 'TABLE summary 2', table_type: 'TABLE table_type 2'});

MATCH (a:EXAMPLE {id: 'EXAMPLE-1'}), (b:TABLE {id: 'TABLE-1'}) CREATE (a)-[:AFFECTS_TABLE]->(b);
MATCH (a:EXAMPLE {id: 'EXAMPLE-2'}), (b:TABLE {id: 'TABLE-2'}) CREATE (a)-[:AFFECTS_TABLE]->(b);
MATCH (a:MODULE {id: 'MODULE-1'}), (b:FUNCTION {id: 'FUNCTION-1'}) CREATE (a)-[:HAS_MEMBER]->(b);
MATCH (a:MODULE {id: 'MODULE-2'}), (b:FUNCTION {id: 'FUNCTION-2'}) CREATE (a)-[:HAS_MEMBER]->(b);
MATCH (a:RULE {id: 'RULE-1'}), (b:EXAMPLE {id: 'EXAMPLE-1'}) CREATE (a)-[:HAS_EXAMPLE]->(b);
MATCH (a:RULE {id: 'RULE-2'}), (b:EXAMPLE {id: 'EXAMPLE-2'}) CREATE (a)-[:HAS_EXAMPLE]->(b);
MATCH (a:TABLE {id: 'TABLE-1'}), (b:TABLE {id: 'TABLE-1'}) CREATE (a)-[:DATA_FLOWS_TO]->(b);
MATCH (a:TABLE {id: 'TABLE-2'}), (b:TABLE {id: 'TABLE-2'}) CREATE (a)-[:DATA_FLOWS_TO]->(b);
MATCH (a:TABLE {id: 'TABLE-1'}), (b:TABLE {id: 'TABLE-1'}) CREATE (a)-[:FK]->(b);
MATCH (a:TABLE {id: 'TABLE-2'}), (b:TABLE {id: 'TABLE-2'}) CREATE (a)-[:FK]->(b);
MATCH (a:TABLE {id: 'TABLE-1'}), (b:TABLE {id: 'TABLE-1'}) CREATE (a)-[:FK_CANDIDATE]->(b);
MATCH (a:TABLE {id: 'TABLE-2'}), (b:TABLE {id: 'TABLE-2'}) CREATE (a)-[:FK_CANDIDATE]->(b);
MATCH (a:TABLE {id: 'TABLE-1'}), (b:COLUMN {id: 'COLUMN-1'}) CREATE (a)-[:HAS_COLUMN]->(b);
MATCH (a:TABLE {id: 'TABLE-2'}), (b:COLUMN {id: 'COLUMN-2'}) CREATE (a)-[:HAS_COLUMN]->(b);
