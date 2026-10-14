-- WMS·CRM (MySQL). 같은 것을 ERP 와 다르게 부른다:
--   공급사  ERP supplier_id  ↔ WMS vendor_code
--   발주    ERP po_id        ↔ WMS ref_po_no
--   품목    ERP item_id      ↔ WMS·CRM sku_code
SET NAMES utf8mb4;

CREATE DATABASE wms CHARACTER SET utf8mb4;
CREATE TABLE wms.shipment (
    shipment_no varchar(20) PRIMARY KEY,
    ref_po_no   varchar(20) NOT NULL,
    vendor_code varchar(20) NOT NULL,
    ship_status varchar(20) NOT NULL,
    delay_days  int NOT NULL,
    eta         date NOT NULL
);
INSERT INTO wms.shipment VALUES
  ('SHP-01', 'PO-001', 'S-CN-01', 'DELIVERED',    0, '2026-08-20'),
  ('SHP-02', 'PO-001', 'S-CN-01', 'IN_TRANSIT',   7, '2026-09-12'),
  ('SHP-03', 'PO-002', 'S-CN-01', 'CUSTOMS_HOLD', 9, '2026-09-15'),
  ('SHP-04', 'PO-003', 'S-KR-01', 'DELIVERED',    0, '2026-08-12'),
  ('SHP-05', 'PO-004', 'S-KR-02', 'DELIVERED',    1, '2026-08-16'),
  ('SHP-06', 'PO-005', 'S-CN-02', 'IN_TRANSIT',   3, '2026-09-10'),
  ('SHP-07', 'PO-006', 'S-VN-01', 'BOOKED',       0, '2026-09-20'),
  -- ERP 에 없는 발주를 가리키는 선적. 조인에서 어떻게 되는지 보려고 둔다.
  ('SHP-08', 'PO-999', 'S-XX-99', 'BOOKED',       0, '2026-09-25');

-- 단일 키가 없는 테이블: (날짜, 품목, 창고) 가 키다.
CREATE TABLE wms.stock_daily (
    snapshot_date date NOT NULL,
    sku_code      varchar(20) NOT NULL,
    warehouse_cd  varchar(10) NOT NULL,
    on_hand_qty   int NOT NULL,
    PRIMARY KEY (snapshot_date, sku_code, warehouse_cd)
);
INSERT INTO wms.stock_daily VALUES
  ('2026-09-01', 'I-PCB', 'WH-A',  420), ('2026-09-01', 'I-PCB', 'WH-B', 900),
  ('2026-09-01', 'I-CAB', 'WH-A', 1500), ('2026-09-01', 'I-MAG', 'WH-A', 250),
  ('2026-09-01', 'I-FG',  'WH-A',   80),
  ('2026-09-02', 'I-PCB', 'WH-A',  380), ('2026-09-02', 'I-PCB', 'WH-B', 880),
  ('2026-09-02', 'I-CAB', 'WH-A',  950), ('2026-09-02', 'I-MAG', 'WH-A', 310),
  ('2026-09-02', 'I-FG',  'WH-A',   45);

CREATE DATABASE crm CHARACTER SET utf8mb4;
CREATE TABLE crm.customer (
    cust_no varchar(20) PRIMARY KEY,
    name    varchar(100) NOT NULL,
    country varchar(40) NOT NULL
);
INSERT INTO crm.customer VALUES
  ('C-100', '서울전자',      'Korea'),
  ('C-200', 'Tokyo Devices', 'Japan'),
  ('C-300', 'Berlin Motion', 'Germany');

CREATE TABLE crm.sales_order (
    so_no    varchar(20) PRIMARY KEY,
    cust_no  varchar(20) NOT NULL,
    sku_code varchar(20) NOT NULL,
    qty      int NOT NULL,
    status   varchar(20) NOT NULL
);
INSERT INTO crm.sales_order VALUES
  ('SO-01', 'C-100', 'I-FG',   20, 'SHIPPED'),
  ('SO-02', 'C-100', 'I-FG',   30, 'OPEN'),
  ('SO-03', 'C-200', 'I-FG',   15, 'OPEN'),
  ('SO-04', 'C-300', 'I-CAB', 200, 'SHIPPED'),
  ('SO-05', 'C-200', 'I-PCB', 100, 'OPEN');

CREATE TABLE crm.claim (
    ticket_no   varchar(20) PRIMARY KEY,
    so_no       varchar(20) NOT NULL,
    severity    varchar(10) NOT NULL,
    opened_date date NOT NULL,
    subject     varchar(200) NOT NULL
);
INSERT INTO crm.claim VALUES
  ('T-1', 'SO-01', 'HIGH', '2026-08-28', '컨트롤러 전원 불량'),
  ('T-2', 'SO-04', 'LOW',  '2026-08-30', '케이블 포장 파손'),
  ('T-3', 'SO-05', 'HIGH', '2026-09-03', 'PCB 납땜 불량');

-- 패브릭이 쓰는 읽기 전용 계정.
CREATE USER 'fabric_ro'@'%' IDENTIFIED BY 'fabric-ro-pw';
GRANT SELECT ON wms.* TO 'fabric_ro'@'%';
GRANT SELECT ON crm.* TO 'fabric_ro'@'%';
