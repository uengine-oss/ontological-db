#!/usr/bin/env python3
"""The virtual-fabric demo video. See ../demo_lib.py for how a demo is recorded.

    python3 record_demo.py            # → docs/tutorials/media/virtual-fabric-demo.mp4
    FAST=1 python3 record_demo.py     # same checks, short holds, no file kept

Needs the fabric-tut sources and a Studio running with PGDATABASE=demo_fabric.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import Demo, Stop, record


def story(d: Demo):
    page = d.page

    d.scene("title")
    d.card("<h1>흩어진 공급망 데이터를<br><em>복사 없이, 사이퍼 하나로</em></h1>"
           "<p>구매는 ERP, 물류는 WMS, 고객은 CRM.<br>세 시스템이 같은 것을 서로 다르게 부릅니다.</p>"
           "<div class='row'><span class='chip'>ERP · <b>PostgreSQL</b></span><span class='chip'>WMS · <b>MySQL</b></span>"
           "<span class='chip'>CRM · <b>MySQL</b></span></div>", 7,
           "흩어진 공급망 데이터를 복사 없이, 사이퍼 하나로. 구매는 ERP, 물류는 WMS, 고객은 CRM에 있습니다.")

    d.scene("graph")
    d.caption("빈 데이터베이스에서 시작합니다. 먼저 그래프를 하나 만듭니다.", read=False)
    d.run("CREATE GRAPH scm", expect="graph scm created", look=2.2)
    # The graph list is read when the page loads; this is the one reload.
    page.reload()
    page.wait_for_selector("#graph-select option", state="attached")
    page.select_option("#graph-select", "scm")
    d.overlay()

    d.scene("datasources")
    d.caption("접속 비밀번호는 파라미터로 넣습니다. 질의 기록에도 화면에도 남지 않습니다.", read=False)
    d.run(":param pw => 'fabric-ro-pw'", expect="not saved", look=3.0)
    d.caption("ERP를 데이터소스로 등록합니다. 원천은 PostgreSQL입니다.", read=False)
    d.run("CREATE DATASOURCE erp WITH ENGINE = 'postgres',\n"
          "  PARAMETERS = {host: 'fabric-tut-erp', database: 'erp', schema: 'erp', user: 'fabric_ro', password: $pw}",
          expect="4 tables as erp.*", look=2.6)
    d.caption("물류와 고객 시스템은 MySQL에 있습니다. 같은 방법으로 등록합니다.", read=False)
    d.run("CREATE DATASOURCE wms WITH ENGINE = 'mysql',\n"
          "  PARAMETERS = {host: 'fabric-tut-mysql', database: 'wms', user: 'fabric_ro', password: $pw}",
          expect="2 tables as wms.*", look=1.4)
    d.run("CREATE DATASOURCE crm WITH ENGINE = 'mysql',\n"
          "  PARAMETERS = {host: 'fabric-tut-mysql', database: 'crm', user: 'fabric_ro', password: $pw}",
          expect="3 tables as crm.*", look=1.4)
    d.clear()
    d.run("SHOW DATASOURCES", expect=["erp", "wms", "crm"], look=0.4)
    d.run("SHOW TABLES FROM wms", expect=["wms.shipment", "wms.stock_daily"], look=0.4)
    d.caption("등록만 했는데 원천의 테이블이 보입니다. 데이터는 한 행도 옮기지 않았습니다.")

    d.scene("first node")
    d.clear()
    d.caption("ERP의 공급사 테이블을 그래프의 노드로 선언합니다.", read=False)
    d.run("CREATE VIRTUAL NODE Supplier FROM erp.supplier KEY supplier_id", expect="virtual node Supplier", look=2.4)
    d.caption("이제 공급사는 평범한 라벨입니다. 중국 공급사를 찾아봅니다.", read=False)
    d.run("MATCH (s:Supplier)\nWHERE s.country = 'China'\nRETURN s.supplier_id AS id, s.name AS name, s.tier AS tier",
          expect=["Shenzhen Goldline PCB", "Ningbo Magnetics"], look=3.0)
    d.caption("이 질의는 어디서 실행됐을까요? 앞에 EXPLAIN FABRIC을 붙여 봅니다.", read=False)
    d.run("EXPLAIN FABRIC\nMATCH (s:Supplier)\nWHERE s.country = 'China'\nRETURN s.supplier_id AS id, s.name AS name",
          expect=["remote:erp", "country = 'China'"], look=1.0)
    d.caption("조건이 ERP로 내려갔습니다. 다섯 행 중 두 행만 넘어왔습니다.")

    d.scene("relationships")
    d.clear()
    d.caption("발주와 선적도 올립니다. 선적은 MySQL의 WMS에 있습니다.", read=False)
    d.run("CREATE VIRTUAL NODE PurchaseOrder FROM erp.purchase_order KEY po_id", expect="virtual node PurchaseOrder", look=0.8)
    d.run("CREATE VIRTUAL NODE Shipment FROM wms.shipment KEY shipment_no", expect="virtual node Shipment", look=1.2)
    d.caption("WMS는 발주를 ref_po_no, 공급사를 vendor_code라고 부릅니다. 그 대응을 관계로 한 번만 선언합니다.", read=False)
    d.run("CREATE VIRTUAL RELATIONSHIP ORDERED_FROM FROM PurchaseOrder(supplier_id) TO Supplier(supplier_id)",
          expect="ORDERED_FROM", look=0.8)
    d.run("CREATE VIRTUAL RELATIONSHIP SHIPS FROM Shipment(ref_po_no) TO PurchaseOrder(po_id)", expect="SHIPS", look=0.8)
    d.run("CREATE VIRTUAL RELATIONSHIP SHIPPED_BY FROM Shipment(vendor_code) TO Supplier(supplier_id)",
          expect="SHIPPED_BY", look=2.4)

    d.scene("cross-db join")
    d.clear()
    d.caption("닷새 넘게 지연된 선적과, 그 발주와 공급사를 묻습니다.", read=False)
    d.run("MATCH (sh:Shipment)-[:SHIPS]->(po:PurchaseOrder)-[:ORDERED_FROM]->(s:Supplier)\n"
          "WHERE sh.delay_days >= 5\n"
          "RETURN sh.shipment_no AS shipment, sh.delay_days AS delay, po.po_id AS po_no, s.name AS supplier\n"
          "ORDER BY delay DESC", expect=["SHP-03", "SHP-02", "Shenzhen Goldline PCB"], look=1.0)
    d.caption("MySQL의 선적에서 PostgreSQL의 발주와 공급사까지. 조인 조건은 한 번도 적지 않았습니다.")
    d.run("EXPLAIN FABRIC\nMATCH (sh:Shipment)-[:SHIPS]->(po:PurchaseOrder)-[:ORDERED_FROM]->(s:Supplier)\n"
          "WHERE sh.delay_days >= 5\nRETURN sh.shipment_no AS shipment, s.name AS supplier",
          expect=["remote:wms", "remote:erp", "local"], look=1.0)
    d.caption("MySQL은 지연 조건으로 두 행만 보냈고, ERP는 발주와 공급사를 자기 안에서 조인했습니다. 두 결과는 여기서 합쳐집니다.")

    d.scene("graph-only knowledge")
    d.clear()
    d.caption("리스크 정보는 어느 업무 시스템에도 없습니다. 그래프에 직접 저장합니다.", read=False)
    d.run("CREATE (ea:Region {name: 'East Asia'}), (sea:Region {name: 'Southeast Asia'})\n"
          "CREATE (:RiskEvent {title: '선전 항만 적체', severity: 'high', supplier_code: 'S-CN-01'})-[:IN_REGION]->(ea)\n"
          "CREATE (:RiskEvent {title: '수출 허가 심사', severity: 'medium', supplier_code: 'S-CN-02'})-[:IN_REGION]->(ea)\n"
          "CREATE (:RiskEvent {title: '하이퐁 태풍', severity: 'high', supplier_code: 'S-VN-01'})-[:IN_REGION]->(sea)",
          look=1.6)
    d.caption("저장된 리스크를 ERP의 공급사와 잇습니다. 한쪽은 그래프, 한쪽은 원천입니다.", read=False)
    d.run("CREATE VIRTUAL RELATIONSHIP AFFECTS FROM RiskEvent(supplier_code) TO Supplier(supplier_id)",
          expect="AFFECTS", look=2.4)

    d.scene("the question")
    d.clear()
    d.caption("처음의 질문입니다. 동아시아에 리스크가 걸린 공급사의, 아직 도착하지 않은 선적은?", read=False)
    d.run("MATCH (:Region {name: 'East Asia'})<-[:IN_REGION]-(e:RiskEvent)-[:AFFECTS]->(s:Supplier)<-[:SHIPPED_BY]-(sh:Shipment)\n"
          "WHERE sh.ship_status <> 'DELIVERED'\n"
          "RETURN e.title AS risk, s.name AS supplier, sh.shipment_no AS shipment, sh.ship_status AS status",
          expect=["선전 항만 적체", "수출 허가 심사", "SHP-06", "CUSTOMS_HOLD"], look=1.0)
    d.caption("리스크는 그래프에, 공급사는 ERP에, 선적은 WMS에 있습니다. 질의는 하나입니다.")
    d.caption("노드를 그대로 반환하면 그래프로 보입니다.", read=False)
    d.run("MATCH (rg:Region)<-[i:IN_REGION]-(e:RiskEvent)-[a:AFFECTS]->(s:Supplier)\nRETURN rg, i, e, a, s", look=3.0)
    if "Graph" not in " ".join(page.locator(".frame").first.locator(".tab").all_inner_texts()):
        raise Stop("the result of the node query has no Graph tab")
    d.caption("저장된 지역·리스크 노드와 ERP의 공급사 행이 한 그림에 섞여 있습니다.")

    d.scene("defined by a query")
    d.clear()
    d.caption("두 시스템을 조인해 집계한 납기 성적표도 노드로 정의합니다. SQL은 이 정의에만 쓰입니다.", read=False)
    d.run("CREATE VIRTUAL NODE DeliveryScore KEY supplier_id AS\n"
          "  SELECT s.supplier_id, count(*)::int AS shipments, max(sh.delay_days) AS worst_delay\n"
          "    FROM erp.supplier s JOIN wms.shipment sh ON sh.vendor_code = s.supplier_id\n"
          "   GROUP BY s.supplier_id", expect="virtual node DeliveryScore", look=1.2)
    d.run("CREATE VIRTUAL RELATIONSHIP SCORE_OF FROM DeliveryScore(supplier_id) TO Supplier(supplier_id)",
          expect="SCORE_OF", look=1.0)
    d.caption("리스크가 걸린 공급사의 납기 성적을 함께 봅니다.", read=False)
    d.run("MATCH (e:RiskEvent)-[:AFFECTS]->(s:Supplier)<-[:SCORE_OF]-(d:DeliveryScore)\n"
          "RETURN e.title AS risk, s.name AS supplier, d.shipments AS shipments, d.worst_delay AS worst_delay\n"
          "ORDER BY worst_delay DESC", expect=["Shenzhen Goldline PCB", "9"], look=4.0)

    d.scene("read-only")
    d.clear()
    d.caption("원천의 데이터를 바꾸려 하면 어떻게 될까요?", read=False)
    d.run("MATCH (s:Supplier {supplier_id: 'S-KR-01'}) SET s.tier = 9", error=True, expect="read-only", look=1.0)
    d.caption("거부됩니다. 패브릭은 원천 시스템에 쓰지 않습니다.")

    d.scene("end")
    d.card("<h1>세 가지 선언으로<br><em>흩어진 데이터가 하나의 그래프</em>가 됩니다</h1>"
           "<div class='row'><span class='chip'><b>CREATE</b> DATASOURCE</span>"
           "<span class='chip'><b>CREATE</b> VIRTUAL NODE</span><span class='chip'><b>CREATE</b> VIRTUAL RELATIONSHIP</span></div>"
           "<p>복사 없음 · 원천은 읽기 전용 · Studio와 Neo4j 드라이버에서 같은 문장</p>", 7,
           "세 가지 선언으로 흩어진 데이터가 하나의 그래프가 됩니다. 데이터소스, 가상 노드, 가상 관계.")
    d.scene(None)


if __name__ == "__main__":
    record("virtual-fabric", story, db="demo_fabric")
