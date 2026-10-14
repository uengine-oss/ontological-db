#!/usr/bin/env python3
"""The built-in security demo video. See ../demo_lib.py for how a demo is recorded.

    FAST=1 TUT_STUDIO=http://127.0.0.1:7482 python3 demo.py   # checks only, nothing kept
    TUT_STUDIO=http://127.0.0.1:7482 python3 demo.py          # → docs/tutorials/media/security-demo.mp4

Needs a Studio running with PGDATABASE=demo_security and OG_STUDIO_ALLOW_RAW_SQL=1.

The Studio connects as the superuser, and a superuser bypasses row-level
security. To ask "as the nurse" from the editor, the setup below defines two
small helpers that do SET LOCAL ROLE, run the statement, and RESET ROLE — in one
SQL statement, so the pooled connection never keeps the identity:

    cypher_as(role, cypher)                         → og_cypher('hospital', cypher) as that role
    action_as(role, action, params, actor, reason)  → og_run_action('hospital', …) as that role

They add no privilege: everything they run is checked against the role named.
Roles are cluster-wide; the demo's are named tutsec_demo_* so that they do not
collide with the tutorial's (tutsec_*), whose database may hold grants to them.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import Demo, Stop, record

SETUP = r"""
DROP ROLE IF EXISTS tutsec_demo_doctor, tutsec_demo_nurse, tutsec_demo_agent, tutsec_demo_pharmacy;
CREATE ROLE tutsec_demo_doctor;  CREATE ROLE tutsec_demo_nurse;
CREATE ROLE tutsec_demo_agent;   CREATE ROLE tutsec_demo_pharmacy;

CREATE FUNCTION cypher_as(who text, q text) RETURNS SETOF jsonb LANGUAGE plpgsql AS $f$
BEGIN
    EXECUTE format('SET LOCAL ROLE %I', who);
    RETURN QUERY SELECT og_cypher('hospital', q);
    RESET ROLE;
END $f$;

CREATE FUNCTION action_as(who text, action text, params jsonb, actor text DEFAULT NULL, reason text DEFAULT NULL)
RETURNS SETOF jsonb LANGUAGE plpgsql AS $f$
BEGIN
    PERFORM set_config('og.actor', coalesce(actor, ''), true), set_config('og.reason', coalesce(reason, ''), true);
    EXECUTE format('SET LOCAL ROLE %I', who);
    RETURN QUERY SELECT og_run_action('hospital', action, params);
    RESET ROLE;
END $f$;
"""

PATH = "MATCH (d:Doctor)<-[:TREATED_BY]-(p:Patient)-[:PRESCRIBED]->(m:Drug)"


def absent(text, word, what):
    if word in text:
        raise Stop(f"{what}: '{word}' should not be on screen:\n{text[:300]}")


def story(d: Demo):
    page = d.page

    d.scene("title")
    d.card("<h1>권한은 애플리케이션이 아니라<br><em>데이터베이스가 지킵니다</em></h1>"
           "<p>작은 병원의 그래프입니다. 의사, 간호사, 그리고 처방을 돕는 AI 에이전트가<br>같은 데이터를 서로 다른 권한으로 봅니다.</p>"
           "<div class='row'><span class='chip'>의사 · <b>doctor</b></span><span class='chip'>간호사 · <b>nurse</b></span>"
           "<span class='chip'>에이전트 · <b>agent</b></span></div>", 7,
           "권한은 애플리케이션이 아니라 데이터베이스가 지킵니다. 의사, 간호사, AI 에이전트가 같은 병원 그래프를 서로 다른 권한으로 봅니다.")

    d.scene("hospital")
    d.caption("병원 그래프를 만듭니다. 환자 셋, 의사 둘, 약 셋입니다.", read=False)
    d.run("CREATE GRAPH hospital", expect="graph hospital created", look=1.2)
    page.reload()
    page.wait_for_selector("#graph-select option", state="attached")
    page.select_option("#graph-select", "hospital")
    d.overlay()
    d.run("CREATE (kim:Doctor {name: '김내과'}), (seo:Doctor {name: '서정신'}),\n"
          "  (amx:Drug {name: '아목시실린'}), (zol:Drug {name: '졸피뎀'}), (met:Drug {name: '메트포르민'}),\n"
          "  (p1:Patient {name: '한지민', ward: '내과', room: '301', diagnosis: '폐렴'}),\n"
          "  (p2:Patient {name: '오세훈', ward: '내과', room: '302', diagnosis: '당뇨'}),\n"
          "  (p3:Patient {name: '윤하늘', ward: '정신건강', room: '501', diagnosis: '불면증'}),\n"
          "  (p1)-[:TREATED_BY]->(kim), (p2)-[:TREATED_BY]->(kim), (p3)-[:TREATED_BY]->(seo),\n"
          "  (p1)-[:PRESCRIBED]->(amx), (p2)-[:PRESCRIBED]->(met), (p3)-[:PRESCRIBED]->(zol),\n"
          "  (p2)-[:ALLERGIC_TO]->(amx)", look=0.8)
    d.run(f"{PATH}\nRETURN d, p, m", look=3.0)
    if "Graph" not in " ".join(page.locator(".frame").first.locator(".tab").all_inner_texts()):
        raise Stop("the result of the node query has no Graph tab")
    d.caption("윤하늘 환자는 정신건강 병동에 있습니다. 이 환자의 기록은 의사만 봐야 합니다.")

    d.scene("default deny")
    d.clear()
    d.caption("사용자는 PostgreSQL 역할입니다. 간호사 역할은 만들어져 있지만 아직 받은 권한이 없습니다.", read=False)
    d.run(":sql SELECT * FROM cypher_as('tutsec_demo_nurse', 'MATCH (p:Patient) RETURN p.name AS name')",
          error=True, expect="permission denied for function og_cypher", look=1.0)
    d.caption("아무것도 주지 않으면 질의 함수조차 부를 수 없습니다. 기본은 거부입니다.")
    d.caption("의사에게는 쓰기까지, 간호사와 에이전트에게는 읽기 권한을 줍니다.", read=False)
    d.run(":sql SELECT og_grant('tutsec_demo_doctor', 'write'), og_grant('tutsec_demo_nurse', 'read'),\n"
          "  og_grant('tutsec_demo_agent', 'read'), og_grant('tutsec_demo_pharmacy', 'read')", look=1.6)

    d.scene("row-level security")
    d.clear()
    d.caption("정신건강 병동 환자는 의사에게만 보이도록 정책을 겁니다. PostgreSQL의 행 수준 보안입니다.", read=False)
    d.run(":sql SELECT og_enable_rls('hospital', 'Patient',\n"
          "  $$p_ward <> '정신건강' OR current_user = 'tutsec_demo_doctor'$$)", look=1.6)
    d.caption("간호사로 환자 목록을 묻습니다.", read=False)
    t = d.run(":sql SELECT r->>'name' AS name, r->>'ward' AS ward\n"
              "FROM cypher_as('tutsec_demo_nurse', 'MATCH (p:Patient) RETURN p.name AS name, p.ward AS ward') r",
              expect=["한지민", "오세훈"], look=1.0)
    absent(t, "윤하늘", "the nurse's patient list")
    d.caption("두 명뿐입니다. 오류가 아니라, 윤하늘 환자는 간호사에게 없는 행입니다.")

    d.scene("mid-traversal")
    d.clear()
    d.caption("이번에는 환자를 묻지 않습니다. 어느 의사가 어떤 약을 처방했는지만 묻습니다. 먼저 의사로.", read=False)
    d.run(":sql SELECT r->>'doctor' AS doctor, r->>'drug' AS drug\n"
          f"FROM cypher_as('tutsec_demo_doctor', '{PATH}\n  RETURN d.name AS doctor, m.name AS drug') r",
          expect=["김내과", "서정신", "졸피뎀"], look=2.2)
    d.caption("같은 질의를 간호사로 실행합니다.", read=False)
    t = d.run(":sql SELECT r->>'doctor' AS doctor, r->>'drug' AS drug\n"
              f"FROM cypher_as('tutsec_demo_nurse', '{PATH}\n  RETURN d.name AS doctor, m.name AS drug') r",
              expect=["김내과", "아목시실린", "메트포르민"], look=1.0)
    absent(t, "졸피뎀", "the nurse's traversal")
    d.caption("서정신 의사도 졸피뎀도 간호사가 볼 수 있는 노드입니다. 하지만 둘을 잇는 환자가 보이지 않아, 경로가 통째로 사라졌습니다.")
    d.caption("결과에 환자가 없어도, 순회의 중간에서 권한이 적용됩니다.")

    d.scene("write grants")
    d.clear()
    d.caption("쓰기는 타입, 연산, 속성 단위로 엽니다. 간호사는 환자의 병실만 바꿀 수 있습니다.", read=False)
    d.run(":sql SELECT og_grant_write('tutsec_demo_nurse', 'hospital', 'Patient', ARRAY['set'], ARRAY['room'])", look=1.2)
    d.run(":sql SELECT r->>'name' AS name, r->>'room' AS room FROM cypher_as('tutsec_demo_nurse',\n"
          "  'MATCH (p:Patient {name: \"한지민\"}) SET p.room = \"305\" RETURN p.name AS name, p.room AS room') r",
          expect=["한지민", "305"], look=1.6)
    d.caption("병실 이동은 됩니다. 진단을 고쳐 봅니다.", read=False)
    d.run(":sql SELECT * FROM cypher_as('tutsec_demo_nurse',\n"
          "  'MATCH (p:Patient {name: \"한지민\"}) SET p.diagnosis = \"완치\"')",
          error=True, expect=["OG_WRITE_DENIED", "diagnosis"], look=1.0)
    d.caption("거부됩니다. 이 권한의 실체는 PostgreSQL의 컬럼 GRANT입니다.", read=False)
    d.run(":sql SELECT grantee, privilege_type, column_name FROM information_schema.column_privileges\n"
          "WHERE grantee = 'tutsec_demo_nurse' AND privilege_type = 'UPDATE' AND table_schema = 'og_data'",
          expect=["UPDATE", "p_room"], look=1.0)
    d.caption("그래서 사이퍼를 거치지 않고 SQL로 직접 들어와도 같은 답을 받습니다.")

    d.scene("action")
    d.clear()
    d.caption("AI 에이전트는 읽기만 합니다. 대신 '처방'이라는 행위 하나를 정의해서 줍니다.", read=False)
    d.run(":sql SELECT og_grant_write('tutsec_demo_pharmacy', 'hospital', 'PRESCRIBED', ARRAY['create'])", look=0.6)
    d.run(":sql SELECT og_define_action('hospital', 'prescribe',\n"
          "  'MATCH (p:Patient {name: $patient}), (m:Drug {name: $drug}) CREATE (p)-[:PRESCRIBED]->(m) RETURN p.name AS patient, m.name AS drug',\n"
          "  'tutsec_demo_pharmacy',\n"
          "  params => '{\"patient\": \"string\", \"drug\": \"string\"}',\n"
          "  precondition => 'MATCH (p:Patient {name: $patient})-[:ALLERGIC_TO]->(m:Drug {name: $drug}) RETURN count(p) = 0 AS ok')",
          look=2.4)
    d.caption("파라미터는 환자와 약, 둘뿐입니다. 전제 조건은 '그 약에 알레르기가 없을 것'입니다.")
    d.run(":sql SELECT og_grant_action('tutsec_demo_agent', 'hospital', 'prescribe')", look=0.8)

    d.scene("agent")
    d.clear()
    d.caption("에이전트가 사이퍼로 직접 처방을 만들려 하면 거부됩니다.", read=False)
    d.run(":sql SELECT * FROM cypher_as('tutsec_demo_agent',\n"
          "  'MATCH (p:Patient {name: \"한지민\"}), (m:Drug {name: \"메트포르민\"}) CREATE (p)-[:PRESCRIBED]->(m)')",
          error=True, expect=["OG_WRITE_DENIED", "PRESCRIBED"], look=2.0)
    d.caption("행위로 실행하면 됩니다. 누구를 대신하는지, 왜 하는지도 함께 보냅니다.", read=False)
    d.run(":sql SELECT * FROM action_as('tutsec_demo_agent', 'prescribe',\n"
          "  '{\"patient\": \"한지민\", \"drug\": \"메트포르민\"}', 'rx-assistant', '야간 처방 보조')",
          expect=["한지민", "메트포르민"], look=2.2)
    d.caption("오세훈 환자는 아목시실린 알레르기가 있습니다. 같은 행위로 처방해 봅니다.", read=False)
    d.run(":sql SELECT * FROM action_as('tutsec_demo_agent', 'prescribe',\n"
          "  '{\"patient\": \"오세훈\", \"drug\": \"아목시실린\"}', 'rx-assistant', '야간 처방 보조')",
          error=True, expect="OG_ACTION_REJECTED", look=1.0)
    d.caption("전제 조건이 막았습니다. 에이전트가 무엇을 만들어 내든, 할 수 있는 일은 정해진 행위뿐입니다.")

    d.scene("audit")
    d.clear()
    d.caption("누가, 누구를 대신해, 왜 했는지는 감사 기록에 남습니다.", read=False)
    d.run(":sql SELECT action, db_user, actor, reason, rows_out FROM og_data.og_audit\n"
          "WHERE action = 'prescribe' AND rows_out = 1 AND query LIKE '%CREATE%'",
          expect=["prescribe", "tutsec_demo_pharmacy", "rx-assistant", "야간 처방 보조"], look=1.0)
    d.caption("실제로 쓴 역할은 데이터베이스가 기록한 값이라, 호출한 쪽이 바꿀 수 없습니다.")
    page.click('.rail-btn[data-panel="audit"]')
    page.wait_for_selector("#audit-list :first-child", timeout=5000)
    d.hold(1.2)
    d.caption("Studio의 질의 기록도 같은 감사 로그를 읽습니다. 쓰는 역할은 이 기록을 고치거나 지울 수 없습니다.")
    page.click('.rail-btn[data-panel="audit"]')

    d.scene("limit")
    d.clear()
    d.caption("한계도 있습니다. 간호사가 중간 노드의 라벨을 빼고 물으면 어떻게 될까요?", read=False)
    t = d.run(":sql SELECT r->>'doctor' AS doctor, r->>'patient' AS patient, r->>'drug' AS drug\n"
              "FROM cypher_as('tutsec_demo_nurse', 'MATCH (d:Doctor)<-[:TREATED_BY]-(p)-[:PRESCRIBED]->(m:Drug)\n"
              "  RETURN DISTINCT d.name AS doctor, p.name AS patient, m.name AS drug') r",
              expect=["서정신", "졸피뎀"], look=1.0)
    absent(t, "윤하늘", "the nurse's unlabelled traversal")
    d.caption("환자의 이름은 여전히 가려지지만, 연결이 있다는 사실은 보입니다. 노드와 간선의 목록 자체는 아직 읽기 제한이 없습니다.")

    d.scene("end")
    d.card("<h1>신원도, 정책도, 권한도<br><em>PostgreSQL의 것</em>입니다</h1>"
           "<div class='row'><span class='chip'><b>og_grant</b> 기본 거부</span><span class='chip'><b>og_enable_rls</b> 순회 중간까지</span>"
           "<span class='chip'><b>og_grant_write</b> 속성 단위</span><span class='chip'><b>og_define_action</b> 정해진 행위</span></div>"
           "<p>사이퍼 · SQL · Bolt 어느 통로로 들어와도 같은 규칙 · 누가 왜 했는지는 감사 기록에</p>", 7,
           "신원도, 정책도, 권한도 PostgreSQL의 것입니다. 어느 통로로 들어와도 같은 규칙이 적용되고, 누가 왜 했는지는 감사 기록에 남습니다.")
    d.scene(None)


if __name__ == "__main__":
    record("security", story, setup_sql=SETUP)
