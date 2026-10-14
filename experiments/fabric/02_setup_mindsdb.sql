-- Approach B — keep MindsDB as the fabric, and let Ontological point at it.
--
-- Same model as 01_setup.sql, but every virtual type reads through MindsDB's
-- MySQL endpoint (robo-mindsdb:47335), where the four systems are already
-- registered as scm_erp / scm_customs / scm_wms / scm_crm. Ontological does not
-- hold a single source credential in this layout; MindsDB does.
--
-- MindsDB fails IMPORT FOREIGN SCHEMA (its information_schema query breaks on an
-- unrelated database), so the foreign tables are declared with the column list
-- the direct import of 01_setup.sql already discovered.
\set ON_ERROR_STOP 1

SELECT og_drop_graph('scm_mdb') WHERE EXISTS (SELECT 1 FROM og_catalog.graph WHERE name = 'scm_mdb');
SELECT og_create_graph('scm_mdb');
SELECT og_create_datasource('mdb', 'mindsdb',
  '{"host":"robo-mindsdb","port":47335,"user":"mindsdb","password":""}');

DROP SCHEMA IF EXISTS og_mdb CASCADE;
CREATE SCHEMA og_mdb;

DO $$
DECLARE
  m record;
  cols text;
BEGIN
  FOR m IN SELECT * FROM (VALUES
      ('og_ext_erp__erp',      'mst_supplier',       'scm_erp'),
      ('og_ext_erp__erp',      'mst_item',           'scm_erp'),
      ('og_ext_erp__erp',      'doc_purchase_order', 'scm_erp'),
      ('og_ext_erp__erp',      'doc_po_line',        'scm_erp'),
      ('og_ext_erp__customs',  'decl_header',        'scm_customs'),
      ('og_ext_wms__wms',      'shipment',           'scm_wms'),
      ('og_ext_crm__crm_scm',  'customer',           'scm_crm'),
      ('og_ext_crm__crm_scm',  'sales_order',        'scm_crm'),
      ('og_ext_crm__crm_scm',  'claim_ticket',       'scm_crm')) AS v(src_schema, tbl, mdb_db)
  LOOP
    SELECT string_agg(format('%I %s', a.attname, format_type(a.atttypid, a.atttypmod)), ', ' ORDER BY a.attnum)
      INTO cols
      FROM pg_attribute a
     WHERE a.attrelid = format('%I.%I', m.src_schema, m.tbl)::regclass AND a.attnum > 0 AND NOT a.attisdropped;
    EXECUTE format('CREATE FOREIGN TABLE og_mdb.%I (%s) SERVER og_ds_mdb OPTIONS (dbname %L, table_name %L)',
                   m.tbl, cols, m.mdb_db, m.tbl);
  END LOOP;
END $$;

SELECT og_map_relation('scm_mdb', 'Supplier',      'og_mdb.mst_supplier',       'supplier_id');
SELECT og_map_relation('scm_mdb', 'Item',          'og_mdb.mst_item',           'item_id');
SELECT og_map_relation('scm_mdb', 'PurchaseOrder', 'og_mdb.doc_purchase_order', 'po_id');
SELECT og_map_relation('scm_mdb', 'POLine',        'og_mdb.doc_po_line',        'po_line_id');
SELECT og_map_relation('scm_mdb', 'Declaration',   'og_mdb.decl_header',        'decl_id');
SELECT og_map_relation('scm_mdb', 'Shipment',      'og_mdb.shipment',           'shipment_no');
SELECT og_map_relation('scm_mdb', 'Customer',      'og_mdb.customer',           'cust_no');
SELECT og_map_relation('scm_mdb', 'SalesOrder',    'og_mdb.sales_order',        'so_no');
SELECT og_map_relation('scm_mdb', 'Claim',         'og_mdb.claim_ticket',       'ticket_no');

SELECT og_create_virtual_rel('scm_mdb', 'ORDERED_FROM', 'PurchaseOrder', 'supplier_id', 'Supplier', 'supplier_id');
SELECT og_create_virtual_rel('scm_mdb', 'LINE_OF',      'POLine', 'po_id',   'PurchaseOrder', 'po_id');
SELECT og_create_virtual_rel('scm_mdb', 'FOR_ITEM',     'POLine', 'item_id', 'Item', 'item_id');
SELECT og_create_virtual_rel('scm_mdb', 'SHIPS',        'Shipment', 'ref_po_no',   'PurchaseOrder', 'po_id');
SELECT og_create_virtual_rel('scm_mdb', 'SHIPPED_BY',   'Shipment', 'vendor_code', 'Supplier', 'supplier_id');
SELECT og_create_virtual_rel('scm_mdb', 'DECLARES',     'Declaration', 'ref_shipment_no', 'Shipment', 'shipment_no');
SELECT og_create_virtual_rel('scm_mdb', 'PLACED_BY',    'SalesOrder', 'cust_no',  'Customer', 'cust_no');
SELECT og_create_virtual_rel('scm_mdb', 'ORDERS_ITEM',  'SalesOrder', 'sku_code', 'Item', 'item_id');
SELECT og_create_virtual_rel('scm_mdb', 'ABOUT_ORDER',  'Claim', 'so_no', 'SalesOrder', 'so_no');

SELECT og_create_type('scm_mdb', 'Region', 'entity');
SELECT og_create_type('scm_mdb', 'RiskEvent', 'entity');
SELECT og_add_property('scm_mdb', 'Region', 'name', 'string');
SELECT og_add_property('scm_mdb', 'RiskEvent', 'title', 'string');
SELECT og_add_property('scm_mdb', 'RiskEvent', 'severity', 'string');
SELECT og_add_property('scm_mdb', 'RiskEvent', 'supplier_code', 'string');
SELECT og_create_virtual_rel('scm_mdb', 'AFFECTS', 'RiskEvent', 'supplier_code', 'Supplier', 'supplier_id');
SELECT * FROM og_cypher('scm_mdb', $$
  CREATE (r1:Region {name:'East Asia'}), (r2:Region {name:'Southeast Asia'})
  CREATE (:RiskEvent {title:'Port congestion Shanghai', severity:'high',   supplier_code:'SUP-CN-003'})-[:IN_REGION]->(r1)
  CREATE (:RiskEvent {title:'Typhoon Haiphong',         severity:'high',   supplier_code:'SUP-VN-002'})-[:IN_REGION]->(r2)
  CREATE (:RiskEvent {title:'Export licence review',    severity:'medium', supplier_code:'SUP-CN-001'})-[:IN_REGION]->(r1)
$$);
