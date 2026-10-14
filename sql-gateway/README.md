# ontological-sql — PostgreSQL-protocol gateway

Lets any PostgreSQL client type fabric statements and Cypher where it types SQL.

```sql
CREATE DATASOURCE erp WITH ENGINE = 'postgres', PARAMETERS = {"host": "…", "schema": "erp", …};
SHOW TABLES FROM erp;
CREATE VIRTUAL NODE Supplier FROM erp.supplier KEY supplier_id;
CREATE VIRTUAL RELATIONSHIP SHIPPED_BY FROM Shipment(vendor_code) TO Supplier(supplier_id);

MATCH (sh:Shipment)-[:SHIPPED_BY]->(s:Supplier) WHERE sh.delay_days >= 5 RETURN s.name, sh.shipment_no;
SELECT s.name, sh.delay_days FROM erp.supplier s JOIN wms.shipment sh ON sh.vendor_code = s.supplier_id;
```

PostgreSQL gives an extension no way to add top-level grammar
([ADR-008](../docs/99_decisions/ADR-008-cypher-entry-via-function-call.md)), so these cannot be statements the
server parses. They can be text that is translated to SQL the server does parse.

## How it works

The gateway is a relay, not a server. Every protocol message passes through byte for byte — authentication,
transactions, prepared statements, `COPY`, cancellation — except the SQL text of a statement whose first words
mark it as a fabric statement or a Cypher query. For those it asks the server what the statement means
(`SELECT og_fabric_translate(…)`) and sends the answer in the statement's place.

```
client ──pgwire──▶ ontological-sql ──pgwire──▶ PostgreSQL + ontological
                     │  first word is CREATE DATASOURCE / SHOW TABLES / MATCH / … ?
                     │    yes → ask og_fabric_translate, forward the SQL it returns
                     └─   no  → forward untouched
```

The grammar lives in the extension (`engine/src/fabric/sqlish.rs`) and is covered by the regression suite
(`engine/tests/sql/14_fabric_statements.sql`). The gateway knows only which statements to ask about, and has no
dependencies beyond the standard library. The same statements are also accepted wherever Cypher is — `og_cypher()`
hands them to the same translator — which is how the Studio and Bolt drivers use them with no gateway at all.

## Run

```bash
cargo build --release
OG_SQL_LISTEN=127.0.0.1:5433 OG_SQL_PGHOST=localhost OG_SQL_PGPORT=5432 ./target/release/ontological-sql
psql -h localhost -p 5433 -d mydb
```

| variable | default | |
|---|---|---|
| `OG_SQL_LISTEN` | `127.0.0.1:5433` | where to accept PostgreSQL clients |
| `OG_SQL_PGHOST` | `localhost` | the server to relay to |
| `OG_SQL_PGPORT` | `5432` | |

## What to know

- **The server sees every client as coming from the gateway's address.** If `pg_hba.conf` trusts that address,
  anyone who can reach the gateway is in without a password. Listen on loopback, or make the server demand
  a password from the gateway's address. Password and SCRAM authentication pass through unchanged.
- **No TLS.** An `SSLRequest` is answered "no"; put the gateway next to the client or behind a tunnel.
- **Cypher results are `text` columns**, one per `RETURN` item; numbers, nodes and lists arrive as their JSON text.
- **Cypher typed here takes no parameters.** Use Bolt, or `og_cypher(graph, query, params)`.
- **One fabric statement or Cypher query per message.** psql sends statements one at a time, so this only
  matters to a client that batches several in one string.
- In the middle of an unsynced extended-protocol batch the server cannot be asked anything, so a fabric
  statement there runs through `og_fabric(…)` instead and returns `jsonb` rows.

A walk-through with real output: [`docs/tutorials/virtual-fabric-tutorial.md`](../docs/tutorials/virtual-fabric-tutorial.md).
