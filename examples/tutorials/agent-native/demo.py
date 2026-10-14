#!/usr/bin/env python3
"""The agent-native demo video. See ../demo_lib.py for how a demo is recorded.

    python3 demo.py            # → docs/tutorials/media/agent-native-demo.mp4
    FAST=1 python3 demo.py     # same checks, short holds, no file kept

Needs a Studio running with PGDATABASE=demo_agent_native and OG_STUDIO_ALLOW_RAW_SQL=1
(TUT_STUDIO names its address). The graph is the one the tutorial builds: it is
loaded before the browser opens, because the story is the agent's, not the loader's.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import Demo, Stop, record

SETUP = r"""
SELECT og_create_graph('shop');
SELECT og_create_type('shop', t, 'entity') FROM unnest(ARRAY['Customer', 'Order', 'Product', 'Category', 'Warehouse']) t;
SELECT og_add_property('shop', 'Customer', 'name', 'string', true, true), og_add_property('shop', 'Customer', 'city', 'string'),
       og_add_property('shop', 'Order', 'order_no', 'string', true, true), og_add_property('shop', 'Order', 'status', 'string'),
       og_add_property('shop', 'Order', 'amount', 'int'), og_add_property('shop', 'Product', 'sku', 'string', true, true),
       og_add_property('shop', 'Product', 'name', 'string'), og_add_property('shop', 'Category', 'name', 'string', true, true),
       og_add_property('shop', 'Warehouse', 'name', 'string', true, true);
SELECT og_create_type('shop', 'PLACED', 'relation'), og_add_role('shop', 'PLACED', 'customer', 'Customer', 0), og_add_role('shop', 'PLACED', 'order', 'Order', 1);
SELECT og_create_type('shop', 'CONTAINS', 'relation'), og_add_role('shop', 'CONTAINS', 'order', 'Order', 0), og_add_role('shop', 'CONTAINS', 'product', 'Product', 1);
SELECT og_create_type('shop', 'IN_CATEGORY', 'relation'), og_add_role('shop', 'IN_CATEGORY', 'product', 'Product', 0), og_add_role('shop', 'IN_CATEGORY', 'category', 'Category', 1);
SELECT og_create_type('shop', 'SHIPS_FROM', 'relation'), og_add_role('shop', 'SHIPS_FROM', 'order', 'Order', 0), og_add_role('shop', 'SHIPS_FROM', 'warehouse', 'Warehouse', 1);
SELECT og_enable_history('shop', 'Order');
SELECT og_cypher('shop', $$
CREATE (c1:Customer {name: '김하늘', city: '서울'}), (c2:Customer {name: '이도윤', city: '부산'}), (c3:Customer {name: '박서연', city: '서울'})
CREATE (k1:Category {name: '입력장치'}), (k2:Category {name: '거치대'})
CREATE (p1:Product {sku: 'P-100', name: '무선 키보드'}), (p2:Product {sku: 'P-200', name: '모니터 암'}), (p3:Product {sku: 'P-300', name: '노트북 스탠드'})
CREATE (w1:Warehouse {name: '이천'}), (w2:Warehouse {name: '김해'})
CREATE (o1:Order {order_no: 'O-1001', status: '배송완료', amount: 89000}), (o2:Order {order_no: 'O-1002', status: '접수', amount: 132000}),
       (o3:Order {order_no: 'O-1003', status: '접수', amount: 45000}), (o4:Order {order_no: 'O-1004', status: '배송중', amount: 45000})
CREATE (c1)-[:PLACED]->(o1), (c1)-[:PLACED]->(o2), (c2)-[:PLACED]->(o3), (c3)-[:PLACED]->(o4)
CREATE (o1)-[:CONTAINS]->(p1), (o2)-[:CONTAINS]->(p2), (o2)-[:CONTAINS]->(p3), (o3)-[:CONTAINS]->(p3), (o4)-[:CONTAINS]->(p3)
CREATE (p1)-[:IN_CATEGORY]->(k1), (p2)-[:IN_CATEGORY]->(k2), (p3)-[:IN_CATEGORY]->(k2)
CREATE (o1)-[:SHIPS_FROM]->(w1), (o2)-[:SHIPS_FROM]->(w2), (o3)-[:SHIPS_FROM]->(w2), (o4)-[:SHIPS_FROM]->(w1)
$$);
SELECT og_cypher('shop', $$MATCH (o:Order {order_no: 'O-1002'}) SET o.status = '배송중'$$);
SELECT og_cypher('shop', $$MATCH (o:Order) WHERE o.order_no IN ['O-1002', 'O-1003'] SET o.status = '지연'$$);
"""

TYPO = "MATCH (c:Custmer)-[:PLACED]->(o:Order)\nRETURN c.name AS customer, o.order_no AS order_no"
REVERSED = "MATCH (o:Order)-[:PLACED]->(c:Customer)\nRETURN c.name AS customer, o.order_no AS order_no"
GUESSED = "MATCH (c:Customer)-[:PLACED]->(o:Order)\nWHERE o.status = 'delayed'\nRETURN c.name AS customer, o.order_no AS order_no"
CARTESIAN = "MATCH (o:Order), (w:Warehouse)\nWHERE o.status = '지연'\nRETURN o.order_no AS order_no, w.name AS warehouse"
ANSWER = ("MATCH (c:Customer)-[:PLACED]->(o:Order)-[:SHIPS_FROM]->(w:Warehouse)\nWHERE o.status = '지연'\n"
          "RETURN o.order_no AS order_no, c.name AS customer, w.name AS warehouse")


def zero_rows(d: Demo, statement):
    """Run a statement that must succeed and return nothing."""
    d.run(statement, look=1.6)
    meta = d.page.locator(".frame").first.inner_text()
    if "0 rows" not in meta:
        raise Stop(f"expected 0 rows for: {statement[:60]}\n{meta[:300]}")


def story(d: Demo):
    page = d.page
    page.wait_for_selector("#graph-select option", state="attached")
    page.select_option("#graph-select", "shop")

    d.scene("title")
    d.card("<h1>처음 보는 그래프에서<br><em>에이전트가 답을 찾기까지</em></h1>"
           "<p>AI 에이전트는 라벨을 지어내고, 화살표를 거꾸로 쓰고, 관계를 빼먹습니다.<br>"
           "그때 데이터베이스가 무엇을 알려 주는지 따라가 봅니다.</p>"
           "<div class='row'><span class='chip'><b>:schema</b></span><span class='chip'><b>:why</b></span>"
           "<span class='chip'><b>:explain</b></span></div>", 7,
           "처음 보는 그래프에서 에이전트가 답을 찾기까지. 에이전트가 틀릴 때 데이터베이스가 무엇을 알려 주는지 따라가 봅니다.")

    d.scene("the graph")
    d.caption("작은 쇼핑몰의 주문 그래프입니다. 고객이 주문을 넣고, 주문은 창고에서 나갑니다.", read=False)
    d.run("MATCH (c:Customer)-[p:PLACED]->(o:Order)-[s:SHIPS_FROM]->(w:Warehouse)\nRETURN c, p, o, s, w", look=3.0)
    if "Graph" not in " ".join(page.locator(".frame").first.locator(".tab").all_inner_texts()):
        raise Stop("the result of the node query has no Graph tab")
    d.caption("운영팀이 에이전트에게 묻습니다. 지연된 주문은 누가 넣었고, 어느 창고에서 나가나?")

    d.scene("schema")
    d.clear()
    d.caption("에이전트는 이 그래프를 처음 봅니다. 먼저 스키마를 읽습니다.", read=False)
    d.run(":schema", expect=['"schema_version"', '"relation_types"', '"player_type": "Customer"', '"position": "source"'], look=1.0)
    # Bring PLACED — the relationship the story turns on — and its roles into view.
    page.evaluate("""() => {
        const pre = document.querySelector('.frame .body pre') || document.querySelector('.frame .body');
        const at = pre.textContent.indexOf('"name": "PLACED"');
        let box = pre; while (box && box.scrollHeight <= box.clientHeight + 4) box = box.parentElement;
        if (box && at > 0) box.scrollTop = box.scrollHeight * at / pre.textContent.length - 40;
    }""")
    d.caption("타입과 프로퍼티, 그리고 관계마다 어느 쪽이 출발이고 어느 쪽이 도착인지가 적혀 있습니다.")

    d.scene("typo")
    d.clear()
    d.caption("첫 질의입니다. 에이전트가 Customer를 Custmer로 썼습니다.", read=False)
    zero_rows(d, TYPO)
    d.caption("오류는 없고, 결과도 없습니다. 왜 비었는지 :why 로 묻습니다.", read=False)
    d.run(":why " + TYPO.replace("\n", " "), expect=["(c:Custmer)", "0 rows", "this is where the match became empty"], look=1.0)
    d.caption("패턴의 첫 요소에서 이미 0건입니다. 라벨 이름부터 틀렸다는 뜻입니다.")

    d.scene("direction")
    d.clear()
    d.caption("라벨을 고쳤습니다. 이번에는 화살표를 주문에서 고객 쪽으로 그렸습니다.", read=False)
    zero_rows(d, REVERSED)
    d.run(":why " + REVERSED.replace("\n", " "),
          expect=["(o:Order) → 4 rows", "(o:Order)-[:PLACED]->(c:Customer) → 0 rows", "direction of the arrow"], look=1.0)
    d.caption("주문은 네 건인데, 화살표를 붙이는 순간 0건이 됩니다. 방향이 반대였습니다.")

    d.scene("where")
    d.clear()
    d.caption("방향을 고치고, 지연 상태를 'delayed' 라고 짐작해 조건을 달았습니다.", read=False)
    d.run(":why " + GUESSED.replace("\n", " "),
          expect=["(c:Customer)-[:PLACED]->(o:Order) → 4 rows", "the WHERE clause removed every row"], look=1.0)
    d.caption("패턴은 네 건까지 맞았습니다. 조건이 전부 지웠습니다. 실제 값을 확인합니다.", read=False)
    d.run("MATCH (o:Order) RETURN o.status AS status, count(*) AS n ORDER BY n DESC", expect=["지연", "배송완료", "배송중"], look=1.0)
    d.caption("상태 값은 한국어였습니다. 찾는 값은 '지연' 입니다.")

    d.scene("before running")
    d.clear()
    d.caption("창고를 붙이면서 주문과 창고 사이의 관계를 빼먹었습니다. 실행하기 전에 먼저 묻습니다.", read=False)
    d.run(":why " + CARTESIAN.replace("\n", " "), expect=["Estimate", "cartesian product"], look=1.0)
    d.caption("관계로 이어지지 않은 노드가 있어 카티션 곱이 된다고 알려 줍니다. 실행하지 않고 고칩니다.")
    d.caption("고친 질의가 어떤 SQL과 실행 계획이 되는지도 실행 전에 볼 수 있습니다.", read=False)
    d.run(":explain " + ANSWER.replace("\n", " "), expect=["SELECT", "og_adj"], look=3.0)

    d.scene("the answer")
    d.clear()
    d.caption("이제 실행합니다.", read=False)
    d.run(ANSWER, expect=["O-1002", "김하늘", "O-1003", "이도윤", "김해"], look=1.0)
    d.caption("지연된 주문은 둘이고, 둘 다 김해 창고에서 나갑니다.")

    d.scene("history")
    d.clear()
    d.caption("운영팀이 이어서 묻습니다. O-1002는 처음부터 지연이었나? 주문에는 변경 이력을 켜 두었습니다.", read=False)
    d.run(":sql SELECT h.op, h.payload->>'p_status' AS status, h.recorded_at\n"
          "FROM og_cypher('shop', $$MATCH (o:Order {order_no: 'O-1002'}) RETURN id(o) AS id$$) r,\n"
          "     og_history((r->>'id')::bigint) h", expect=["접수", "배송중", "지연"], look=1.0)
    d.caption("접수로 들어와 배송중이 됐다가 지연으로 바뀌었습니다. 최근 변경이 위에 있습니다.")
    d.caption("고객 정보에도 이력을 켜려다 타입 이름을 틀렸습니다.", read=False)
    d.run(":sql SELECT og_enable_history('shop', 'Custmer')", error=True, expect=["does not exist", "did you mean", "Customer"], look=1.0)
    d.caption("카탈로그 함수는 틀린 이름에 가장 가까운 실제 이름을 함께 돌려줍니다.")

    d.scene("audit")
    d.caption("에이전트가 무엇을 했는지는 나중에도 볼 수 있습니다. 질의 기록을 엽니다.", read=False)
    page.click('.rail-btn[data-panel="audit"]')
    page.click("#audit-refresh")
    page.wait_for_selector("#audit-list .row")
    d.hold(1.0)
    log = page.locator("#audit-list").inner_text()
    for want in ["Custmer", "(o:Order)-[:PLACED]->(c:Customer)", "SHIPS_FROM"]:
        if want not in log:
            raise Stop(f"the query log does not show '{want}':\n{log[:400]}")
    d.caption("0건으로 끝난 질의도, 답을 낸 질의도 실행된 순서대로 감사 로그에 남아 있습니다.")

    d.scene("role")
    page.click('.rail-btn[data-panel="schema"]')
    d.clear()
    d.caption("에이전트 역할에는 시간과 메모리 상한을 선언해 둡니다.", read=False)
    d.run(":sql SELECT og_create_role('analyst', '{\"statement_timeout_ms\": 5000, \"work_mem_kb\": 65536}')", look=0.8)
    d.run(":sql SELECT name, limits FROM og_catalog.agent_role", expect=["analyst", "statement_timeout_ms", "work_mem_kb"], look=1.0)
    d.caption("저장된 역할은 og_apply_role 로 에이전트의 세션에 적용합니다. 제한은 그 세션에 걸립니다.")

    d.scene("end")
    d.card("<h1>읽고, 고치고, 가늠하고<br><em>한 일은 기록으로 남습니다</em></h1>"
           "<div class='row'><span class='chip'><b>og_schema</b></span><span class='chip'><b>og_diagnose_empty</b></span>"
           "<span class='chip'><b>og_estimate</b></span><span class='chip'><b>og_history</b></span>"
           "<span class='chip'><b>og_audit</b></span></div>"
           "<p>전부 SQL 함수입니다. Studio에서는 :schema · :why · :explain</p>", 7,
           "읽고, 고치고, 가늠하고, 한 일은 기록으로 남습니다. 전부 SQL 함수이고, Studio에서는 schema, why, explain 명령입니다.")
    d.scene(None)


if __name__ == "__main__":
    record("agent-native", story, setup_sql=SETUP)
