#!/usr/bin/env python3
"""The three-languages demo video. See ../demo_lib.py for how a demo is recorded.

    python3 demo.py            # → docs/tutorials/media/three-languages-demo.mp4
    FAST=1 python3 demo.py     # same checks, short holds, no file kept

Needs a Studio running with PGDATABASE=demo_three_languages and OG_STUDIO_ALLOW_RAW_SQL=1
(RDF and TypeQL are SQL functions, so the Studio reaches them with `:sql`).
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import Demo, Stop, record

TURTLE = """@prefix ex: <http://example.org/fleet#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Vehicle a owl:Class .
ex:Truck a owl:Class ; rdfs:subClassOf ex:Vehicle .
ex:Van a owl:Class ; rdfs:subClassOf ex:Vehicle .
ex:ElectricTruck a owl:Class ; rdfs:subClassOf ex:Truck .
ex:Driver a owl:Class .
ex:drives a owl:ObjectProperty ; rdfs:domain ex:Driver ; rdfs:range ex:Vehicle .
ex:v1 a ex:Truck ; ex:plate "81바1001" .
ex:v2 a ex:ElectricTruck ; ex:plate "81바1002" .
ex:v3 a ex:Van ; ex:plate "81바1003" .
ex:d1 a ex:Driver ; ex:name "김민수" .
ex:d2 a ex:Driver ; ex:name "이서연" .
ex:d1 ex:drives ex:v1 .
ex:d2 ex:drives ex:v2 .
ex:d2 ex:memberOf ex:union7 ."""


def typeql(query, alias="answer"):
    return f":sql SELECT og_typeql('fleet', $$\n{query}\n$$) AS {alias}"


def graph_tab(d, what):
    if "Graph" not in " ".join(d.page.locator(".frame").first.locator(".tab").all_inner_texts()):
        raise Stop(f"the result of {what} has no Graph tab")


def story(d: Demo):
    page = d.page

    d.scene("title")
    d.card("<h1>세 언어,<br><em>하나의 그래프</em></h1>"
           "<p>표준 담당은 OWL로, 배차 앱은 Cypher로, 정비팀은 TypeQL로.<br>한빛물류의 세 팀이 같은 차량 데이터를 씁니다.</p>"
           "<div class='row'><span class='chip'><b>OWL</b> / RDF</span><span class='chip'><b>Cypher</b></span>"
           "<span class='chip'><b>TypeQL</b></span></div>", 7,
           "세 언어, 하나의 그래프. 표준 담당은 OWL로, 배차 앱은 Cypher로, 정비팀은 TypeQL로 같은 차량 데이터를 씁니다.")

    d.scene("ontology arrives")
    d.caption("빈 데이터베이스에 그래프를 하나 만듭니다.", read=False)
    d.run("CREATE GRAPH fleet", expect="graph fleet created", look=1.6)
    page.reload()   # the graph list is read when the page loads
    page.wait_for_selector("#graph-select option", state="attached")
    page.select_option("#graph-select", "fleet")
    d.overlay()
    d.caption("표준 담당이 보낸 온톨로지입니다. Turtle 파일을 그대로 넣습니다.", read=False)
    d.run(":sql SELECT r->>'classes' AS classes, r->>'subclass_axioms' AS subclass_of, r->>'instances' AS instances,\n"
          "  r->>'facts' AS facts, r->>'unmapped' AS unmapped\nFROM og_load_rdf('fleet', $rdf$\n" + TURTLE + "\n$rdf$) AS r",
          expect="5\t3\t5\t7\t1", look=2.4)
    d.caption("클래스 다섯, 하위 클래스 관계 셋, 인스턴스 다섯. 매핑하지 못한 트리플 하나는 버리지 않고 따로 보관합니다.")

    d.scene("classes are types")
    d.clear()
    d.caption("OWL 클래스는 어디로 갔을까요? 타입 카탈로그를 봅니다.", read=False)
    d.run(":sql SELECT name, kind, parents, iri FROM og_type_view WHERE graph = 'fleet' ORDER BY lft",
          expect=["ElectricTruck", '["Truck"]', "http://example.org/fleet#Vehicle"], look=2.6)
    d.caption("subClassOf가 상속이 되었습니다. 트리플 저장소가 따로 있는 것이 아니라, 이 데이터베이스의 타입입니다.")

    d.scene("cypher reads")
    d.clear()
    d.caption("배차 앱은 Cypher로 묻습니다. 차량 전체를 달라고 해 봅니다.", read=False)
    d.run("MATCH (v:Vehicle)\nRETURN v.plate AS plate, labels(v) AS labels\nORDER BY plate",
          expect=["81바1001", "81바1002", "81바1003", "ElectricTruck"], look=2.6)
    d.caption("Vehicle의 인스턴스는 하나도 없습니다. 트럭, 전기 트럭, 밴이 Vehicle이라는 라벨에 답했습니다.")
    d.run("MATCH (d:Driver)-[r:drives]->(v:Vehicle)\nRETURN d, r, v", look=2.6)
    graph_tab(d, "the drives query")
    d.caption("RDF로 들어온 기사와 차량, 그 사이의 관계입니다.")

    d.scene("cypher writes")
    d.clear()
    d.caption("김민수 기사에게 새 밴을 배정합니다.", read=False)
    d.run("MATCH (d:Driver {name: '김민수'})\nCREATE (v:Van {plate: '81바1004'})\nCREATE (d)-[:drives]->(v)\n"
          "RETURN d.name AS driver, v.plate AS plate", expect=["김민수", "81바1004"], look=1.8)
    d.caption("그럼 기사가 기사를 운전한다고 쓰면 어떻게 될까요?", read=False)
    d.run("MATCH (a:Driver {name: '김민수'}), (b:Driver {name: '이서연'})\nCREATE (a)-[:drives]->(b)",
          error=True, expect="requires a 'Vehicle'", look=1.2)
    d.caption("거부됩니다. 온톨로지의 domain과 range가 Cypher의 쓰기에도 적용됩니다.")

    d.scene("typeql reads")
    d.clear()
    d.caption("정비팀은 TypeDB를 쓰던 팀입니다. 같은 그래프의 스키마를 TypeQL로 뽑아 봅니다.", read=False)
    d.run(":sql SELECT og_typeql_schema('fleet') AS schema",
          expect=["entity ElectricTruck sub Truck", "entity Truck sub Vehicle"], look=2.6)
    d.caption("아무것도 정의하지 않았는데, OWL에서 온 계층이 TypeQL 스키마로 나옵니다.")
    d.run(typeql("match $v isa Vehicle; reduce $n = count;"), expect="4", look=2.0)
    d.caption("넷입니다. RDF로 들어온 세 대에, 방금 Cypher로 만든 밴까지 TypeQL이 셉니다.")

    d.scene("typeql writes")
    d.clear()
    d.caption("정비팀이 자기 스키마를 얹습니다. 차대번호와 차고, 그리고 역할이 셋인 점검 관계입니다.", read=False)
    d.run(typeql("define\n"
                 "  attribute code @abstract, value string;\n"
                 "  attribute vin sub code;\n"
                 "  attribute staff-no sub code;\n"
                 "  attribute depot, value string;\n"
                 "  attribute result, value string @values(\"pass\", \"fail\");\n"
                 "  entity Vehicle, owns vin @key, owns depot, plays inspection:subject;\n"
                 "  entity Driver, owns staff-no @key, plays inspection:witness;\n"
                 "  entity Inspector, owns staff-no @key, plays inspection:inspector;\n"
                 "  relation inspection, relates subject, relates inspector, relates witness, owns result;"),
          expect="defined", look=1.6)
    d.caption("전기 트럭과 밴, 점검자와 입회 기사를 넣고, 점검 한 건을 기록합니다.", read=False)
    d.run(typeql("insert\n"
                 "  $e isa ElectricTruck, has vin \"KMF-E-0005\", has depot \"인천\";\n"
                 "  $n isa Van, has vin \"KMF-V-0006\", has depot \"인천\";\n"
                 "  $i isa Inspector, has staff-no \"Q-17\";\n"
                 "  $d isa Driver, has staff-no \"D-03\";"), expect="operations", look=1.0)
    d.run(typeql("match\n"
                 "  $v isa Vehicle, has vin \"KMF-E-0005\";\n"
                 "  $i isa Inspector, has staff-no \"Q-17\";\n"
                 "  $d isa Driver, has staff-no \"D-03\";\n"
                 "insert (subject: $v, inspector: $i, witness: $d) isa inspection, has result \"pass\";"),
          expect="operations", look=1.8)

    d.scene("cypher reads typeql")
    d.clear()
    d.caption("이제 배차 앱으로 돌아옵니다. 차량은 몇 대일까요?", read=False)
    d.run("MATCH (v:Vehicle) RETURN count(v) AS vehicles", expect="6", look=1.8)
    d.caption("여섯 대. RDF 셋, Cypher 하나, TypeQL 둘입니다. 누가 넣었는지는 묻지 않았습니다.")
    d.caption("TypeQL의 속성은 Cypher에서 노드로 보입니다. 같은 차고를 쓰는 차량을 패턴으로 찾아봅니다.", read=False)
    d.run("MATCH (a:Vehicle)-[:`$has`]->(d:depot)<-[:`$has`]-(b:Vehicle),\n"
          "      (a)-[:`$has`]->(x:vin), (b)-[:`$has`]->(y:vin)\nWHERE x.val < y.val\n"
          "RETURN d.val AS depot, x.val AS vehicle_a, y.val AS vehicle_b",
          expect=["인천", "KMF-E-0005", "KMF-V-0006"], look=2.6)
    d.caption("두 차량이 인천이라는 같은 노드를 가리킵니다. TypeQL의 속성은 값마다 하나이기 때문입니다.")

    d.scene("cypher extends typeql")
    d.clear()
    d.caption("차대번호로 그 전기 트럭을 찾아 번호판을 달고, 이서연 기사에게 배정합니다.", read=False)
    d.run("MATCH (d:Driver {name: '이서연'}), (v:Vehicle)-[:`$has`]->(:vin {val: 'KMF-E-0005'})\n"
          "SET v.plate = '81바1005'\nCREATE (d)-[:drives]->(v)", look=1.2)
    d.run("MATCH (d:Driver)-[:drives]->(v:Truck)-[:`$has`]->(n:vin)\n"
          "RETURN d.name AS driver, v.plate AS plate, n.val AS vin, labels(v) AS labels",
          expect=["이서연", "81바1005", "KMF-E-0005", "ElectricTruck"], look=2.4)
    d.caption("TypeQL이 만든 차량에 Cypher의 번호판과 배차가 붙었습니다. 같은 노드입니다.")

    d.scene("typeql answers")
    d.clear()
    d.caption("점검 기록은 TypeQL로 묻습니다. 차량, 점검자, 입회 기사가 한 관계에 있습니다.", read=False)
    d.run(typeql("match $r isa inspection, links (subject: $v, inspector: $i, witness: $w), has result $res;\n"
                 "fetch { \"vin\": $v.vin, \"inspector\": $i.staff-no, \"witness\": $w.staff-no, \"result\": $res };"),
          expect=["KMF-E-0005", "Q-17", "D-03", "pass"], look=2.6)
    d.caption("경계도 있습니다. Cypher가 쓴 번호판은 프로퍼티라서, TypeQL의 has로는 읽지 못합니다.", read=False)
    d.run(typeql("match $v isa Vehicle, has plate $p;"), error=True, expect="type 'plate' is not defined", look=1.2)
    d.caption("조용히 빈 결과를 주지 않고, 그런 속성 타입이 없다고 말합니다.")

    d.scene("back to rdf")
    d.clear()
    d.caption("마지막으로 표준 담당에게 돌려줍니다. 그래프를 다시 RDF로 내보냅니다.", read=False)
    d.run(":sql SELECT line FROM regexp_split_to_table(og_dump_rdf('fleet', 'ntriples'), E'\\n') AS line\n"
          "WHERE line LIKE '%fleet#v2>%' OR line LIKE '%memberOf%'",
          expect=["fleet#ElectricTruck", "fleet#drives", "fleet#memberOf", "urn:og:prop:plate"], look=3.0)
    d.caption("클래스와 관계의 IRI는 그대로이고, 처음에 매핑하지 못한 트리플도 다시 나옵니다. 질의는 Cypher와 TypeQL로 합니다. SPARQL은 아직 없습니다.")

    d.scene("end")
    d.card("<h1>온톨로지는 OWL로 들어오고<br><em>같은 그래프가 Cypher와 TypeQL에 답합니다</em></h1>"
           "<div class='row'><span class='chip'><b>og_load_rdf</b> · og_dump_rdf</span>"
           "<span class='chip'><b>og_cypher</b> · Bolt</span><span class='chip'><b>og_typeql</b></span></div>"
           "<p>타입 계층 하나 · 트랜잭션 하나 · 감사 로그 하나</p>", 7,
           "온톨로지는 OWL로 들어오고, 같은 그래프가 Cypher와 TypeQL에 답합니다. 타입 계층도, 트랜잭션도, 감사 로그도 하나입니다.")
    d.scene(None)


if __name__ == "__main__":
    record("three-languages", story)
