# 상호운용 · RLS · RDF/OWL API

> **이 문서가 답하는 질문**
> - Cypher 순회 중간에 RLS가 왜 저절로 적용되는가?
> - 기존 관계형 테이블을 복사 없이 그래프 타입으로 노출하려면?
> - PostgREST / supabase-js에서 어떻게 부르는가?
> - RDF를 넣으면 무엇이 타입 시스템에 매핑되고, 무엇이 남는가?
> - SPARQL은 왜 없는가?

---

## 1. 결정(Decision) — RLS가 "저절로" 적용되는 이유

컴파일된 Cypher 질의는 **평범한 테이블**을 읽는다. 그러므로 그 테이블에 걸린
row-level security가 **순회 중간에** 적용된다. 우리 쪽에 강제 코드가 없다.
호출자가 볼 수 없는 노드는 그냥 조인되지 않고, 그 노드를 지나는 모든 경로가
사라진다(spec 005 FR-013).

포크한 데이터베이스였다면 이걸 직접 만들어야 했다. 확장은 물려받는다
([engine/src/interop/mod.rs:3](../../engine/src/interop/mod.rs#L3)).

---

## 2. RLS

### `og_enable_rls(graph text, type_name text, policy_expr text) RETURNS void`

정의: [engine/src/interop/mod.rs:18](../../engine/src/interop/mod.rs#L18) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값

**무엇을 하는가**: 타입과 **모든 서브타입**의 저장 테이블에 RLS를 켜고 `og_policy` 정책을 만든다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 타입 이름 |
| `policy_expr` | `text` | 필수 | — | ⚠️ **SQL 불리언 표현식.** 타입 테이블의 **물리 컬럼 이름**으로 작성 |

**반환**: 없음.

**실행되는 SQL** ([interop/mod.rs:24](../../engine/src/interop/mod.rs#L24)):

```sql
ALTER TABLE <table> ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS og_policy ON <table>;
CREATE POLICY og_policy ON <table> USING (<policy_expr>);
```

**필수**: `policy_expr`에는 **물리 컬럼 이름**(`p_tenant_id`)을 쓴다. 선언 이름
(`tenantId`)이 아니다. 매핑 규칙은 [01_graph_ddl.md §4](01_graph_ddl.md) 참조,
또는 `og_property_view`에서 확인:

```sql
SELECT property, column_name FROM og_property_view
 WHERE graph = 'default' AND type_name = 'Document';
```

**예제**

```sql
SELECT og_add_property('default', 'Document', 'tenant_id', 'int');
SELECT og_enable_rls('default', 'Document',
                     'p_tenant_id = current_setting(''app.tenant'')::int');
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 그래프/타입 없음 | `graph '…' does not exist` / `type '…' does not exist. did you mean: …` |
| RLS 활성화 실패 | `failed to enable RLS on <table>: <e>` ([interop/mod.rs:25](../../engine/src/interop/mod.rs#L25)) |
| 정책 표현식 오류 | `failed to create policy on <table>: <e>` ([interop/mod.rs:30](../../engine/src/interop/mod.rs#L30)) |

> 🔒 **`policy_expr`는 SQL 텍스트로 그대로 보간된다.** 신뢰할 수 없는 입력을
> 넣지 말 것 → [12_improvements_api.md](12_improvements_api.md) **API-05**.

> ⚠️ **끄는 함수가 없다.** `ALTER TABLE ... DISABLE ROW LEVEL SECURITY`를 직접
> 실행해야 한다. `og_interop_report`가 어느 타입에 RLS가 켜져 있는지 알려준다.

> ⚠️ **CSR 순회는 RLS를 우회한다.** `og_csr_reach` / `og_csr_hops`는 힙을 떠나므로
> RLS를 전혀 참조하지 않는다([05_traversal_and_stats.md §4](05_traversal_and_stats.md)).

---

## 3. PostgREST / RPC 진입점

### `og_cypher_json(graph text, query text, params jsonb DEFAULT '{}') RETURNS jsonb`

정의: [engine/src/interop/mod.rs:35](../../engine/src/interop/mod.rs#L35) · 휘발성: 기본값 · 병렬: 기본값

한 번의 호출로 JSON 배열 하나를 돌려준다. 상세는 [03_cypher.md §2](03_cypher.md).

```bash
curl -X POST "$SUPABASE_URL/rest/v1/rpc/og_cypher_json" \
  -H "apikey: $KEY" -H "Content-Type: application/json" \
  -d '{"graph":"default","query":"MATCH (p:Person) RETURN p.name AS name LIMIT 3","params":{}}'
```

---

## 4. 관계형 테이블 매핑

### `og_map_table(graph text, source_table text, type_name text, id_column text, property_map jsonb) RETURNS void`

정의: [engine/src/interop/mod.rs:60](../../engine/src/interop/mod.rs#L60) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 기존 테이블을 **복사 없이** 노드 타입으로 노출한다. 생성된 타입 테이블과 같은 모양의 **뷰**를 만들어 Cypher 컴파일러가 네이티브 저장소처럼 다루게 한다(FR-006..FR-009).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `source_table` | `text` | 필수 | — | ⚠️ **SQL 식별자로 그대로 보간된다** (`FROM <source_table>`) |
| `type_name` | `text` | 필수 | — | 이미 존재해야 하는 타입 이름 |
| `id_column` | `text` | 필수 | — | ⚠️ **SQL 식으로 그대로 보간된다.** `int8`로 캐스트 가능해야 함 |
| `property_map` | `jsonb` | 필수 | — | `{"graphProperty": "sourceColumn"}` 객체 |

**반환**: 없음.

**생성되는 뷰** ([interop/mod.rs:98](../../engine/src/interop/mod.rs#L98)):

```sql
DROP TABLE IF EXISTS og_data.n_<tid> CASCADE;
CREATE VIEW og_data.n_<tid> AS
SELECT (og_make_id(0, <tid>, (<id_column>)::int8)) AS id,
       (<source_col>)::<declared_type> AS p_<prop>,
       ...
       NULL::jsonb AS __ext
  FROM <source_table>;
```

**부수 효과**
- `og_catalog.mapping`에 upsert (`writable = false`).
- `og_data`의 모든 별칭 뷰를 재생성 (`drop_all_views()`, [interop/mod.rs:114](../../engine/src/interop/mod.rs#L114)).

**필수 순서**: 매핑하려는 프로퍼티는 **먼저 선언**되어야 한다 —
`declare property '<prop>' on '<type_name>' before mapping a column to it`
([interop/mod.rs:89](../../engine/src/interop/mod.rs#L89)).

**예제**

```sql
SELECT og_create_graph('crm');
SELECT og_create_type('crm', 'Customer', 'entity');
SELECT og_add_property('crm', 'Customer', 'name',  'string');
SELECT og_add_property('crm', 'Customer', 'email', 'string');

SELECT og_map_table('crm', 'public.customers', 'Customer', 'customer_id',
                    '{"name": "full_name", "email": "email_address"}'::jsonb);

SELECT og_cypher('crm', 'MATCH (c:Customer) RETURN c.name LIMIT 5');
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| `property_map`이 객체가 아님 | `property_map must be a JSON object of { "graphProperty": "sourceColumn" }` ([interop/mod.rs:71](../../engine/src/interop/mod.rs#L71)) |
| 값이 문자열이 아님 | `property_map values must be column names` ([interop/mod.rs:79](../../engine/src/interop/mod.rs#L79)) |
| 프로퍼티 미선언 | `declare property '<p>' on '<t>' before mapping a column to it` |
| 추상 타입 | `'<type_name>' is abstract and cannot back a mapping` ([interop/mod.rs:96](../../engine/src/interop/mod.rs#L96)) |
| 뷰 생성 실패 | `failed to create mapping view: <e>` ([interop/mod.rs:102](../../engine/src/interop/mod.rs#L102)) |

> 🔒 **`source_table`과 `id_column`은 SQL로 보간된다** — 신뢰 경계 밖의 입력을
> 넣지 말 것 → [12_improvements_api.md](12_improvements_api.md) **API-05**.

> ⚠️ **`DROP TABLE IF EXISTS <table> CASCADE`가 먼저 실행된다**
> ([interop/mod.rs:97](../../engine/src/interop/mod.rs#L97)). 이미 데이터가 들어 있는
> 네이티브 타입에 `og_map_table`을 부르면 **그 데이터가 사라진다.** 경고나 확인이
> 없다 → [12_improvements_api.md](12_improvements_api.md) **API-23**.

> ⚠️ 매핑된 타입은 **읽기 전용**이다(`writable = false`). Cypher `CREATE`로
> 쓰려 하면 뷰에 대한 INSERT가 실패한다.

### `og_materialize_mapping(graph text, type_name text) RETURNS int8`

정의: [engine/src/interop/mod.rs:120](../../engine/src/interop/mod.rs#L120) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 매핑 뷰를 네이티브 저장소로 전환한다(spec 005 FR-010). 같은 Cypher 질의, 같은 결과, 네이티브 성능.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 매핑된 타입 이름 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int8` | 아니오 | 물질화된 행 수 |

**수행 절차** ([interop/mod.rs:138](../../engine/src/interop/mod.rs#L138)):
1. `CREATE TABLE <table>_mat AS SELECT * FROM <table>`
2. `DROP VIEW <table> CASCADE`
3. `ALTER TABLE <table>_mat RENAME TO <table>`
4. `ALTER TABLE <table> ADD PRIMARY KEY (id)`
5. `og_data.og_node`에 id 등록 (`ON CONFLICT DO NOTHING`)
6. `og_catalog.mapping.writable = true`
7. 별칭 뷰 전체 재생성

**예제**

```sql
SELECT og_materialize_mapping('crm', 'Customer');   -- 12043
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 저장소 없음 | `no storage for '<type_name>'` ([interop/mod.rs:124](../../engine/src/interop/mod.rs#L124)) |
| 이미 네이티브 | `'<type_name>' is already stored natively` ([interop/mod.rs:135](../../engine/src/interop/mod.rs#L135)) |
| 물질화 실패 | `materialisation failed: <e>` |
| 이름 변경 실패 | `rename failed: <e>` |

> ⚠️ 3~7단계 중 여러 SPI 호출이 `.ok()`로 실패를 무시한다
> ([interop/mod.rs:144](../../engine/src/interop/mod.rs#L144), [:155](../../engine/src/interop/mod.rs#L155),
> [:160](../../engine/src/interop/mod.rs#L160), [:165](../../engine/src/interop/mod.rs#L165)).
> PRIMARY KEY 추가나 레지스트리 등록이 실패해도 함수는 성공을 반환한다
> → [12_improvements_api.md](12_improvements_api.md) **API-23**.

### `og_interop_report(graph text) RETURNS jsonb`

정의: [engine/src/interop/mod.rs:171](../../engine/src/interop/mod.rs#L171) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 그래프가 SQL에 어떻게 노출되어 있는지 한 번에 보고한다.

**반환 구조** ([interop/mod.rs:211](../../engine/src/interop/mod.rs#L211))

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `relational_views[]` | array | 하드코딩된 문자열 4개: `og_node_view`, `og_edge_view`, `og_type_view`, `og_property_view` |
| `rpc_entry_point` | text | `"og_cypher_json(graph, query, params)"` |
| `sql_bridge` | text | `"og_cypher_sql(graph, query) returns embeddable SQL"` |
| `mapped_types[]` | array | `{type, source_table, id_column, writable}` |
| `row_level_security[]` | array | `{type, rls: true}` — RLS가 켜진 타입만 |

> ⚠️ `relational_views`는 하드코딩이라 실제로 존재하는 `og_role_view`,
> `og_typeql_attribute`, `og_typeql_role`이 빠져 있다
> ([interop/mod.rs:213](../../engine/src/interop/mod.rs#L213)).

**예제**

```sql
SELECT jsonb_pretty(og_interop_report('crm'));
```

---

## 5. 상호운용 뷰

BI/ETL 도구가 평문 SQL로 그래프를 읽게 하는 뷰들(spec 005 FR-011).

```sql
SELECT * FROM og_node_view     WHERE graph = 'default' LIMIT 10;
SELECT * FROM og_edge_view     WHERE graph = 'default' LIMIT 10;
SELECT * FROM og_type_view     WHERE graph = 'default';
SELECT * FROM og_property_view WHERE graph = 'default';
SELECT * FROM og_role_view     WHERE graph = 'default';
```

컬럼 정의는 [00_index.md §4.10](00_index.md).

개별 엔티티를 JSON으로 읽으려면:

```sql
SELECT og_node_json(412316860417);
SELECT og_edge_json(549755813889);
SELECT og_prop(412316860417, 'name');   -- text
```

---

## 6. RDF / OWL (spec 006 — **partial**)

**결정(Decision)**: 하나의 코어 모델, 가장자리의 어댑터(헌법 원칙 VI).
RDF는 spec 002의 타입 시스템 위로 **매핑**된다 — 자기 저장소를 갖지 않는다.
매핑되지 않는 것은 `og_data.og_triple_overflow`에 **원문 그대로 보존**되고
보고된다. 트리플을 조용히 버리는 것이 왕복에 대한 신뢰를 파괴하는 실패 양식이기
때문이다(FR-010, [engine/src/adapters/mod.rs:3](../../engine/src/adapters/mod.rs#L3)).

### `og_add_prefix(prefix text, iri text) RETURNS void`

정의: [engine/src/adapters/mod.rs:17](../../engine/src/adapters/mod.rs#L17) · 휘발성: 기본값 · 병렬: 기본값

네임스페이스 접두사를 등록한다. `og_catalog.prefix`에 upsert.
**그래프 인자가 없다 — 전역이다.**

```sql
SELECT og_add_prefix('foaf', 'http://xmlns.com/foaf/0.1/');
```

### `og_set_iri(graph text, type_name text, iri text) RETURNS void`

정의: [engine/src/adapters/mod.rs:28](../../engine/src/adapters/mod.rs#L28) · 휘발성: 기본값 · 병렬: 기본값

기존 타입을 IRI에 묶어 RDF에서 주소 지정 가능하게 한다.
`og_catalog.type.iri`를 갱신한다.

```sql
SELECT og_set_iri('default', 'Person', 'http://xmlns.com/foaf/0.1/Person');
```

**실패 조건**: 그래프/타입 없음.

### `og_load_rdf(graph text, document text, format text DEFAULT 'turtle') RETURNS jsonb`

정의: [engine/src/adapters/mod.rs:40](../../engine/src/adapters/mod.rs#L40) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: RDF 문서를 그래프에 적재한다(spec 006 FR-004, FR-011..FR-015).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `document` | `text` | 필수 | — | RDF 문서 전문 |
| `format` | `text` | 선택 | `'turtle'` | `turtle` \| `ttl` \| `ntriples` \| `nt` \| `n3` (대소문자 무관) |

**반환 구조** ([engine/src/adapters/rdf.rs:586](../../engine/src/adapters/rdf.rs#L586))

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `triples_read` | int | 파싱된 트리플 수 |
| `classes` | int | `owl:Class` / `rdfs:Class` → 엔티티 타입 |
| `object_properties` | int | `owl:ObjectProperty` → 관계 타입 |
| `datatype_properties` | int | `owl:DatatypeProperty` |
| `subclass_axioms` | int | `rdfs:subClassOf` → 상속 |
| `instances` | int | 생성된 인스턴스 노드 |
| `facts` | int | 생성된 엣지 |
| `inference_rules` | int | `owl:TransitiveProperty` / `owl:SymmetricProperty` → `og_add_rule` |
| `unmapped` | int | 오버플로로 보존된 트리플 수 |
| `note` | text | `unmapped > 0`이면 `"unmapped triples were preserved verbatim — see og_mapping_report(graph)"`, 아니면 `"all triples mapped"` |

**오버플로 사유 (`reason` 컬럼에 저장되는 4가지, [rdf.rs:527](../../engine/src/adapters/rdf.rs#L527) 이하)**

| 사유 |
|---|
| `non-IRI subject or predicate` |
| `subject has no rdf:type in this document` |
| `object IRI is not an instance in this document` |
| `blank node object` |

**예제**

```sql
SELECT jsonb_pretty(og_load_rdf('ont', $rdf$
@prefix ex:   <http://example.org/> .
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

ex:Animal a owl:Class .
ex:Dog    a owl:Class ; rdfs:subClassOf ex:Animal .
ex:rex    a ex:Dog .
$rdf$, 'turtle'));
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 지원하지 않는 포맷 | `unsupported RDF format '<f>' (turtle \| ntriples)` ([rdf.rs:383](../../engine/src/adapters/rdf.rs#L383)) |
| 파싱 실패 | `RDF parse error: <msg>` ([rdf.rs:386](../../engine/src/adapters/rdf.rs#L386)) |
| 그래프 없음 | `graph '<g>' does not exist` |

> ⚠️ 접두사 등록은 `.ok()`로 실패를 무시한다([rdf.rs:397](../../engine/src/adapters/rdf.rs#L397)).

### `og_dump_rdf(graph text, format text DEFAULT 'turtle') RETURNS text`

정의: [engine/src/adapters/mod.rs:47](../../engine/src/adapters/mod.rs#L47) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 그래프를 RDF로 직렬화한다(spec 006 FR-005). 매핑되지 않은 트리플도 다시 내보낸다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `format` | `text` | 선택 | `'turtle'` | `ntriples` \| `nt` 이면 N-Triples, **그 외 모든 값은 Turtle** |

> ⚠️ `og_load_rdf`와 달리 **포맷을 검증하지 않는다.** 오타(`'turtl'`)는 조용히
> Turtle이 된다([rdf.rs:697](../../engine/src/adapters/rdf.rs#L697))
> → [12_improvements_api.md](12_improvements_api.md) **API-24**.

**반환**: RDF 텍스트. Turtle이면 `@prefix` 선언이 앞에 붙는다.
IRI가 없는 타입은 `urn:og:<name>` 으로 합성된다
([rdf.rs:719](../../engine/src/adapters/rdf.rs#L719)).

**예제**

```sql
SELECT og_dump_rdf('ont', 'turtle');
SELECT og_dump_rdf('ont', 'ntriples');
```

### `og_mapping_report(graph text) RETURNS jsonb`

정의: [engine/src/adapters/mod.rs:53](../../engine/src/adapters/mod.rs#L53) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 무엇이 매핑되지 않았고 왜 그런지 보고한다(FR-010, FR-015).

**반환 구조**

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `unmapped_triples` | int | 오버플로 테이블의 **전체** 개수 |
| `sample[]` | array | `{subject, predicate, object, reason}` — **최대 200개** ([adapters/mod.rs:60](../../engine/src/adapters/mod.rs#L60)) |
| `note` | text | `"unmapped triples are preserved verbatim and re-emitted on export"` |

**예제**

```sql
SELECT og_mapping_report('ont') -> 'unmapped_triples';
SELECT jsonb_pretty(og_mapping_report('ont') -> 'sample');
```

> ⚠️ 이름이 비슷한 `og_interop_report`(§4)와 **전혀 다른 것**을 보고한다.
> 전자는 관계형 노출, 후자는 RDF 오버플로 → API-02.

---

## 7. 사실 — SPARQL은 없다

`grep -rn -i 'sparql' engine/src/` 결과는 [engine/src/lib.rs:15](../../engine/src/lib.rs#L15)의
모듈 표 주석 한 줄뿐이다. **파서도, 진입 함수도, 스텁도 없다.**

README 스펙 상태표는 spec 006을 "partial — RDF load & dump, OWL→type-hierarchy
mapping, overflow fidelity. SPARQL not yet"로 표기한다([README.md:148](../../README.md#L148)).

> ⚠️ SPARQL을 시도하면 **함수 자체가 없어서** PostgreSQL의
> `function og_sparql(...) does not exist` 오류가 난다. 제품이 그 언어를
> 의도적으로 미지원한다는 안내가 없다 → [12_improvements_api.md](12_improvements_api.md) **API-25**.
> SHACL도 마찬가지로 코드에 존재하지 않는다.

---

## 8. 금지 / 필수

- 🔒 **금지**: `og_enable_rls(policy_expr)`, `og_map_table(source_table, id_column)`에
  신뢰할 수 없는 입력을 넣지 말 것 — SQL로 보간된다.
- **금지**: 데이터가 들어 있는 네이티브 타입에 `og_map_table`을 부르지 말 것 —
  저장 테이블이 `DROP TABLE ... CASCADE`된다.
- **금지**: RLS를 켠 뒤 `og_csr_*` 순회를 권한 경계로 신뢰하지 말 것.
- **금지**: SPARQL / SHACL 진입점을 찾지 말 것 — 존재하지 않는다.
- **필수**: RLS 정책은 **물리 컬럼 이름**(`p_*`)으로 작성할 것.
  `og_property_view`에서 확인할 것.
- **필수**: 매핑할 프로퍼티는 `og_map_table` **이전에** `og_add_property`로 선언할 것.
- **필수**: `og_load_rdf` 후 `og_mapping_report(graph)`로 `unmapped_triples`를
  반드시 확인할 것. 0이 아니면 왕복이 완전하지 않다.
- **필수**: `og_dump_rdf`의 `format`은 오타를 검증하지 않으므로 정확히
  `'turtle'` 또는 `'ntriples'`를 쓸 것.

---

## 9. 관련 문서

- Cypher 진입 → [03_cypher.md](03_cypher.md)
- CSR 순회의 RLS 우회 → [05_traversal_and_stats.md §4](05_traversal_and_stats.md)
- 타입/프로퍼티 선언 → [01_graph_ddl.md](01_graph_ddl.md)
- 원문 요약 → [docs/api.md:144](../../docs/api.md), [docs/architecture.md:273](../../docs/architecture.md)

<!-- affects: api, backend, data, security -->
<!-- requires-update: 02_api/12_improvements_api.md -->
