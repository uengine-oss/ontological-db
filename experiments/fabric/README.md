# Virtual fabric experiment

Design, measurements and findings are in
[`docs/99_decisions/ADR-029-virtual-fabric-via-fdw.md`](../../docs/99_decisions/ADR-029-virtual-fabric-via-fdw.md).

## Layout

| file | what |
|---|---|
| `01_setup.sql` | approach A: three datasources (PG, MySQL ×2), ten virtual types, eleven virtual relationships, native `Region`/`RiskEvent` nodes pointing at virtual suppliers |
| `02_setup_mindsdb.sql` | approach B: the same model with every foreign table pointing at MindsDB. Setup succeeds; every query through it fails (see the ADR) |
| `bench.py` | nine questions; A (one Cypher query) against the equivalent federated SQL sent to MindsDB; answers compared before times |
| `results.json` | last run, including the compiled SQL and the Remote SQL each source received |
| `.env.fabric` | generated credentials for the experiment's read-only source accounts (git-ignored) |

## Reproduce

Needs the robo stack running (`robo-postgres`, `robo-mysql-sample`, `robo-mindsdb` on `robo-network`) with the
SCM demo loaded (`ontologic/demo/scm-agent/README.md`).

```bash
# container: same image as `og`, on robo-network, its own port
docker run -d --name og-fabric --network robo-network -v "$PWD":/work -p 28826:28816 -w /work ontological-dev sleep infinity
docker exec -u root og-fabric bash -lc 'apt-get update -qq && apt-get install -y -qq postgresql-16-mysql-fdw'
docker exec og-fabric bash -lc 'cd /work/engine && cargo pgrx install --features pg16 --no-default-features \
  --pg-config /usr/lib/postgresql/16/bin/pg_config --sudo && cargo pgrx start pg16 && createdb -h localhost -p 28816 fabric'

# read-only source accounts: og_fabric on robo-postgres (erp, customs) and robo-mysql-sample (wms, crm_scm),
# with the password in experiments/fabric/.env.fabric as FABRIC_PW

source experiments/fabric/.env.fabric
docker exec -i og-fabric psql -h localhost -p 28816 -d fabric -v pw="$FABRIC_PW" < experiments/fabric/01_setup.sql
docker exec -i og-fabric psql -h localhost -p 28816 -d fabric < experiments/fabric/02_setup_mindsdb.sql
OG_PW=$OG_PW python3 experiments/fabric/bench.py --runs 7
```

The self-contained regression (postgres_fdw loopback, no external databases) runs with the rest of the suite:
`./tests/run.sh` → `07_virtual_fabric.sql`.
