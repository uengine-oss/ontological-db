#!/usr/bin/env python3
"""The schema-evolution demo video. See ../demo_lib.py for how a demo is recorded.

    python3 demo.py            # → docs/tutorials/media/schema-evolution-demo.mp4
    FAST=1 python3 demo.py     # same checks, short holds, no file kept

Needs a Studio running with PGDATABASE=demo_schema_evolution and
OG_STUDIO_ALLOW_RAW_SQL=1 (TUT_STUDIO names its address).

The story is the tutorial's HR graph: `age` was declared a string. The change to
an integer is staged and run in the Studio's schema designer, stops at the one
value that cannot convert, and is finished after that value is fixed with an
ordinary write — while the same query is asked before, during and after. No
LLM key is needed: the "Ask" box of the designer is not used.
"""
import os, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import Demo, Stop, record

# The graph as production left it: five people, an index, and `age` as text.
SETUP = """
SELECT og_create_graph('hr');
SELECT og_create_type('hr', 'Person', 'entity');
SELECT og_add_property('hr', 'Person', 'name', 'string');
SELECT og_add_property('hr', 'Person', 'age',  'string');
SELECT og_create_type('hr', 'Employee', 'entity', ARRAY['Person']);
SELECT og_create_index('hr', 'Person', 'age');
SELECT * FROM og_cypher('hr', $$CREATE (:Person {name: 'ann', age: '31'}), (:Person {name: 'bob', age: '42'}),
  (:Person {name: 'cy', age: 'abc'}), (:Person {name: 'dee'}), (:Employee {name: 'eve', age: '9'})$$);
"""

OVER_40_TEXT = "MATCH (p:Person) WHERE p.age > '40'\nRETURN p.name AS name, p.age AS age ORDER BY name"
OVER_40_INT = "MATCH (p:Person) WHERE p.age > 40\nRETURN p.name AS name, p.age AS age ORDER BY name"


def panel(d: Demo, name):
    """Switch the sidebar panel; 'evolve' opens the schema designer, anything else returns to the editor."""
    d.page.click(f".rail-btn[data-panel='{name}']")
    d.page.wait_for_selector("#dz-canvas .dcard" if name == "evolve" else "#editor", state="visible")
    time.sleep(0.4)


def see(d: Demo, selector, wants, timeout=30):
    """Wait until the element shows every expected text; stop the recording if it never does."""
    deadline = time.monotonic() + timeout
    text = ""
    while time.monotonic() < deadline:
        text = d.page.locator(selector).first.inner_text()
        if all(w.lower() in text.lower() for w in wants):      # some labels are upper-cased by CSS
            if os.environ.get("DEMO_DEBUG"):
                print(f"--- {selector}\n{text}\n")
            return text
        time.sleep(0.15)
    raise Stop(f"{selector} does not show {wants}:\n{text[:400]}")


def absent(text, what, where):
    if what in text:
        raise Stop(f"{where} should not show '{what}':\n{text[:300]}")


def story(d: Demo):
    page = d.page
    page.select_option("#graph-select", "hr")

    d.scene("title")
    d.card("<h1>운영 중인 그래프의<br><em>스키마를 멈추지 않고 바꾸기</em></h1>"
           "<p>나이를 문자열로 선언해 버린 인사 그래프.<br>데이터는 이미 들어 있고, 애플리케이션은 계속 돌아갑니다.</p>"
           "<div class='row'><span class='chip'><b>expand</b></span><span class='chip'><b>migrate</b></span>"
           "<span class='chip'><b>validate</b></span><span class='chip'><b>contract</b></span></div>", 7,
           "운영 중인 그래프의 스키마를 멈추지 않고 바꾸기. 나이를 문자열로 선언해 버린 인사 그래프가 있습니다.")

    d.scene("the mistake")
    d.caption("인사 그래프에 다섯 사람이 있습니다. 나이가 문자열로 들어가 있습니다.", read=False)
    d.run("MATCH (p:Person) RETURN p.name AS name, p.age AS age, labels(p) AS labels ORDER BY name",
          expect=["ann", "abc", "dee", "Employee"], look=3.0)
    d.caption("마흔이 넘은 사람을 찾으면 아홉 살 eve가 나옵니다. 문자열은 사전순으로 비교하기 때문입니다.", read=False)
    text = d.run(OVER_40_TEXT, expect=["bob", "cy", "eve"], look=1.0)
    absent(text, "ann", "the string comparison")
    d.caption("맞는 답은 bob 한 명입니다. age를 정수로 바꿔야 합니다.")

    d.scene("stage the change")
    d.caption("스키마 디자이너를 엽니다. 타입과 프로퍼티, 그리고 실제 컬럼이 지도로 보입니다.", read=False)
    panel(d, "evolve")
    see(d, "#dz-canvas", ["Person", "age", "p_age", "Employee"])
    d.hold(3.0)
    d.caption("Person의 age를 눌러 타입을 int로 바꾸겠다고 올려 둡니다. 아직 아무것도 실행되지 않습니다.", read=False)
    page.click("#dz-canvas .prop[data-type='Person'][data-prop='age']")
    page.wait_for_selector("#dz-menu")
    d.hold(1.6)
    page.click("#dz-menu button:text-is('int')")
    see(d, "#dz-plan", ["alter property type", "staged", "Person.age"])
    d.hold(2.4)
    d.caption("먼저 영향을 봅니다. 계획 단계는 데이터를 바꾸지 않습니다.", read=False)
    page.click("#dz-review")
    see(d, "#dz-plan", ["reviewed", "1 value(s) will not convert", "5 rows in scope"])
    d.hold(1.0)
    d.caption("다섯 행이 대상이고, 그중 값 하나는 정수로 바뀌지 않는다고 알려 줍니다.")

    d.scene("run until blocked")
    d.caption("실행합니다. 새 컬럼을 옆에 만들고, 기존 값을 배치로 옮기고, 검증합니다.", read=False)
    page.click("#dz-run")
    see(d, "#dz-plan", ["blocked", "expand", "migrate", "validation found 1 violation(s)", "68719476739"])
    d.hold(1.2)
    d.caption("검증에서 멈췄습니다. 막고 있는 노드의 id를 알려 줍니다. 옛 컬럼은 그대로 있습니다.")

    d.scene("during the change")
    d.caption("변경이 끝나지 않은 지금, 애플리케이션의 질의는 어떻게 될까요?", read=False)
    panel(d, "schema")
    d.clear()
    d.run(OVER_40_TEXT, expect=["bob", "cy", "eve"], look=1.0)
    d.caption("그대로 답합니다. 읽기는 아직 옛 컬럼을 봅니다.")
    d.caption("다만 새로 쓰는 값은 지금부터 정수로 바뀔 수 있어야 합니다.", read=False)
    d.run("CREATE (:Person {name: 'fay', age: 'old'})", error=True, expect="invalid input syntax for type integer", look=1.0)
    d.caption("정수가 될 수 없는 나이는 거부됩니다. 정리하는 동안 잘못된 값이 더 늘지 않습니다.")
    d.run("CREATE (:Person {name: 'gus', age: '60'})", look=0.6)
    d.run(":changes", expect=["alter_property_type", "migrating"], look=1.0)
    d.caption("60처럼 바뀔 수 있는 값은 평소대로 들어갑니다. 변경은 migrating 단계로 기록되어 있습니다.")

    d.scene("fix the row")
    d.clear()
    d.caption("막고 있던 노드를 찾아봅니다.", read=False)
    d.run("MATCH (p) WHERE id(p) = 68719476739 RETURN p.name AS name, p.age AS age", expect=["cy", "abc"], look=2.0)
    d.caption("cy의 나이를 평소 쓰던 방법으로 고칩니다. 새 컬럼에도 함께 쓰입니다.", read=False)
    d.run("MATCH (p:Person {name: 'cy'}) SET p.age = '38' RETURN p.name AS name, p.age AS age", expect=["cy", "38"], look=2.4)

    d.scene("finish")
    d.caption("디자이너로 돌아가 검증을 다시 시킵니다.", read=False)
    panel(d, "evolve")
    see(d, "#dz-plan", ["blocked", "Retry validate"])
    d.hold(1.6)
    page.click("#dz-plan [data-retry]")
    see(d, "#dz-plan", ["done", "contract", "rows rewritten"])
    see(d, "#dz-canvas", ["int4", "p_age"])
    d.hold(1.2)
    d.caption("위반이 없어 변경이 끝났습니다. 새 컬럼이 age라는 이름을 이어받았고, 지도의 타입도 int4입니다.")

    d.scene("after")
    panel(d, "schema")
    d.clear()
    d.caption("같은 질문을 이번에는 숫자로 묻습니다.", read=False)
    text = d.run(OVER_40_INT, expect=["bob", "42", "gus", "60"], look=1.0)
    absent(text, "eve", "the numeric comparison")
    absent(text, "cy", "the numeric comparison")
    d.caption("bob과, 변경 도중에 들어온 gus입니다. 아홉 살 eve는 더 나오지 않습니다.")

    d.scene("a rename costs nothing")
    d.caption("이름을 바꾸는 변경은 성격이 다릅니다. name을 fullName으로 바꿔 봅니다.", read=False)
    panel(d, "evolve")
    page.click("#dz-clear")
    page.click("#dz-canvas .prop[data-type='Person'][data-prop='name']")
    page.wait_for_selector("#dz-menu input")
    page.fill("#dz-menu input", "fullName")
    d.hold(1.6)
    page.click("#dz-menu button:text-is('Rename')")
    see(d, "#dz-plan", ["rename property", "staged"])
    d.hold(1.2)
    page.click("#dz-run")
    see(d, "#dz-plan", ["done", "0 / 6 rows rewritten"])
    see(d, "#dz-canvas", ["fullName", "p_name"])
    d.hold(1.0)
    d.caption("여섯 행 가운데 다시 쓰인 행은 0입니다. 이름은 카탈로그에만 있고, 컬럼은 여전히 p_name입니다.")

    d.scene("history")
    panel(d, "schema")
    d.clear()
    d.caption("스키마 변경은 기록으로 남습니다.", read=False)
    d.run(":changes", expect=["rename_property", "alter_property_type", "contracted"], look=3.0)
    d.caption("스키마 버전마다 카탈로그 스냅샷도 남습니다. 변경 전인 버전 6을 봅니다.", read=False)
    text = d.run(":snapshot 6", expect=["age", "text", "p_age", "name"], look=1.0)
    absent(text, "fullName", "the snapshot of version 6")
    d.caption("그때 age는 text였고, fullName은 없었습니다.")
    d.run(":snapshot", expect=["int4", "p_age", "fullName", "p_name"], look=1.0)
    d.caption("지금은 age가 int4이고, fullName이 예전 컬럼 p_name을 가리킵니다.")

    d.scene("end")
    d.card("<h1>스키마 변경은 한 문장이 아니라<br><em>기록되는 네 단계</em>입니다</h1>"
           "<div class='row'><span class='chip'><b>plan</b> 영향 보기</span><span class='chip'><b>expand</b> 옆에 추가</span>"
           "<span class='chip'><b>migrate</b> 배치로</span><span class='chip'><b>validate</b> id로 보고</span>"
           "<span class='chip'><b>contract</b> 교체</span></div>"
           "<p>읽기는 계속되고 · 막는 행은 평소의 쓰기로 고치고 · 이름 변경은 행을 다시 쓰지 않습니다</p>", 7,
           "스키마 변경은 한 문장이 아니라 기록되는 네 단계입니다. 계획, 확장, 이행, 검증, 축소.")
    d.scene(None)


if __name__ == "__main__":
    record("schema-evolution", story, setup_sql=SETUP)
