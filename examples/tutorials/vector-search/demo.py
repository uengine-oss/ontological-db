#!/usr/bin/env python3
"""The vector-search demo video. See ../demo_lib.py for how a demo is recorded.

    python3 demo.py            # → docs/tutorials/media/vector-search-demo.mp4
    FAST=1 python3 demo.py     # same checks, short holds, no file kept

Needs a Studio running with PGDATABASE=demo_vector_search and OG_STUDIO_ALLOW_RAW_SQL=1
(the hybrid ranking is a SQL function, run with `:sql`). The vectors are three-dimensional
and written by hand — axis 1 account/login, axis 2 payment, axis 3 delivery — because
there is no embedding model here; the captions say so.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from demo_lib import Demo, Stop, record

INDEX_OPTIONS = "OPTIONS {indexConfig: {`vector.dimensions`: 3, `vector.similarity_function`: 'cosine'}}"

DATA = """CREATE (acct:Product {name: '계정'}), (pay:Product {name: '결제'}), (ship:Product {name: '배송'})
CREATE (a1:Article {title: '비밀번호 재설정 방법', status: 'published', emb: '[1,0,0]'})-[:ABOUT]->(acct)
CREATE (a2:Article {title: '2단계 인증 설정', status: 'draft', emb: '[0.9,0.1,0]'})-[:ABOUT]->(acct)
CREATE (a3:Article {title: '카드 결제 오류 해결', status: 'published', emb: '[0.1,0.9,0]'})-[:ABOUT]->(pay)
CREATE (a4:Article {title: '환불 절차 안내', status: 'published', emb: '[0,0.9,0.3]'})-[:ABOUT]->(pay)
CREATE (a5:Article {title: '배송 조회 방법', status: 'published', emb: '[0,0,1]'})-[:ABOUT]->(ship)
CREATE (a6:Article {title: '해외 배송 관세 안내', status: 'published', emb: '[0,0.4,0.9]'})-[:ABOUT]->(ship)
CREATE (:Ticket {title: '로그인이 안 돼요'})-[:RESOLVED_BY {note: '비밀번호 분실. 재설정으로 해결', note_emb: '[1,0,0]'}]->(a1)
CREATE (:Ticket {title: '결제가 계속 실패해요'})-[:RESOLVED_BY {note: '세션 만료로 결제 인증이 막힘. 재설정 후 결제 성공', note_emb: '[0.6,0.8,0]'}]->(a1)
CREATE (:Ticket {title: '카드 승인이 거절돼요'})-[:RESOLVED_BY {note: '카드 한도 초과. 다른 카드로 결제', note_emb: '[0,1,0]'}]->(a3)
CREATE (:Ticket {title: '환불이 안 들어와요'})-[:RESOLVED_BY {note: '환불은 영업일 3일이 걸린다고 안내', note_emb: '[0,0.9,0.2]'}]->(a4)
CREATE (:Ticket {title: '택배가 안 와요'})-[:RESOLVED_BY {note: '송장 조회로 배송 지연 확인', note_emb: '[0,0,1]'}]->(a5)"""

HYBRID = (":sql SELECT h.entity->>'title' AS title, round(h.vector_score::numeric, 3) AS vector_score,\n"
          "  round(h.graph_score::numeric, 3) AS graph_score, round(h.score::numeric, 5) AS score\n"
          "FROM og_hybrid_search('support', 'Article', 'emb', '[0,1,0.2]',\n"
          "  (SELECT (r->>'id')::bigint FROM og_cypher('support', $$MATCH (p:Product {name: '배송'}) RETURN id(p) AS id$$) r),\n"
          "  4, 1.0, 0.02) h")


def in_order(text, *wanted):
    """Stop unless the strings appear in the frame in this order."""
    at = [text.find(w) for w in wanted]
    if -1 in at or at != sorted(at):
        raise Stop(f"expected the order {wanted}, the frame shows:\n{text[:500]}")


def story(d: Demo):
    page = d.page

    d.scene("title")
    d.card("<h1>문서가 아니라<br><em>해결된 이유를 의미로 찾습니다</em></h1>"
           "<p>고객지원 지식 베이스. 문의는 자신을 해결한 문서를 가리키고,<br>상담원은 그 관계에 왜 풀렸는지를 적어 둡니다.</p>"
           "<div class='row'><span class='chip'>노드 임베딩 · <b>Article.emb</b></span>"
           "<span class='chip'>관계 임베딩 · <b>RESOLVED_BY.note_emb</b></span></div>", 7,
           "문서가 아니라 해결된 이유를 의미로 찾습니다. 임베딩을 노드와 관계 양쪽에 둡니다.")

    d.scene("graph")
    d.caption("빈 데이터베이스에서 시작합니다. 고객지원 그래프를 만듭니다.", read=False)
    d.run("CREATE GRAPH support", expect="graph support created", look=2.0)
    # The graph list is read when the page loads; this is the one reload.
    page.reload()
    page.wait_for_selector("#graph-select option", state="attached")
    page.select_option("#graph-select", "support")
    d.overlay()

    d.scene("declare")
    d.caption("문서의 임베딩을 선언합니다. Neo4j와 같은 벡터 인덱스 문장입니다.", read=False)
    d.run("CREATE VECTOR INDEX article_emb FOR (a:Article) ON (a.emb)\n" + INDEX_OPTIONS, look=1.6)
    d.caption("같은 문장을 관계에 씁니다. 문의를 문서로 해결한 이유에 임베딩을 붙입니다.", read=False)
    d.run("CREATE VECTOR INDEX resolution_note FOR ()-[r:RESOLVED_BY]-() ON (r.note_emb)\n" + INDEX_OPTIONS, look=1.2)
    d.run(":sql SELECT t.name AS type, e.prop, e.dims, e.metric FROM og_catalog.embedding e JOIN og_catalog.type t USING (type_id) ORDER BY 1",
          expect=["Article", "RESOLVED_BY", "note_emb", "cosine"], look=1.0)
    d.caption("임베딩은 노드에 하나, 관계에 하나. 둘 다 3차원 벡터 프로퍼티입니다.")

    d.scene("data")
    d.clear()
    d.caption("문서 여섯 건과 처리된 문의 다섯 건을 넣습니다. 벡터는 손으로 만든 3차원입니다. 실제로는 임베딩 모델이 만듭니다.",
              read=False)
    d.run(DATA, look=1.0)
    d.caption("세 축은 계정, 결제, 배송입니다. 눈으로 검산할 수 있습니다.", read=False)
    d.run("MATCH (t:Ticket)-[r:RESOLVED_BY]->(a:Article)\n"
          "RETURN t.title AS ticket, a.title AS article, r.note AS why, r.note_emb AS note_emb",
          expect=["결제가 계속 실패해요", "세션 만료로 결제 인증이 막힘", "[0.6,0.8,0]"], look=1.0)
    d.caption("결제 실패 문의가 비밀번호 재설정 문서로 풀렸습니다. 이유는 관계의 메모에만 적혀 있습니다.")

    d.scene("search the articles")
    d.clear()
    d.caption("새 문의입니다. 결제가 안 되는데 계정 쪽 문제 같다고 합니다. 벡터로는 결제 0.8, 계정 0.6입니다.", read=False)
    d.run(":param q => [0.6, 0.8, 0]", expect="not saved", look=1.6)
    d.caption("먼저 문서에서 가까운 것을 찾습니다.", read=False)
    text = d.run("CALL db.index.vector.queryNodes('article_emb', 3, $q) YIELD node, score\n"
                 "RETURN node.title AS article, round(toFloat(score) * 1000) / 1000 AS score",
                 expect=["카드 결제 오류 해결", "0.861", "환불 절차 안내"], look=1.0)
    if "비밀번호 재설정" in text:
        raise Stop("the password article should not be among the three nearest articles")
    d.caption("결제 문서가 나옵니다. 실제로 이런 문의를 풀었던 비밀번호 재설정 문서는 셋 안에 없습니다.")

    d.scene("search the relationships")
    d.clear()
    d.caption("같은 벡터로 이번에는 관계를 찾습니다. 관계의 임베딩을 패턴 안에서 그대로 씁니다.", read=False)
    text = d.run("MATCH (t:Ticket)-[r:RESOLVED_BY]->(a:Article)\n"
                 "RETURN t.title AS ticket, a.title AS article, r.note AS why,\n"
                 "       round(vector.similarity(r.note_emb, $q) * 1000) / 1000 AS score\n"
                 "ORDER BY score DESC LIMIT 3",
                 expect=["비밀번호 재설정 방법", "세션 만료로 결제 인증이 막힘", "0.781"], look=1.0)
    in_order(text, "세션 만료로 결제 인증이 막힘", "카드 한도 초과", "환불은 영업일 3일")
    d.caption("첫 줄입니다. 결제 실패가 비밀번호 재설정으로 풀린 사례를, 해결된 이유의 의미로 찾았습니다.")
    d.caption("노드와 관계를 그대로 반환하면 그래프로 보입니다.", read=False)
    d.run("MATCH (t:Ticket)-[r:RESOLVED_BY]->(a:Article)\n"
          "RETURN t, r, a ORDER BY vector.similarity(r.note_emb, $q) DESC LIMIT 2", look=3.0)
    if "Graph" not in " ".join(page.locator(".frame").first.locator(".tab").all_inner_texts()):
        raise Stop("the result of the node query has no Graph tab")

    d.scene("one query")
    d.clear()
    d.caption("유사도는 질의의 한 부분입니다. 계정 영역의 문서로 풀린 사례 중 결제와 가까운 것을 묻습니다.", read=False)
    d.run(":param q => [0, 1, 0]", expect="not saved", look=0.8)
    text = d.run("MATCH (t:Ticket)-[r:RESOLVED_BY]->(a:Article)-[:ABOUT]->(p:Product {name: '계정'})\n"
                 "RETURN t.title AS ticket, a.title AS article,\n"
                 "       round(vector.similarity(r.note_emb, $q) * 1000) / 1000 AS score\n"
                 "ORDER BY score DESC",
                 expect=["결제가 계속 실패해요", "0.8", "로그인이 안 돼요"], look=1.0)
    if "카드 승인이 거절돼요" in text:
        raise Stop("a case resolved by a payment article passed the product condition")
    d.caption("두 홉의 패턴, 제품 조건, 관계의 유사도가 한 문장에 있습니다. 결제 영역의 사례는 빠졌습니다.")

    d.scene("filter before top-k")
    d.clear()
    d.caption("조건은 언제 적용될까요? 고객에게는 공개된 문서만 보여야 합니다. 로그인 문의로 상위 두 건을 찾습니다.", read=False)
    d.run(":param q => [1, 0, 0]", expect="not saved", look=0.8)
    text = d.run("CALL db.index.vector.queryNodes('article_emb', 2, $q) YIELD node, score\n"
                 "WITH node, score WHERE node.status = 'published'\n"
                 "RETURN node.title AS article, node.status AS status",
                 expect="비밀번호 재설정 방법", look=1.0)
    if "카드 결제" in text or "2단계 인증" in text:
        raise Stop("filtering after the top two should leave one article")
    d.caption("두 건을 뽑은 뒤에 걸렀더니 한 건만 남았습니다. 둘째가 초안이었기 때문입니다.")
    d.caption("조건을 같은 질의의 WHERE에 쓰면 조건을 통과한 것 중에서 두 건을 고릅니다.", read=False)
    text = d.run("MATCH (a:Article) WHERE a.status = 'published'\n"
                 "RETURN a.title AS article, round(vector.similarity(a.emb, $q) * 1000) / 1000 AS score\n"
                 "ORDER BY score DESC LIMIT 2",
                 expect=["비밀번호 재설정 방법", "카드 결제 오류 해결", "0.11"], look=1.0)
    if "2단계 인증" in text:
        raise Stop("the draft article passed the status condition")
    d.caption("두 건이 채워졌습니다. 조건과 유사도 정렬이 SQL 한 문장으로 컴파일되기 때문입니다.")

    d.scene("hybrid")
    d.clear()
    d.caption("마지막으로 순위입니다. 배송 담당 상담원에게 환불 문의가 왔습니다. 벡터 순위에 그래프 거리를 더합니다.",
              read=False)
    text = d.run(HYBRID, expect=["환불 절차 안내", "0.992", "해외 배송 관세 안내", "0.500", "0.01620"], look=1.0)
    in_order(text, "환불 절차 안내", "해외 배송 관세 안내", "카드 결제 오류 해결", "배송 조회 방법")
    d.caption("환불 문서가 1위이고, 배송 영역에 연결된 관세 문서가 카드 결제 문서를 앞섰습니다.")
    d.caption("벡터 점수와 그래프 점수가 따로 보입니다. 두 순위는 RRF로 합쳐지고 가중치는 인자로 정합니다.")

    d.scene("what it does not do")
    d.clear()
    d.caption("문장을 벡터로 바꾸는 일은 임베딩 모델의 몫입니다. 이를 위한 함수는 기본으로 꺼져 있습니다.", read=False)
    d.run("RETURN genai.vector.encode('결제가 안 됩니다') AS v", error=True, expect="disabled", look=1.0)
    d.caption("외부 서버를 호출하는 함수라서 켜기 전에는 실행되지 않습니다. 이 데모는 손으로 만든 벡터를 썼습니다.")

    d.scene("end")
    d.card("<h1>임베딩은 프로퍼티입니다<br><em>노드에도, 관계에도</em></h1>"
           "<div class='row'><span class='chip'><b>CREATE VECTOR INDEX</b></span>"
           "<span class='chip'>vector.<b>similarity</b>(r.note_emb, $q)</span>"
           "<span class='chip'><b>og_hybrid_search</b></span></div>"
           "<p>패턴 · 조건 · 유사도를 한 질의로 · 벡터 순위와 그래프 거리를 RRF로</p>", 7,
           "임베딩은 프로퍼티입니다. 노드에도, 관계에도. 패턴과 조건과 유사도를 한 질의로 씁니다.")
    d.scene(None)


if __name__ == "__main__":
    record("vector-search", story)
