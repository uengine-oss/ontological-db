#!/usr/bin/env python3
"""The bitemporal demo video. See ../demo_lib.py for how a demo is recorded.

    python3 demo.py            # → docs/tutorials/media/temporal-demo.mp4
    FAST=1 python3 demo.py     # same checks, short holds, no file kept

Needs a Studio running with PGDATABASE=demo_temporal and OG_STUDIO_ALLOW_RAW_SQL=1
(TUT_STUDIO names its address). The story asks ONE question — who works here, as
what, for how much — and then only moves the instant in the Studio's time-travel
bar. Every answer the captions mention is asserted on the result frame, and so
is the claim that the compiled SQL is the same text at every instant.
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import FAST, Demo, Stop, record

# The schema is the boring part; it is in place before the recording starts.
# Versioning is switched on in front of the camera.
SETUP = """
SELECT og_create_graph('corp');
SELECT og_create_type('corp', 'Person', 'entity');
SELECT og_add_property('corp', 'Person', 'name', 'string'), og_add_property('corp', 'Person', 'salary', 'int');
SELECT og_create_type('corp', 'Company', 'entity'), og_add_property('corp', 'Company', 'name', 'string');
SELECT og_create_type('corp', 'WORKS_AT', 'relation'), og_add_property('corp', 'WORKS_AT', 'title', 'string');
"""

QUESTION = ("MATCH (p:Person)-[w:WORKS_AT]->(c:Company)\n"
            "RETURN p.name AS name, w.title AS title, p.salary AS salary ORDER BY name")


def dated(valid_from, cypher):
    """A write that says since when it holds. The time-travel bar only reads; a
    write names its valid time with og.valid_from, here for one statement."""
    return f":sql SELECT set_config('og.valid_from', '{valid_from}', true),\n  og_cypher('corp', $${cypher}$$)"


def story(d: Demo):
    page = d.page
    prints = []     # (label, compiled-SQL fingerprint) of every time the question was asked

    def ask(label, expect, absent=(), badge=None, look=2.4):
        """Ask the one question again and check the frame it produced."""
        text = d.run(QUESTION, expect=expect, look=look)
        for bad in absent:
            if bad in text:
                raise Stop(f"asked {label}: the answer should not show '{bad}':\n{text[:300]}")
        frame = page.locator(".frame").first
        shown = frame.locator(".asof-badge").inner_text() if frame.locator(".asof-badge").count() else ""
        if (badge or "") not in shown or (badge is None and shown):
            raise Stop(f"asked {label}: the frame's instant badge is '{shown}', expected '{badge}'")
        prints.append((label, frame.locator(".status .fp").inner_text()))

    def valid_at(when):
        page.click("#tb-valid")
        page.fill("#tb-valid", "")
        page.keyboard.type(when, delay=0 if FAST else 45)
        time.sleep(0.3)

    d.scene("title")
    d.card("<h1>같은 질문, 다른 시점<br><em>시간을 아는 그래프</em></h1>"
           "<p>그때 실제로 어땠는가, 그리고 그때 우리는 무엇을 알고 있었는가.<br>두 질문은 서로 다른 시간 축에 있습니다.</p>"
           "<div class='row'><span class='chip'>유효 시간 · <b>valid time</b></span>"
           "<span class='chip'>시스템 시간 · <b>system time</b></span></div>", 7,
           "같은 질문, 다른 시점. 그때 실제로 어땠는가, 그리고 그때 우리는 무엇을 알고 있었는가.")

    d.scene("enable")
    page.select_option("#graph-select", "corp")
    d.caption("인사 그래프입니다. 사람, 회사, 근무 관계 타입에 시간 버전을 켭니다.", read=False)
    d.run(":sql SELECT og_enable_temporal('corp', 'Person'), og_enable_temporal('corp', 'Company'),\n"
          "  og_enable_temporal('corp', 'WORKS_AT')", look=1.0)
    d.run(":sql SELECT og_temporal_types('corp') AS temporal_types", expect=["Person", "Company", "WORKS_AT"], look=2.2)

    d.scene("hire and raise")
    d.clear()
    d.caption("ann과 bob은 2020년에 입사했습니다. 입력은 오늘 하지만, 2020년부터라고 적습니다.", read=False)
    d.run(dated("2020-01-01",
                "CREATE (c:Company {name: 'acme'})\n"
                "  CREATE (a:Person {name: 'ann', salary: 100})-[:WORKS_AT {title: 'engineer'}]->(c)\n"
                "  CREATE (b:Person {name: 'bob', salary: 80})-[:WORKS_AT {title: 'designer'}]->(c)\n"
                "  RETURN a.name AS hired, b.name AS also"), expect=["ann", "bob"], look=2.0)
    d.caption("2023년 1월부터 ann의 연봉이 200으로 올랐습니다. 세상이 바뀐 것, 변경입니다.", read=False)
    d.run(dated("2023-01-01", "MATCH (p:Person {name: 'ann'}) SET p.salary = 200\n  RETURN p.name AS name, p.salary AS salary"),
          expect="200", look=2.4)

    d.scene("report, then correction")
    d.clear()
    d.caption("이 숫자로 감사 보고서를 제출합니다. 2023년 ann의 연봉은 200.", read=False)
    d.run("CREATE (r:Report {title: '2023 급여 감사', ann_salary: 200})\nRETURN r.title AS filed",
          expect="2023 급여 감사", look=2.4)

    d.caption("그런데 인상액을 잘못 적었습니다. 250이었습니다. 세상은 그대로이고 우리가 틀렸던 것, 정정입니다.", read=False)
    d.run(dated("-infinity", "MATCH (p:Person {name: 'ann'}) SET p.salary = 250\n  RETURN p.name AS name, p.salary AS salary"),
          expect="250", look=2.0)
    d.caption("-infinity는 언제부터인지를 새로 정하지 않는다는 뜻입니다. 유효 구간은 그대로 두고 값만 고칩니다.")

    d.scene("a future change")
    d.clear()
    d.caption("bob의 연봉은 2030년부터 120이 됩니다. 아직 오지 않은 변경도 오늘 기록합니다.", read=False)
    d.run(dated("2030-01-01", "MATCH (p:Person {name: 'bob'}) SET p.salary = 120\n  RETURN p.name AS name, p.salary AS salary"),
          expect="120", look=2.0)

    d.scene("the question, now")
    d.clear()
    d.caption("이제 질문은 하나뿐입니다. 누가, 어떤 직함으로, 얼마를 받는가.", read=False)
    ask("now", expect=["ann", "250", "bob", "80"], absent=["120", "200"])
    d.caption("지금의 답입니다. ann은 250, bob은 아직 80입니다. 2030년의 인상은 오늘의 답을 바꾸지 않습니다.")

    d.scene("valid time")
    page.click("#time-btn")
    page.wait_for_selector("#time-bar:not(.hidden)")
    d.caption("시계 버튼이 시간 여행 막대를 엽니다. valid at에 2021년 6월을 넣고, 같은 질의를 그대로 다시 실행합니다.", read=False)
    valid_at("2021-06-01")
    ask("valid 2021-06-01", expect=["ann", "100", "bob", "80"], absent=["250", "200"], badge="valid 2021-06-01")
    d.caption("2021년의 ann은 인상 전이라 100입니다. 질의는 한 글자도 바뀌지 않았습니다.")
    valid_at("2023-06-01")
    ask("valid 2023-06-01", expect=["ann", "250"], absent=["200", "100"], badge="valid 2023-06-01")
    d.caption("2023년 6월은 250입니다. 지금 아는 바로는, 200이었던 날은 없습니다.")

    d.scene("system time")
    d.clear()
    d.caption("그러면 감사 보고서의 200은 어디로 갔을까요? as known at에서 보고서를 제출한 순간을 고릅니다.", read=False)
    page.wait_for_function("() => [...document.querySelectorAll('#tb-system option')].some(o => o.textContent.includes('Report'))")
    report = page.evaluate("() => [...document.querySelectorAll('#tb-system option')].find(o => o.textContent.includes('Report')).value")
    page.select_option("#tb-system", report)
    d.hold(1.6)
    ask("valid 2023-06-01, known at the report", expect=["ann", "200"], absent=["250", "100"], badge="known", look=2.6)
    d.caption("그때 우리가 알던 2023년 6월입니다. ann은 200. 정정은 과거의 믿음을 지우지 않았습니다.")
    d.caption("변경은 유효 시간에, 정정은 시스템 시간에 남습니다. 두 축이 따로 있어야 둘을 구분할 수 있습니다.")

    d.scene("the future")
    d.clear()
    page.click("#tb-clear")
    d.caption("막대를 비우고, 이번에는 미래로 갑니다. 2030년 6월.", read=False)
    valid_at("2030-06-01")
    ask("valid 2030-06-01", expect=["bob", "120", "ann", "250"], absent=["80"], badge="valid 2030-06-01")
    d.caption("bob의 인상이 효력을 가진 뒤의 답입니다. 120.")

    d.scene("same sql")
    fps = {fp for _, fp in prints}
    if len(prints) != 5 or len(fps) != 1 or not next(iter(fps)):
        raise Stop(f"the compiled SQL fingerprint should be one value across the five asks: {prints}")
    d.caption("같은 질의를 다섯 번 실행했고 답은 시점마다 달랐습니다. 그런데 결과 아래의 compiled SQL 지문은 매번 같았습니다.", read=False)
    frame = page.locator(".frame").first
    frame.locator(".tab", has_text="SQL").click()
    sql = frame.locator(".body").inner_text()
    if "og_valid_at()" not in sql or "2030" in sql:
        raise Stop(f"the SQL tab should read the instant from the session, not carry a date:\n{sql[:400]}")
    d.hold(3.0)
    d.caption("SQL에는 날짜가 없습니다. 시점은 세션 설정이고, 타입 뷰가 그것을 읽습니다. 플랜은 하나입니다.")

    d.scene("refusal")
    d.clear()
    valid_at("2021-06-01")
    d.caption("시점을 답할 수 없는 질의 모양도 있습니다. 가변 길이 경로를 2021년 기준으로 물어 봅니다.", read=False)
    d.run("MATCH (p:Person)-[:WORKS_AT*1..2]->(c:Company)\nRETURN p.name AS name", error=True,
          expect="OG_TEMPORAL_UNSUPPORTED", look=2.4)
    d.caption("오늘 데이터로 그럴듯하게 답하는 대신 거부합니다. 틀린 과거보다 오류가 낫습니다.")

    d.scene("end")
    d.card("<h1>질의는 그대로,<br><em>바뀌는 것은 시점뿐</em>입니다</h1>"
           "<div class='row'><span class='chip'><b>og.valid_at</b> 그때 실제로</span>"
           "<span class='chip'><b>og.system_at</b> 그때 알던 대로</span>"
           "<span class='chip'><b>og.valid_from</b> 언제부터</span></div>"
           "<p>타입마다 켜는 이중 시간 · 변경과 정정의 구분 · 시점과 무관한 컴파일된 SQL<br>"
           "가변 길이 경로·라벨 없는 스캔·TypeQL의 시점 질의와 이력 보존 정책은 아직 없습니다</p>", 8,
           "질의는 그대로, 바뀌는 것은 시점뿐입니다. 변경과 정정을 구분하고, 시점과 무관하게 같은 SQL로 답합니다.")
    d.scene(None)


if __name__ == "__main__":
    record("temporal", story, setup_sql=SETUP)
