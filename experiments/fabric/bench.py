#!/usr/bin/env python3
"""Virtual fabric — same questions, three ways, answers checked before timings.

  A  one Cypher query to Ontological; virtual types read through FDW pushdown
  M  the equivalent federated SQL, hand-written, sent to MindsDB's HTTP API
     (the best case for "keep MindsDB as the fabric", and the SQL step of
     today's flow, where an agent writes that SQL after a graph lookup)
  C  today's flow end to end, only where the question needs graph-only data:
     Cypher for the native part, then SQL to MindsDB with the keys it returned

Usage:  source .env.fabric; python3 bench.py [--runs 7]
"""
import argparse, json, os, statistics, time
import psycopg2, requests

MINDSDB = os.environ.get("MINDSDB_URL", "http://localhost:47334/api/sql/query")

QUESTIONS = [
    {
        "id": "Q1", "title": "Single source, filter",
        "cypher": "MATCH (s:Supplier) WHERE s.country = 'China' RETURN s.supplier_id AS a, s.supplier_name AS b",
        "sql": "SELECT supplier_id, supplier_name FROM scm_erp.mst_supplier WHERE country = 'China'",
    },
    {
        "id": "Q2", "title": "Same-source hop + aggregate (PG·PG)",
        "cypher": "MATCH (po:PurchaseOrder)-[:ORDERED_FROM]->(s:Supplier) WHERE po.total_amount_usd > 50000 "
                  "RETURN s.supplier_id AS a, count(po) AS b",
        "sql": "SELECT s.supplier_id, count(*) FROM scm_erp.doc_purchase_order po "
               "JOIN scm_erp.mst_supplier s ON po.supplier_id = s.supplier_id "
               "WHERE po.total_amount_usd > 50000 GROUP BY s.supplier_id",
    },
    {
        "id": "Q3", "title": "Cross-source 2 hops (MySQL→PG→PG)",
        "cypher": "MATCH (sh:Shipment)-[:SHIPS]->(po:PurchaseOrder)-[:ORDERED_FROM]->(s:Supplier) "
                  "WHERE sh.delay_days >= 5 RETURN sh.shipment_no AS a, po.po_id AS b, s.supplier_name AS c",
        "sql": "SELECT sh.shipment_no, po.po_id, s.supplier_name FROM scm_wms.shipment sh "
               "JOIN scm_erp.doc_purchase_order po ON sh.ref_po_no = po.po_id "
               "JOIN scm_erp.mst_supplier s ON po.supplier_id = s.supplier_id WHERE sh.delay_days >= 5",
    },
    {
        "id": "Q4", "title": "Three systems (customs PG → WMS MySQL → ERP PG)",
        "cypher": "MATCH (d:Declaration)-[:DECLARES]->(sh:Shipment)-[:SHIPPED_BY]->(s:Supplier) "
                  "WHERE d.status IN ['HOLD','REJECTED'] RETURN d.decl_id AS a, sh.shipment_no AS b, s.supplier_id AS c",
        "sql": "SELECT d.decl_id, sh.shipment_no, s.supplier_id FROM scm_customs.decl_header d "
               "JOIN scm_wms.shipment sh ON d.ref_shipment_no = sh.shipment_no "
               "JOIN scm_erp.mst_supplier s ON sh.vendor_code = s.supplier_id "
               "WHERE d.status IN ('HOLD','REJECTED')",
    },
    {
        "id": "Q5", "title": "Branching pattern over CRM (MySQL) and ERP (PG)",
        "cypher": "MATCH (cu:Customer)<-[:PLACED_BY]-(so:SalesOrder)<-[:ABOUT_ORDER]-(c:Claim {severity:'HIGH'}), "
                  "(so)-[:ORDERS_ITEM]->(i:Item) RETURN c.ticket_no AS a, cu.cust_name AS b, i.item_name AS c",
        "sql": "SELECT c.ticket_no, cu.cust_name, i.item_name FROM scm_crm.claim_ticket c "
               "JOIN scm_crm.sales_order so ON c.so_no = so.so_no "
               "JOIN scm_crm.customer cu ON so.cust_no = cu.cust_no "
               "JOIN scm_erp.mst_item i ON so.sku_code = i.item_id WHERE c.severity = 'HIGH'",
    },
    {
        "id": "Q7", "title": "Four hops, three systems, mixed directions (customs → WMS → ERP)",
        "cypher": "MATCH (d:Declaration)-[:DECLARES]->(sh:Shipment)-[:SHIPS]->(po:PurchaseOrder)<-[:LINE_OF]-(l:POLine)"
                  "-[:FOR_ITEM]->(i:Item) WHERE d.fta_applied = false RETURN i.item_id AS a, count(DISTINCT d.decl_id) AS b",
        "sql": "SELECT i.item_id, count(DISTINCT d.decl_id) FROM scm_customs.decl_header d "
               "JOIN scm_wms.shipment sh ON d.ref_shipment_no = sh.shipment_no "
               "JOIN scm_erp.doc_purchase_order po ON sh.ref_po_no = po.po_id "
               "JOIN scm_erp.doc_po_line l ON l.po_id = po.po_id "
               "JOIN scm_erp.mst_item i ON l.item_id = i.item_id "
               "WHERE d.fta_applied = false GROUP BY i.item_id",
    },
    {
        "id": "Q8", "title": "Virtual view with a composite key (WMS stock × ERP item)",
        "cypher": "MATCH (st:StockDaily)-[:STOCK_OF]->(i:Item) WHERE st.snapshot_date >= '2026-08-20' "
                  "AND st.on_hand_qty < i.safety_stock RETURN st.sku_code AS a, st.warehouse_cd AS b, st.on_hand_qty AS c",
        "sql": "SELECT st.sku_code, st.warehouse_cd, st.on_hand_qty "
               "FROM scm_wms.stock_daily st JOIN scm_erp.mst_item i ON st.sku_code = i.item_id "
               "WHERE st.snapshot_date >= '2026-08-20' AND st.on_hand_qty < i.safety_stock",
    },
    {
        "id": "Q9", "title": "Same as Q8 with LIMIT 20 — a limit above a cross-source predicate",
        "cypher": "MATCH (st:StockDaily)-[:STOCK_OF]->(i:Item) WHERE st.snapshot_date >= '2026-08-20' "
                  "AND st.on_hand_qty < i.safety_stock RETURN st.sku_code AS a ORDER BY a LIMIT 20",
        "sql": "SELECT st.sku_code FROM scm_wms.stock_daily st JOIN scm_erp.mst_item i ON st.sku_code = i.item_id "
               "WHERE st.snapshot_date >= '2026-08-20' AND st.on_hand_qty < i.safety_stock ORDER BY st.sku_code LIMIT 20",
    },
    {
        "id": "Q6", "title": "Graph-only knowledge → virtual rows (native RiskEvent → Supplier ← Shipment)",
        "cypher": "MATCH (:Region {name:'East Asia'})<-[:IN_REGION]-(e:RiskEvent)-[:AFFECTS]->(s:Supplier)"
                  "<-[:SHIPPED_BY]-(sh:Shipment) WHERE sh.ship_status <> 'DELIVERED' "
                  "RETURN e.title AS a, sh.shipment_no AS b",
        # MindsDB cannot see RiskEvent at all; today's flow asks the graph first.
        "sql": None,
        "two_step": {
            "cypher": "MATCH (:Region {name:'East Asia'})<-[:IN_REGION]-(e:RiskEvent) "
                      "RETURN e.title AS t, e.supplier_code AS k",
            "sql": "SELECT vendor_code, shipment_no FROM scm_wms.shipment "
                   "WHERE ship_status <> 'DELIVERED' AND vendor_code IN ({keys})",
        },
    },
]


def norm(rows):
    """Order-free comparison: each row as a tuple of strings, numbers normalised."""
    out = []
    for r in rows:
        vals = []
        for v in r:
            if isinstance(v, float) and v.is_integer():
                v = int(v)
            vals.append(str(v))
        out.append(tuple(vals))
    return sorted(out)


def run_cypher(cur, graph, q):
    cur.execute("SELECT r FROM og_cypher(%s, %s) r", (graph, q))
    rows = [r[0] for r in cur.fetchall()]
    keys = sorted(rows[0].keys()) if rows else []
    return [[row[k] for k in keys] for row in rows]


def run_mindsdb(sql):
    r = requests.post(MINDSDB, json={"query": sql}, timeout=120)
    d = r.json()
    if d.get("type") == "error":
        raise RuntimeError(d.get("error_message", "")[:300])
    return d.get("data", [])


def timed(fn, runs):
    times, result = [], None
    for _ in range(runs):
        t = time.perf_counter()
        result = fn()
        times.append((time.perf_counter() - t) * 1000)
    return result, statistics.median(times)


def rows_fetched(cur, graph, q):
    """Rows that crossed the wire: actual rows of every Foreign Scan."""
    cur.execute("SELECT og_cypher_sql(%s, %s)", (graph, q))
    sql = cur.fetchone()[0]
    cur.execute("EXPLAIN (ANALYZE, FORMAT JSON) " + sql)
    plan = cur.fetchone()[0][0]["Plan"]
    total = 0
    def walk(n):
        nonlocal total
        if n.get("Node Type") == "Foreign Scan":
            total += n.get("Actual Rows", 0) * n.get("Actual Loops", 1)
        for c in n.get("Plans", []):
            walk(c)
    walk(plan)
    return total


def remote_sql(cur, graph, q):
    """What actually reached each source: the Remote SQL lines of EXPLAIN VERBOSE."""
    cur.execute("SELECT og_cypher_sql(%s, %s)", (graph, q))
    sql = cur.fetchone()[0]
    cur.execute("EXPLAIN (VERBOSE, COSTS OFF) " + sql)
    plan = [r[0] for r in cur.fetchall()]
    remote = []
    for i, line in enumerate(plan):
        s = line.strip()
        if s.startswith("Remote SQL:") or s.startswith("Remote query:"):
            remote.append(s.split(":", 1)[1].strip())
        elif "Foreign Scan" in s:
            remote.append("[" + s.replace("->", "").strip() + "]")
    return sql, remote


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=7)
    ap.add_argument("--json", default="results.json")
    args = ap.parse_args()

    conn = psycopg2.connect(host="127.0.0.1", port=28826, dbname="fabric", user="dev",
                            password=os.environ["OG_PW"])
    conn.autocommit = True
    cur = conn.cursor()
    report = []

    for q in QUESTIONS:
        entry = {"id": q["id"], "title": q["title"], "cypher": q["cypher"]}
        a_rows, a_ms = timed(lambda: run_cypher(cur, "scm", q["cypher"]), args.runs)
        entry.update(a_rows=len(a_rows), a_ms=round(a_ms, 1))
        entry["compiled_sql"], entry["remote"] = remote_sql(cur, "scm", q["cypher"])
        entry["fetched"] = rows_fetched(cur, "scm", q["cypher"])

        if q["sql"]:
            entry["sql"] = q["sql"]
            try:
                m_rows, m_ms = timed(lambda: run_mindsdb(q["sql"]), args.runs)
                entry.update(m_rows=len(m_rows), m_ms=round(m_ms, 1),
                             equal=norm(a_rows) == norm(m_rows))
            except Exception as e:  # noqa: BLE001 — the failure is the finding
                entry.update(m_error=str(e))
        else:
            ts = q["two_step"]

            def two_step():
                g = run_cypher(cur, "scm", ts["cypher"])          # [[supplier_code, title]]
                keys = ", ".join("'" + k.replace("'", "''") + "'" for k, _ in g)
                rows = run_mindsdb(ts["sql"].format(keys=keys))   # [[vendor_code, shipment_no]]
                titles = {}
                for k, t in g:
                    titles.setdefault(k, []).append(t)
                return [[t, shp] for vc, shp in rows for t in titles.get(vc, [])]

            c_rows, c_ms = timed(two_step, args.runs)
            entry.update(sql="(two steps) " + ts["cypher"] + "  ⟶  " + ts["sql"],
                         c_rows=len(c_rows), c_ms=round(c_ms, 1),
                         equal=norm(a_rows) == norm(c_rows))
        report.append(entry)

        other = entry.get("m_ms", entry.get("c_ms"))
        print(f"{q['id']} {q['title']}")
        print(f"   A cypher/FDW : {entry['a_ms']:>8} ms  rows={entry['a_rows']}  fetched-from-sources={entry['fetched']}")
        if "m_error" in entry:
            print(f"   MindsDB SQL  : ERROR {entry['m_error'][:160]}")
        else:
            label = "MindsDB SQL " if "m_ms" in entry else "C two-step  "
            print(f"   {label} : {other:>8} ms  rows={entry.get('m_rows', entry.get('c_rows'))}  equal={entry['equal']}")
        for r in entry["remote"]:
            print(f"     ↳ {r[:200]}")

    with open(args.json, "w") as f:
        json.dump(report, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
