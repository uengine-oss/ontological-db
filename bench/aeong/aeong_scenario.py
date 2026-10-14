"""AeonG(시간 그래프 DB) 시나리오 — Ontological 쪽 ontological_scenario.sql과 같은 이야기.

입사(salary 100) → 연봉 200 → 정정 250 → 퇴사(관계 삭제) → 노드 삭제.
각 쓰기 뒤의 벽시계 시각(ms)을 기록해 두고 TT AS / TT FROM..TO 로 되돌아가 본다.
AeonG를 --real_time_flag=true 로 띄우면 TT 값은 epoch 밀리초다.

사용: python3 aeong_scenario.py [--container aeong-arm] [--gc]
  쿼리는 컨테이너 안의 mgconsole 로 보낸다. (이 AeonG 빌드(Memgraph 2.3 기반)는
  Python neo4j 4.4 드라이버의 Bolt 4.3 HELLO 를 거절해서, 드라이버 대신 mgconsole 을 쓴다.)
  --keep-node 를 주면 마지막 노드 삭제를 건너뛴다.
  --gc 를 주면 쿼리 전에 FREE MEMORY 로 GC를 돌려 과거 버전이 RocksDB로 옮겨진 뒤에 읽는다.
"""
import csv
import io
import subprocess
import sys
import time

args = sys.argv[1:]
container = args[args.index("--container") + 1] if "--container" in args else "aeong-arm"
force_gc = "--gc" in args
keep_node = "--keep-node" in args  # 마지막 DETACH DELETE 를 건너뛴다
MGCONSOLE = ["docker", "exec", "-i", container, "/home/aeong/build/bin/mgconsole",
             "--host", "127.0.0.1", "--port", "7687", "--output-format", "csv"]


def run(q, show=True):
    p = subprocess.run(MGCONSOLE, input=q + ";\n", capture_output=True, text=True)
    rows = None
    if p.returncode == 0 and "Failed query" not in p.stderr:
        table = list(csv.reader(io.StringIO(p.stdout)))
        rows = [dict(zip(table[0], r)) for r in table[1:]] if table else []
    else:
        msg = (p.stderr or p.stdout).strip().splitlines()[-1][:200] if (p.stderr or p.stdout).strip() else f"rc={p.returncode}"
    if show:
        print(f"  {q}")
        print(f"    -> {rows if rows is not None else 'ERROR: ' + msg}")
    return rows


def mark(label):
    time.sleep(0.05)
    t = int(time.time() * 1000)
    time.sleep(0.05)
    print(f"[{label}] t={t}")
    return t


run("MATCH (n) DETACH DELETE n", show=False)
t0 = mark("T0 빈 그래프")

print("== 쓰기 ==")
run("CREATE (:Person {name:'ann', salary:100})-[:WORKS_AT {title:'eng'}]->(:Company {name:'acme'})")
t1 = mark("T1 입사, salary 100")
run("MATCH (p:Person {name:'ann'}) SET p.salary = 200")
t2 = mark("T2 연봉 200")
run("MATCH (p:Person {name:'ann'}) SET p.salary = 250")
t3 = mark("T3 정정 250")
run("MATCH (:Person {name:'ann'})-[w:WORKS_AT]->() DELETE w")
t4 = mark("T4 퇴사(관계 삭제)")
if not keep_node:
    run("MATCH (p:Person {name:'ann'}) DETACH DELETE p")
t5 = mark("T5 사람 노드 삭제")

if force_gc:
    print("== GC: FREE MEMORY (과거 버전 → RocksDB) ==")
    run("FREE MEMORY")
    time.sleep(2)
    run("FREE MEMORY")

print("== 현재 ==")
run("MATCH (p:Person) RETURN p.name AS name, p.salary AS salary")
run("MATCH (c:Company) RETURN c.name AS name")

print("== 시점(TT AS) ==")
for label, t in [("T0", t0), ("T1", t1), ("T2", t2), ("T3", t3), ("T4", t4), ("T5", t5)]:
    print(f"-- {label}")
    run(f"MATCH (p:Person {{name:'ann'}}) TT AS {t} RETURN p.salary AS salary")
    run(f"MATCH (p:Person {{name:'ann'}})-[w:WORKS_AT]->(c:Company) TT AS {t} RETURN c.name AS company, w.title AS title")

print("== 구간(TT FROM .. TO) ==")
run(f"MATCH (p:Person {{name:'ann'}}) TT FROM {t0} TO {t5} RETURN p.salary AS salary")
run(f"MATCH (p:Person {{name:'ann'}}) TT FROM {t1} TO {t2} RETURN p.salary AS salary")
run(f"MATCH (p:Person {{name:'ann'}})-[w:WORKS_AT]->(c) TT FROM {t0} TO {t5} RETURN c.name AS company")

print("== 형태별 지원 여부 (TT AS T3) ==")
run(f"MATCH (n) TT AS {t3} RETURN labels(n) AS l, n.name AS name")
run(f"MATCH (p:Person)-[:WORKS_AT*1..2]->(c) TT AS {t3} RETURN c.name AS company")
run(f"MATCH (p:Person)-[r]->(c) TT AS {t3} RETURN type(r) AS t")
run(f"MATCH (p:Person {{name:'ann'}}) WHERE p.salary > 150 TT AS {t3} RETURN p.salary AS salary")
run(f"MATCH (p:Person {{name:'ann'}}) TT AS {t1} WITH p MATCH (p)-[:WORKS_AT]->(c) RETURN c.name AS company")
run(f"OPTIONAL MATCH (p:Person {{name:'ann'}}) TT AS {t3} RETURN p.salary AS salary")

print("== 유효시간(VT) 절 — 문법만 있고 동작은? ==")
run("MATCH (c:Company) VT AS 0 RETURN c.name AS name")
run(f"MATCH (p:Person {{name:'ann'}}) TT AS {t3} VT AS 0 RETURN p.salary AS salary")

print("== 쓰기 쪽 시간 지정(유효시간 소급) — 구문이 있는가? ==")
run("CREATE (:Person {name:'bob', salary:1}) VT AS 0")
run("MATCH (c:Company) TT AS 0 SET c.name = 'x'")

