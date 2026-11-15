-- The fabric's source system: a small ERP in plain PostgreSQL tables. Same for every target.
DROP SCHEMA IF EXISTS compat_erp CASCADE;
CREATE SCHEMA compat_erp;
CREATE TABLE compat_erp.supplier (supplier_id text PRIMARY KEY, name text NOT NULL, rating numeric(3,1), city text);
CREATE TABLE compat_erp.purchase_order (po_id text PRIMARY KEY, supplier_id text REFERENCES compat_erp.supplier, amount integer, status text, ordered_on date);
CREATE TABLE compat_erp.item (item_id text PRIMARY KEY, name text, unit text);
CREATE TABLE compat_erp.po_line (po_id text REFERENCES compat_erp.purchase_order, line_no integer, item_id text REFERENCES compat_erp.item, qty integer, PRIMARY KEY (po_id, line_no));
INSERT INTO compat_erp.supplier VALUES ('S01', 'Hanil Steel', 4.5, 'Busan'), ('S02', 'Busan Parts', 3.2, 'Busan'), ('S03', 'Nord Cable', NULL, 'Seoul');
INSERT INTO compat_erp.purchase_order VALUES ('PO-1', 'S01', 1500, 'open', '2026-01-05'), ('PO-2', 'S02', 700, 'closed', '2026-01-07'), ('PO-3', 'S01', 90, 'open', '2026-01-09');
INSERT INTO compat_erp.item VALUES ('I-1', 'cable 10m', 'ea'), ('I-2', 'bolt M8', 'box');
INSERT INTO compat_erp.po_line VALUES ('PO-1', 1, 'I-1', 10), ('PO-1', 2, 'I-2', 3), ('PO-2', 1, 'I-2', 1);
