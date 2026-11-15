// graph: ontology
// Studio's graph as its queries expect it: instances under the abstract _Entity root (made by the
// bootstrap, as Studio's installer does), the publish registry (_Ontology*), indexed documents, and
// virtual nodes over the erp datasource. Made up; no customer data.

// ---- instances (classes Customer / Supplier / PurchaseOrder under _Entity)
CREATE (:_Entity:Customer {_source_id: 'cust-1', customer_id: 'C001', name: 'Daesung Cable', title: 'Daesung Cable', content: 'cable maker, buys steel wire', grade: 'A', email: 'buy@daesung.example', credit_limit: 5000, _source_system: 'erp', number: 1, embedding: [0.9, 0.1, 0.0, 0.2, 0.1, 0.0, 0.3, 0.1]});
CREATE (:_Entity:Customer {_source_id: 'cust-2', customer_id: 'C002', name: 'Acme', title: 'Acme', content: 'Acme corp, spare parts', grade: 'B', email: 'ops@acme.example', credit_limit: 1200, _source_system: 'crm', number: 2, embedding: [0.1, 0.8, 0.2, 0.0, 0.0, 0.1, 0.1, 0.4]});
CREATE (:_Entity:Customer {_source_id: 'cust-3', customer_id: 'C003', name: 'Acme Korea', title: 'Acme Korea', content: 'Acme branch', email: 'kr@acme.example', credit_limit: 300, _source_system: 'erp', number: 3});
CREATE (:_Entity:Supplier {_source_id: 'sup-1', supplier_id: 'S01', name: 'Hanil Steel', title: 'Hanil Steel', rating: 4.5, number: 11, embedding: [0.7, 0.2, 0.1, 0.1, 0.3, 0.0, 0.2, 0.0]});
CREATE (:_Entity:Supplier {_source_id: 'sup-2', supplier_id: 'S02', name: 'Busan Parts', title: 'Busan Parts', rating: 3.2, number: 12});
CREATE (:_Entity:PurchaseOrder {_source_id: 'po-1', po_id: 'PO-1', name: 'PO-1', amount: 1500, status: 'open', number: 21});
CREATE (:_Entity:PurchaseOrder {_source_id: 'po-2', po_id: 'PO-2', name: 'PO-2', amount: 700, status: 'closed', number: 22});
CREATE (:_Entity:PurchaseOrder {_source_id: 'po-3', po_id: 'PO-3', name: 'PO-3', amount: 90, status: 'open', number: 23});

MATCH (c:Customer {_source_id: 'cust-1'}), (p:PurchaseOrder {_source_id: 'po-1'}) CREATE (c)-[:PLACED {channel: 'web', _confidence: 0.9}]->(p);
MATCH (c:Customer {_source_id: 'cust-1'}), (p:PurchaseOrder {_source_id: 'po-3'}) CREATE (c)-[:PLACED {channel: 'phone'}]->(p);
MATCH (c:Customer {_source_id: 'cust-2'}), (p:PurchaseOrder {_source_id: 'po-2'}) CREATE (c)-[:PLACED {channel: 'web', _confidence: 0.6}]->(p);
MATCH (p:PurchaseOrder {_source_id: 'po-1'}), (s:Supplier {_source_id: 'sup-1'}) CREATE (p)-[:ORDERED_FROM]->(s);
MATCH (p:PurchaseOrder {_source_id: 'po-2'}), (s:Supplier {_source_id: 'sup-2'}) CREATE (p)-[:ORDERED_FROM]->(s);
MATCH (p:PurchaseOrder {_source_id: 'po-3'}), (s:Supplier {_source_id: 'sup-1'}) CREATE (p)-[:ORDERED_FROM]->(s);
MATCH (a:Customer {_source_id: 'cust-2'}), (b:Customer {_source_id: 'cust-3'}) CREATE (a)-[:SAME_AS {method: 'name', _confidence: 0.8}]->(b);
MATCH (a:Supplier {_source_id: 'sup-1'}), (b:PurchaseOrder {_source_id: 'po-1'}) CREATE (a)-[:CONTAINS]->(b);

CREATE VECTOR INDEX entity_embedding_vector IF NOT EXISTS FOR (n:_Entity) ON (n.embedding) OPTIONS {indexConfig: {`vector.dimensions`: 8, `vector.similarity_function`: 'cosine'}};

// ---- publish registry
CREATE (:_OntologyModel {schema_id: 'schema-1', name: 'compat', description: 'compat corpus model', intent: 'orders', golden_questions: '[]', golden_cypher: '[]'});
CREATE (:_OntologyModel {schema_id: 'schema-2', name: 'other', description: 'another model'});
MATCH (m:_OntologyModel {schema_id: 'schema-1'})
CREATE (m)-[:HAS_CLASS]->(:_OntologyClass {class_name: 'Customer', description: 'a buyer', binding_kind: 'materialized', properties: '[{"name": "name", "type": "string"}, {"name": "grade", "type": "string"}, {"name": "email", "type": "string", "identifier": true}]', synonyms: ['client'], certified: true}),
       (m)-[:HAS_CLASS]->(:_OntologyClass {class_name: 'PurchaseOrder', description: 'an order', binding_kind: 'materialized', properties: '[{"name": "amount", "type": "int"}]'}),
       (m)-[:HAS_CLASS]->(:_OntologyClass {class_name: 'Supplier', description: 'a vendor', binding_kind: 'virtual', properties: '[{"name": "name", "type": "string"}]'});
MATCH (m:_OntologyModel {schema_id: 'schema-2'}) CREATE (m)-[:HAS_CLASS]->(:_OntologyClass {class_name: 'Item', description: 'stock item', binding_kind: 'virtual', properties: '[]'});
MATCH (c:_OntologyClass {class_name: 'Customer'})
CREATE (c)-[:HAS_BINDING]->(:_OntologyBinding {class_name: 'Customer', datasource: 'erp', source_schema: 'compat_erp', table_name: 'customer', extra_tables: '[]', key_columns: '["customer_id"]', column_map: '[{"column": "customer_id", "property": "customer_id"}]', base_conditions: '[]', label_property: 'name', default_limit: 100, virtual_node: ''});
MATCH (c:_OntologyClass {class_name: 'Supplier'})
CREATE (c)-[:HAS_BINDING]->(:_OntologyBinding {class_name: 'Supplier', datasource: 'erp', source_schema: 'compat_erp', table_name: 'supplier', extra_tables: '[]', key_columns: '["supplier_id"]', column_map: '[{"column": "supplier_id", "property": "supplier_id"}, {"column": "name", "property": "name"}]', base_conditions: '[]', label_property: 'name', default_limit: 100, virtual_node: 'ErpSupplier'});
MATCH (c:_OntologyClass {class_name: 'Customer'})
CREATE (c)-[:HAS_BEHAVIOR]->(:_OntologyBehavior {behavior_name: 'top_customers', behavior_type: 'cypher', class_name: 'Customer', description: 'customers by credit', parameters: '[]', cypher_template: 'MATCH (n:_Entity:Customer) RETURN n.name ORDER BY n.credit_limit DESC LIMIT $limit', output_columns: '["name"]', result_kind: 'table', max_rows: 50, golden_question: 'who buys most?'});
MATCH (c:_OntologyClass {class_name: 'Customer'})
CREATE (c)-[:HAS_ACTION]->(:_OntologyBehavior {behavior_name: 'set_grade', behavior_type: 'action', class_name: 'Customer', description: 'change a grade', parameters: '[{"name": "grade"}]'});
MATCH (:_OntologyClass {class_name: 'Supplier'})-[:HAS_BINDING]->(b:_OntologyBinding)
CREATE (b)-[:HAS_BEHAVIOR]->(:_OntologyBehavior {behavior_name: 'supplier_rating', behavior_type: 'sql', class_name: 'Supplier', description: 'rating', parameters: '[]', sql_template: 'SELECT name, rating FROM erp.supplier', output_columns: '["name", "rating"]', result_kind: 'table', max_rows: 20});
MATCH (m:_OntologyModel {schema_id: 'schema-1'})
CREATE (m)-[:HAS_RELATIONSHIP]->(:_OntologyRelationship {name: 'PLACED', from_class: 'Customer', to_class: 'PurchaseOrder', description: 'who ordered', properties: '[]'}),
       (m)-[:HAS_RELATIONSHIP]->(:_OntologyRelationship {name: 'ORDERED_FROM', from_class: 'PurchaseOrder', to_class: 'Supplier', description: 'vendor', properties: '[]'});

// ---- indexed documents
CREATE CONSTRAINT document_id_unique IF NOT EXISTS FOR (d:Document) REQUIRE d.document_id IS UNIQUE;
CREATE CONSTRAINT chunk_id_unique IF NOT EXISTS FOR (c:Chunk) REQUIRE c.chunk_id IS UNIQUE;
CREATE FULLTEXT INDEX chunk_source_text_fulltext IF NOT EXISTS FOR (c:Chunk) ON EACH [c.source_text];
CREATE VECTOR INDEX chunk_embedding_vector IF NOT EXISTS FOR (c:Chunk) ON (c.embedding) OPTIONS {indexConfig: {`vector.dimensions`: 8, `vector.similarity_function`: 'cosine'}};
CREATE (d:Document {document_id: 'doc-1', name: 'supply contract', file_name: 'contract.pdf', source_path: '/docs/contract.pdf'})
CREATE (d)-[:HAS_CHUNK]->(c1:Chunk {chunk_id: 'doc-1#1', document_id: 'doc-1', chunk_ref: 'p1', source_page: 1, source_text: 'Customer Daesung Cable orders steel wire from Hanil Steel.', embedding: [0.8, 0.1, 0.1, 0.2, 0.1, 0.0, 0.3, 0.1]})
CREATE (d)-[:HAS_CHUNK]->(c2:Chunk {chunk_id: 'doc-1#2', document_id: 'doc-1', chunk_ref: 'p2', source_page: 2, source_text: 'Payment is due in 30 days. Customer Acme pays by card.', embedding: [0.1, 0.7, 0.2, 0.1, 0.0, 0.1, 0.1, 0.3]})
CREATE (c1)-[:NEXT_CHUNK]->(c2);

// ---- virtual nodes over the erp datasource
CREATE VIRTUAL NODE ErpSupplier IN ontology FROM erp.supplier KEY supplier_id;
CREATE VIRTUAL NODE ErpPurchaseOrder IN ontology FROM erp.purchase_order KEY po_id;
CREATE VIRTUAL RELATIONSHIP SUPPLIED_BY IN ontology FROM ErpPurchaseOrder(supplier_id) TO ErpSupplier(supplier_id);
