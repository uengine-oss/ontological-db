#!/usr/bin/env python3
"""Write the landing page's feature and video sections into web/index.html.

The copy lives here, as data, so a feature's claim, its limit, its example and
its links stay together. Every claim is one a tutorial demonstrates by running
it (examples/tutorials/*); every "아직" line is a limit the same tutorial shows.

    python3 web/landing_sections.py
"""
import html, re
from pathlib import Path

WEB = Path(__file__).resolve().parent
T = "docs/tutorials/"

FEATURES = [
 dict(id="languages", name="three-languages", tag="OWL · Cypher · TypeQL", status="working · partial",
  title="온톨로지는 <em>세 언어로</em> 들어오고, 그래프는 하나입니다",
  body="OWL/Turtle 파일이 별도의 트리플 저장소가 아니라 데이터베이스의 <b>타입 계층</b>이 됩니다. "
       "<code>rdfs:subClassOf</code>는 상속이, <code>domain</code>/<code>range</code>는 쓰기 시점에 검사되는 제약이 됩니다. "
       "그 위에서 같은 그래프가 Cypher와 TypeQL에 답합니다.",
  points=["<b>라벨 하나에 계층 전체가 답합니다.</b> <code>MATCH (v:Vehicle)</code>와 <code>match $v isa Vehicle</code>이 하위 타입까지 같은 수를 돌려줍니다",
          "한 언어로 넣은 것을 다른 언어가 바로 읽고, 두 언어가 <b>트랜잭션 하나와 감사 로그 하나</b>를 공유합니다",
          "Neo4j 드라이버는 주소만 바꿔 Bolt로 접속합니다. Neo4j의 MCP 서버가 수정 없이 동작합니다"],
  honest="SPARQL과 SHACL은 없습니다. TypeQL 사용자 정의 함수는 평가하지 않습니다. RDF 덤프는 무손실 왕복이 아닙니다.",
  bar=("한 그래프, 세 입구", "fleet"),
  code=[("-- OWL: 온톨로지가 타입 계층이 된다\nSELECT og_load_rdf('fleet', $rdf$\n  :Truck         rdfs:subClassOf :Vehicle .\n  :ElectricTruck rdfs:subClassOf :Truck .\n$rdf$);",
         "// Cypher: 배차 앱\nMATCH (v:Vehicle) RETURN count(v)      // 트럭·전기트럭 포함\n\n# TypeQL: 정비팀\nmatch $v isa Vehicle;                 # 같은 수")]),
 dict(id="fabric", name="virtual-fabric", tag="Data Fabric", status="proposed",
  title="다른 시스템의 데이터를 <em>복사 없이</em> 그래프로 묻습니다",
  body="ERP는 PostgreSQL에, 물류는 MySQL에 있어도 됩니다. 외부 테이블을 노드로, 시스템 사이의 키 대응을 관계로 한 번 선언하면 "
       "사이퍼 패턴 하나가 두 DB를 건넙니다. <b>조건과 같은 DB 안의 조인은 원천으로 내려가고</b>, 넘어오는 것은 필요한 행뿐입니다.",
  points=["그래프에 저장된 지식과 원천의 행을 <b>한 질의에서</b> 섞습니다. 순수한 연합 질의 엔진이 못 하는 일입니다",
          "<code>EXPLAIN FABRIC</code>이 각 원천에 실제로 보낸 SQL과 넘어온 행 수를 보여 줍니다",
          "<b>원천은 읽기 전용</b>입니다. 원천 계정에 쓰기 권한이 있어도 엔진이 먼저 거부합니다"],
  honest="제안 단계(ADR-029)입니다. 지원 원천은 PostgreSQL과 MySQL이고, 가상 관계의 가변 길이 순회와 원천 쓰기는 없습니다.",
  bar=("선언 셋, 질의 하나", "scm"),
  code=[("CREATE DATASOURCE wms WITH ENGINE = 'mysql',\n  PARAMETERS = {host: '…', database: 'wms', password: $pw}\nCREATE VIRTUAL NODE Shipment FROM wms.shipment KEY shipment_no\nCREATE VIRTUAL RELATIONSHIP SHIPPED_BY\n  FROM Shipment(vendor_code) TO Supplier(supplier_id)",
         "// 리스크는 그래프, 공급사는 ERP, 선적은 WMS\nMATCH (:Region {name: 'East Asia'})<-[:IN_REGION]-(e:RiskEvent)\n      -[:AFFECTS]->(s:Supplier)<-[:SHIPPED_BY]-(sh:Shipment)\nWHERE sh.ship_status <> 'DELIVERED'\nRETURN e.title, s.name, sh.shipment_no")]),
 dict(id="temporal", name="temporal", tag="Time-aware · Bitemporal", status="working · partial",
  title="<em>그때 실제로 어땠는지</em>와 <em>그때 무엇을 믿었는지</em>를 따로 묻습니다",
  body="타입마다 유효 시간과 시스템 시간을 함께 기록합니다. 세상이 바뀐 일(인상)과 잘못 적은 값을 고친 일(정정)이 구별되므로, "
       "\"2023년 6월의 실제 연봉\"과 \"감사 보고서를 낼 때 우리가 알던 2023년 6월의 연봉\"이 서로 다른 답으로 나옵니다.",
  points=["<b>질의는 고치지 않습니다.</b> 세션 설정만 바꾸면 같은 사이퍼가 다른 시점에 답하고, 컴파일된 SQL은 같은 문장입니다",
          "2030년부터 적용될 변경은 그날이 되기 전까지 오늘의 답을 바꾸지 않습니다",
          "시점을 답할 수 없는 질의는 오늘 데이터로 답하는 대신 <b>오류로 거부</b>합니다"],
  honest="가변 길이 경로·라벨 없는 스캔·TypeQL은 시점 질의를 거부합니다. 이력 보존 정책은 없습니다. 미래 날짜 변경 뒤에는 패턴에 라벨을 붙여야 합니다.",
  bar=("같은 질의, 다른 시점", "corp"),
  code=[("MATCH (p:Person {name: 'ann'}) RETURN p.salary",
         "SET og.valid_at  = '2023-06-01';          → 250   그때 실제로\nSET og.system_at = '<보고서를 낸 시각>';  → 200   그때 우리가 알던\nRESET …;                                   → 현재")]),
 dict(id="security", name="security", tag="Built-in security", status="working · partial",
  title="권한은 애플리케이션이 아니라 <em>데이터베이스의 것</em>입니다",
  body="사용자는 PostgreSQL 역할이고, Neo4j 드라이버로 접속해도 그 역할로 인증됩니다. 타입에 행 수준 보안을 걸면 "
       "그 타입을 지나는 패턴에서 <b>보이지 않는 노드는 조인에서 빠지고, 그 노드를 지나는 경로 전체가 사라집니다.</b>",
  points=["쓰기 권한은 타입·연산·속성 단위이고 실제로는 컬럼 GRANT입니다. <b>사이퍼든 원시 SQL이든 Bolt든 같은 거부</b>를 받습니다",
          "읽기만 가능한 AI 에이전트에게 파라미터 계약과 전제 조건이 붙은 <b>행위(Action)</b>만 실행하게 할 수 있습니다",
          "누가, 누구를 대신해, 왜 했는지가 감사 로그에 남고 쓰기 권한자도 고칠 수 없습니다"],
  honest="노드·간선 레지스트리의 읽기 제한은 아직 없어, 연결이 존재한다는 사실까지 숨기지는 못합니다. 라벨 없는 패턴에서는 경로가 남습니다.",
  bar=("같은 질의, 다른 역할", "hospital"),
  code=[("SELECT og_enable_rls('hospital', 'Patient',\n  $$p_ward <> '정신건강' OR current_user = 'doctor'$$);\nSELECT og_grant_write('nurse', 'hospital', 'Patient',\n  ARRAY['set'], ARRAY['room']);",
         "MATCH (d:Doctor)<-[:TREATED_BY]-(p:Patient)-[:PRESCRIBED]->(m:Drug)\nRETURN d.name, m.name\n\n의사   → 경로 3개\n간호사 → 경로 2개   가려진 환자를 지나는 경로가 없다")]),
 dict(id="agent", name="agent-native", tag="Agent-native", status="working · partial",
  title="에이전트가 <em>스스로 탐색하고, 틀리면 고치고, 실행 전에 가늠합니다</em>",
  body="처음 보는 그래프에서 에이전트가 필요로 하는 것을 데이터베이스가 직접 줍니다. 프롬프트에 넣을 크기로 줄인 스키마, "
       "실행하지 않고 받는 오류 코드와 위치, <b>틀리지 않았는데 0행인 이유</b>, 실행 전의 추정입니다.",
  points=["<code>og_schema</code>가 관계의 어느 쪽이 출발이고 도착인지까지 돌려주고, 예산을 주면 줄였다는 사실과 함께 줄여서 줍니다",
          "<code>og_diagnose_empty</code>가 패턴을 한 요소씩 세어 <b>어디서 끊겼는지</b>, 또는 WHERE가 전부 지웠는지를 짚습니다",
          "<code>og_estimate</code>가 관계 없이 나열된 노드의 카티션 곱을 실행 전에 경고합니다. 성공한 질의는 누가 왜 보냈는지와 함께 감사 로그에 남습니다"],
  honest="없는 라벨은 오류가 아니라 0행입니다. 스키마 예산은 토큰이 아니라 타입 개수로 줄입니다. 역할 제한 중 max_rows와 read_only는 동작하지 않으니 읽기 전용은 PostgreSQL 역할로 겁니다.",
  bar=("에이전트의 한 바퀴", "shop"),
  code=[("SELECT og_schema('shop', 4000);                -- 읽는다\nSELECT og_explain_error('shop', $$MATCH …$$);  -- 검사한다\nSELECT og_estimate('shop', $$MATCH …$$);       -- 가늠한다",
         "SELECT * FROM og_cypher('shop', $$MATCH …$$); -- 실행한다\nSELECT og_diagnose_empty('shop', $$MATCH …$$); -- 0행이면 이유를 묻는다\n\nStudio:  :schema   :explain <cypher>   :why <cypher>")]),
 dict(id="vector", name="vector-search", tag="Vector · Hybrid", status="working",
  title="임베딩은 노드뿐 아니라 <em>관계에도</em> 붙습니다",
  body="임베딩은 별도 벡터 저장소가 아니라 프로퍼티입니다. 그래서 관계에도 같은 방식으로 선언됩니다. "
       "\"이 문의가 <b>왜</b> 이 문서로 해결됐는가\" 같은 관계의 맥락을 의미로 찾을 수 있습니다.",
  points=["유사도 정렬이 그래프 패턴·프로퍼티 조건과 함께 SQL 한 문장이 됩니다. <b>조건은 top-k를 자르기 전에</b> 적용됩니다",
          "Neo4j와 같은 <code>CREATE VECTOR INDEX</code>와 <code>db.index.vector.queryNodes</code>를 받습니다",
          "하이브리드 검색은 벡터 순위와 그래프 거리를 RRF로 합치고 두 점수를 따로 돌려줍니다"],
  honest="임베딩 생성은 외부 모델의 몫입니다. 전문 검색과 벡터의 결합, 경로 임베딩은 없습니다. 하이브리드는 노드 타입만 됩니다.",
  bar=("관계를 의미로 찾는다", "support"),
  code=[("CREATE VECTOR INDEX article_emb FOR (a:Article) ON (a.emb)\nOPTIONS {indexConfig: {`vector.dimensions`: 3,\n         `vector.similarity_function`: 'cosine'}}",
         "MATCH (t:Ticket)-[r:RESOLVED_BY]->(a:Article)-[:ABOUT]->(p:Product {name: '계정'})\nWHERE a.status = 'published'\nRETURN t.title, a.title, r.note,\n       vector.similarity(r.note_emb, $q) AS score\nORDER BY score DESC LIMIT 5")]),
 dict(id="evolution", name="schema-evolution", tag="Schema evolution", status="working · partial",
  title="데이터가 들어 있는 그래프의 스키마를 <em>서비스 중에</em> 바꿉니다",
  body="스키마 변경은 plan → expand → migrate → validate → contract를 거치는 기록된 객체입니다. plan은 아무것도 바꾸지 않고 "
       "<b>변환할 수 없는 값이 몇 개이고 어느 노드인지</b> 먼저 알려 줍니다.",
  points=["타입 변경 중에도 읽기는 계속 답하고, 새 타입에 맞지 않는 쓰기는 사이퍼든 원시 SQL이든 그 순간부터 거부됩니다",
          "기존 행은 배치로 옮기고 중단한 곳에서 이어 갑니다. <b>이름 변경과 상위 타입 추가는 행을 한 줄도 다시 쓰지 않습니다</b>",
          "모든 변경이 스키마 버전과 함께 이력으로 남고, 지난 버전의 카탈로그를 조회할 수 있습니다"],
  honest="타입 분할·병합은 없습니다. TypeQL의 redefine은 이 절차와 연결되지 않습니다. 유니크 제약은 계층 전체가 아니라 테이블 단위입니다.",
  bar=("계획하고, 옮기고, 닫는다", "hr"),
  code=[("SELECT og_schema_plan('hr', '{\"kind\": \"alter_property_type\",\n  \"type\": \"Person\", \"property\": \"age\", \"to\": \"int\"}');\n-- report: 5행 중 1개 값이 변환되지 않음 · 노드 id",
         "og_schema_expand  → 이중 쓰기 시작, 읽기는 옛 컬럼\nog_schema_migrate → 배치로, 이어서\nog_schema_validate → 막히는 행의 id\nog_schema_contract → 새 타입으로 전환")]),
]

VIDEO_NOTE = {
 "three-languages": "OWL로 들어온 온톨로지를 Cypher와 TypeQL이 함께 읽고 씁니다",
 "virtual-fabric": "PostgreSQL과 MySQL의 데이터를 복사 없이 한 패턴으로",
 "temporal": "같은 질의를 다섯 번, 시점만 바꿔 묻습니다",
 "security": "같은 질의가 역할에 따라 다른 경로를 돌려줍니다",
 "agent-native": "에이전트가 스키마를 읽고, 틀리고, 고치는 과정",
 "vector-search": "문서 검색이 놓친 답을 관계 검색이 찾습니다",
 "schema-evolution": "운영 중인 그래프에서 잘못된 타입을 고칩니다",
}


def esc(s):
    return html.escape(s, quote=False)


def feature(i, f):
    code = "".join(f"<pre>{esc(c)}</pre>" for c in f["code"][0])
    points = "".join(f"<li>{p}</li>" for p in f["points"])
    name = f["name"]
    return f'''
    <div class="frow" id="f-{f["id"]}">
      <div>
        <span class="no">{i:02d} · {f["tag"]} · <span class="st st-{'ok' if f['status'] == 'working' else 'part'}" style="font-size:9.5px;padding:1px 7px">{f["status"]}</span></span>
        <h3>{f["title"]}</h3>
        <p>{f["body"]}</p>
        <ul>{points}</ul>
        <p class="honest"><b>아직</b> — {f["honest"]}</p>
        <div class="flinks">
          <a class="go" href="{T}{name}-tutorial.html">튜토리얼</a>
          <a href="#v-{f["id"]}">2분 영상</a>
          <a href="{T}{name}-tutorial.pdf">PDF</a>
        </div>
      </div>
      <div class="fpanel">
        <div class="bar"><span>{f["bar"][0]}</span><b>{f["bar"][1]}</b></div>
        {code}
      </div>
    </div>'''


features = f'''<section class="band" id="features">
  <div class="wrap band-inner">
    <div class="shead">
      <div class="rail">01 · 기능</div>
      <div>
        <h2>일곱 가지를 데이터베이스가 직접 합니다. 각각 실행으로 확인한 튜토리얼이 있습니다.</h2>
        <p>
          아래의 문장은 모두 튜토리얼에서 <b>실제로 실행해 보인 것</b>입니다. 튜토리얼의 예제는 릴리스 때마다 회귀 테스트로 다시 돕니다.
          되지 않는 것은 기능마다 "아직" 줄에 적었습니다.
        </p>
      </div>
    </div>
{"".join(feature(i, f) for i, f in enumerate(FEATURES, 1))}
  </div>
</section>'''

cards = "".join(f'''
      <div class="vcard" id="v-{f["id"]}">
        <video controls preload="none" poster="{T}media/{f["name"]}-demo.jpg" src="{T}media/{f["name"]}-demo.mp4"></video>
        <div class="vt"><b>{re.sub(r"<[^>]+>", "", f["title"])}</b>
          <span>{VIDEO_NOTE[f["name"]]}</span>
          <a href="{T}{f["name"]}-tutorial.html">튜토리얼로 따라 하기 →</a></div>
      </div>''' for f in FEATURES)
videos = f'''<section class="band" id="videos">
  <div class="wrap band-inner">
    <div class="shead">
      <div class="rail">02 · 영상</div>
      <div>
        <h2>2~3분씩, 빈 데이터베이스에서 시작해 실제로 실행한 화면입니다.</h2>
        <p>
          연출한 화면이 아닙니다. 영상 속의 문장은 녹화 중에 Studio 편집기에 입력해 실행한 것이고,
          화면이 자막의 내용과 다르면 녹화가 중단되도록 장면마다 검사했습니다. 설명은 자막으로 들어 있습니다.
        </p>
      </div>
    </div>
    <div class="vgrid">{cards}
    </div>
  </div>
</section>'''

page = WEB / "index.html"
s = page.read_text(encoding="utf-8")
s = re.sub(r"<!-- FEATURES:BEGIN -->.*?<!-- FEATURES:END -->", lambda m: f"<!-- FEATURES:BEGIN -->\n{features}\n<!-- FEATURES:END -->", s, flags=re.S)
s = re.sub(r"<!-- VIDEOS:BEGIN -->.*?<!-- VIDEOS:END -->", lambda m: f"<!-- VIDEOS:BEGIN -->\n{videos}\n<!-- VIDEOS:END -->", s, flags=re.S)
page.write_text(s, encoding="utf-8")
print(f"wrote {len(FEATURES)} features and {len(FEATURES)} videos into web/index.html")
