-- ERP (PostgreSQL): 구매·조달. 키 이름은 supplier_id / po_id / item_id.
CREATE SCHEMA erp;

CREATE TABLE erp.supplier (
    supplier_id text PRIMARY KEY,
    name        text NOT NULL,
    country     text NOT NULL,
    tier        int  NOT NULL
);
INSERT INTO erp.supplier VALUES
  ('S-KR-01', '한빛정밀',              'Korea',   1),
  ('S-KR-02', '대성케이블',            'Korea',   2),
  ('S-CN-01', 'Shenzhen Goldline PCB', 'China',   1),
  ('S-CN-02', 'Ningbo Magnetics',      'China',   2),
  ('S-VN-01', 'Hanoi Assembly',        'Vietnam', 2);

CREATE TABLE erp.item (
    item_id      text PRIMARY KEY,
    name         text NOT NULL,
    safety_stock int  NOT NULL
);
INSERT INTO erp.item VALUES
  ('I-PCB', 'PCB 기판',        500),
  ('I-CAB', '전원 케이블',    1000),
  ('I-MAG', '마그넷 코어',     300),
  ('I-FG',  '완제품 컨트롤러',  50);

CREATE TABLE erp.purchase_order (
    po_id       text PRIMARY KEY,
    supplier_id text NOT NULL REFERENCES erp.supplier,
    order_date  date NOT NULL,
    amount_usd  numeric(12,2) NOT NULL,
    status      text NOT NULL
);
INSERT INTO erp.purchase_order VALUES
  ('PO-001', 'S-CN-01', '2026-08-03', 82000, 'OPEN'),
  ('PO-002', 'S-CN-01', '2026-08-17', 41000, 'OPEN'),
  ('PO-003', 'S-KR-01', '2026-08-05', 15000, 'CLOSED'),
  ('PO-004', 'S-KR-02', '2026-08-09',  9000, 'CLOSED'),
  ('PO-005', 'S-CN-02', '2026-08-20', 60000, 'OPEN'),
  ('PO-006', 'S-VN-01', '2026-08-22', 33000, 'OPEN'),
  ('PO-007', 'S-CN-02', '2026-09-01', 12000, 'OPEN'),
  ('PO-008', 'S-KR-01', '2026-09-02', 27000, 'OPEN');

CREATE TABLE erp.po_line (
    po_line_id int  PRIMARY KEY,
    po_id      text NOT NULL REFERENCES erp.purchase_order,
    item_id    text NOT NULL REFERENCES erp.item,
    qty        int  NOT NULL
);
INSERT INTO erp.po_line VALUES
  (1, 'PO-001', 'I-PCB', 2000),
  (2, 'PO-002', 'I-PCB', 1000),
  (3, 'PO-003', 'I-MAG',  300),
  (4, 'PO-004', 'I-CAB', 1500),
  (5, 'PO-005', 'I-MAG',  800),
  (6, 'PO-006', 'I-CAB', 2000),
  (7, 'PO-006', 'I-PCB',  500),
  (8, 'PO-007', 'I-MAG',  200),
  (9, 'PO-008', 'I-MAG',  600);

-- 패브릭이 쓰는 읽기 전용 계정.
CREATE ROLE fabric_ro LOGIN PASSWORD 'fabric-ro-pw';
GRANT USAGE ON SCHEMA erp TO fabric_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA erp TO fabric_ro;
