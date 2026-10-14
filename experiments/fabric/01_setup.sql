-- Virtual fabric experiment — SCM demo, four systems, zero copies.
--
--   erp, customs  PostgreSQL (robo-postgres / meetingroom)
--   wms, crm_scm  MySQL      (robo-mysql-sample)
--
-- Run:  psql -v pw="$FABRIC_PW" -f 01_setup.sql
\set ON_ERROR_STOP 1

CREATE EXTENSION IF NOT EXISTS ontological CASCADE;
SELECT og_drop_graph('scm') WHERE EXISTS (SELECT 1 FROM og_catalog.graph WHERE name = 'scm');
SELECT og_create_graph('scm');

-- 1. Datasources — MindsDB's CREATE DATABASE ... WITH ENGINE.
SELECT og_create_datasource('erp', 'postgres',
  jsonb_build_object('host','robo-postgres','port',5432,'database','meetingroom','user','og_fabric','password',:'pw'));
SELECT og_create_datasource('wms', 'mysql',
  jsonb_build_object('host','robo-mysql-sample','port',3306,'user','og_fabric','password',:'pw'));
-- The same MySQL server again, under another name: CRM is a separate system
-- that happens to share a host. The fabric must not care.
SELECT og_create_datasource('crm', 'mysql',
  jsonb_build_object('host','robo-mysql-sample','port',3306,'user','og_fabric','password',:'pw'));

-- 2. Virtual types — columns and types are read from the source.
SELECT og_map_virtual('scm', 'Supplier',      'erp', 'erp.mst_supplier',       'supplier_id');
SELECT og_map_virtual('scm', 'Item',          'erp', 'erp.mst_item',           'item_id');
SELECT og_map_virtual('scm', 'PurchaseOrder', 'erp', 'erp.doc_purchase_order', 'po_id');
SELECT og_map_virtual('scm', 'POLine',        'erp', 'erp.doc_po_line',        'po_line_id');
SELECT og_map_virtual('scm', 'Declaration',   'erp', 'customs.decl_header',    'decl_id');
SELECT og_map_virtual('scm', 'Shipment',      'wms', 'wms.shipment',           'shipment_no');
SELECT og_map_virtual('scm', 'Customer',      'crm', 'crm_scm.customer',       'cust_no');
SELECT og_map_virtual('scm', 'SalesOrder',    'crm', 'crm_scm.sales_order',    'so_no');
SELECT og_map_virtual('scm', 'Claim',         'crm', 'crm_scm.claim_ticket',   'ticket_no');

-- 3. Virtual relationships — the join, declared once.
SELECT og_create_virtual_rel('scm', 'ORDERED_FROM', 'PurchaseOrder', 'supplier_id', 'Supplier', 'supplier_id');
SELECT og_create_virtual_rel('scm', 'LINE_OF',      'POLine', 'po_id',   'PurchaseOrder', 'po_id');
SELECT og_create_virtual_rel('scm', 'FOR_ITEM',     'POLine', 'item_id', 'Item', 'item_id');
SELECT og_create_virtual_rel('scm', 'SHIPS',        'Shipment', 'ref_po_no',   'PurchaseOrder', 'po_id');        -- MySQL → PG
SELECT og_create_virtual_rel('scm', 'SHIPPED_BY',   'Shipment', 'vendor_code', 'Supplier', 'supplier_id');       -- MySQL → PG
SELECT og_create_virtual_rel('scm', 'DECLARES',     'Declaration', 'ref_shipment_no', 'Shipment', 'shipment_no'); -- PG → MySQL
SELECT og_create_virtual_rel('scm', 'PLACED_BY',    'SalesOrder', 'cust_no',  'Customer', 'cust_no');
SELECT og_create_virtual_rel('scm', 'ORDERS_ITEM',  'SalesOrder', 'sku_code', 'Item', 'item_id');               -- MySQL → PG
SELECT og_create_virtual_rel('scm', 'ABOUT_ORDER',  'Claim', 'so_no', 'SalesOrder', 'so_no');

-- 3b. A virtual view: stock_daily has no single key (date · sku · warehouse),
--     so a view gives it one, and the view — not the table — becomes the type.
--     This is MindsDB's CREATE VIEW, except that filters still reach MySQL.
CREATE SCHEMA IF NOT EXISTS og_views;
SELECT og_import_foreign('wms', 'wms.stock_daily');
CREATE OR REPLACE VIEW og_views.stock_daily AS
  SELECT snapshot_date::text || '|' || sku_code || '|' || warehouse_cd AS stock_key, *
    FROM og_ext_wms__wms.stock_daily;
SELECT og_map_relation('scm', 'StockDaily', 'og_views.stock_daily', 'stock_key');
SELECT og_create_virtual_rel('scm', 'STOCK_OF', 'StockDaily', 'sku_code', 'Item', 'item_id');           -- MySQL → PG

-- 4. Native, materialized knowledge that lives only in the graph — and points
--    at virtual rows by key.
SELECT og_create_type('scm', 'Region', 'entity');
SELECT og_create_type('scm', 'RiskEvent', 'entity');
SELECT og_add_property('scm', 'Region', 'name', 'string');
SELECT og_add_property('scm', 'RiskEvent', 'title', 'string');
SELECT og_add_property('scm', 'RiskEvent', 'severity', 'string');
SELECT og_add_property('scm', 'RiskEvent', 'supplier_code', 'string');
SELECT og_create_virtual_rel('scm', 'AFFECTS', 'RiskEvent', 'supplier_code', 'Supplier', 'supplier_id');  -- native → virtual

SELECT * FROM og_cypher('scm', $$
  CREATE (r1:Region {name:'East Asia'}), (r2:Region {name:'Southeast Asia'})
  CREATE (:RiskEvent {title:'Port congestion Shanghai', severity:'high',   supplier_code:'SUP-CN-003'})-[:IN_REGION]->(r1)
  CREATE (:RiskEvent {title:'Typhoon Haiphong',         severity:'high',   supplier_code:'SUP-VN-002'})-[:IN_REGION]->(r2)
  CREATE (:RiskEvent {title:'Export licence review',    severity:'medium', supplier_code:'SUP-CN-001'})-[:IN_REGION]->(r1)
$$);

SELECT jsonb_pretty(og_fabric_catalog('scm'));
