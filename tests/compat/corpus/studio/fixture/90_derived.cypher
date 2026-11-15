// graph: ontology
// Derived from the labels, properties and relationship patterns the corpus queries name
// (tests/compat/extract/prepare.py). Graph: ontology. Do not edit — re-run extract.py.

CREATE (:Column {description: 'Column description 1', dtype: 'Column dtype 1', id: 'Column-1', is_primary_key: true, name: 'Column 1', nullable: true, ordinal_position: 10, primary_key: 'Column primary_key 1', type: 'Column type 1'});
CREATE (:Column {description: 'Column description 2', dtype: 'Column dtype 2', id: 'Column-2', is_primary_key: false, name: 'Column 2', nullable: false, ordinal_position: 20, primary_key: 'Column primary_key 2', type: 'Column type 2'});
CREATE (:_Entity:MfgProcessStep {id: 'MfgProcessStep-1', name: 'MfgProcessStep 1', step_id: 'step_id-1', step_name: 'MfgProcessStep step_name 1'});
CREATE (:_Entity:MfgProcessStep {id: 'MfgProcessStep-2', name: 'MfgProcessStep 2', step_id: 'step_id-2', step_name: 'MfgProcessStep step_name 2'});
CREATE (:_Entity:MfgProduct {id: 'MfgProduct-1', name: 'MfgProduct 1', product_id: 'product_id-1'});
CREATE (:_Entity:MfgProduct {id: 'MfgProduct-2', name: 'MfgProduct 2', product_id: 'product_id-2'});
CREATE (:_Entity:MfgProductRouting {id: 'MfgProductRouting-1', name: 'MfgProductRouting 1'});
CREATE (:_Entity:MfgProductRouting {id: 'MfgProductRouting-2', name: 'MfgProductRouting 2'});
CREATE (:Table {datasource: 'Table datasource 1', db: 'Table db 1', description: 'Table description 1', id: 'Table-1', name: 'Table 1', schema: 'Table schema 1'});
CREATE (:Table {datasource: 'Table datasource 2', db: 'Table db 2', description: 'Table description 2', id: 'Table-2', name: 'Table 2', schema: 'Table schema 2'});

MATCH (a:Column {id: 'Column-1'}), (b:Column {id: 'Column-1'}) CREATE (a)-[:REFERENCES]->(b);
MATCH (a:Column {id: 'Column-2'}), (b:Column {id: 'Column-2'}) CREATE (a)-[:REFERENCES]->(b);
MATCH (a:MfgProduct {id: 'MfgProduct-1'}), (b:MfgProductRouting {id: 'MfgProductRouting-1'}) CREATE (a)-[:REFERENCES_MFG_PRODUCT]->(b);
MATCH (a:MfgProduct {id: 'MfgProduct-2'}), (b:MfgProductRouting {id: 'MfgProductRouting-2'}) CREATE (a)-[:REFERENCES_MFG_PRODUCT]->(b);
MATCH (a:MfgProductRouting {id: 'MfgProductRouting-1'}), (b:MfgProcessStep {id: 'MfgProcessStep-1'}) CREATE (a)-[:REFERENCES_MFG_PROCESS_STEP]->(b);
MATCH (a:MfgProductRouting {id: 'MfgProductRouting-2'}), (b:MfgProcessStep {id: 'MfgProcessStep-2'}) CREATE (a)-[:REFERENCES_MFG_PROCESS_STEP]->(b);
MATCH (a:Table {id: 'Table-1'}), (b:Table {id: 'Table-1'}) CREATE (a)-[:FK_TO_TABLE]->(b);
MATCH (a:Table {id: 'Table-2'}), (b:Table {id: 'Table-2'}) CREATE (a)-[:FK_TO_TABLE]->(b);
MATCH (a:Table {id: 'Table-1'}), (b:Column {id: 'Column-1'}) CREATE (a)-[:HAS_COLUMN]->(b);
MATCH (a:Table {id: 'Table-2'}), (b:Column {id: 'Column-2'}) CREATE (a)-[:HAS_COLUMN]->(b);
