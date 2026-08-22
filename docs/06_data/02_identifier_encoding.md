# 02. 64비트 식별자 인코딩

> **이 문서가 답하는 질문**
> - 노드/엣지 id의 64비트는 어떻게 나뉘어 있는가?
> - 왜 `type_id`를 id 안에 박았는가? 그 대가는 무엇인가?
> - id는 언제 발급되고, 재사용되는가?
> - 공간이 언제 고갈되고, 고갈되면 무슨 일이 일어나는가?

**정본**: [`engine/src/id.rs`](../../engine/src/id.rs) (112줄),
발급은 [`engine/src/storage/mod.rs:24-34`](../../engine/src/storage/mod.rs).

---

## 사실 — 비트 배치

```text
 bit 63        54           36                              0
 +---+----------+------------+-------------------------------+
 | 0 | shard: 9 | type_id:18 |          local_id: 36         |
 +---+----------+------------+-------------------------------+
```
(`engine/src/id.rs:3-8`)

| 필드 | 비트 수 | 시프트 | 최댓값 | 상수 |
|---|---|---|---|---|
| (부호) | 1 | 63 | 항상 0 | — |
| `shard` | 9 | 54 | 511 | `SHARD_BITS`, `SHARD_SHIFT` (`id.rs:18,21`) |
| `type_id` | 18 | 36 | 262,143 | `TYPE_BITS`, `TYPE_SHIFT` (`id.rs:17,20`) |
| `local_id` | 36 | 0 | 68,719,476,735 (약 687억) | `LOCAL_BITS` (`id.rs:16`) |

합이 63이라는 사실은 단위 테스트가 지킨다:
```rust
assert_eq!(LOCAL_BITS + TYPE_BITS + SHARD_BITS, 63);
```
(`engine/src/id.rs:108`)

**최상위 비트가 항상 0**이므로 모든 식별자는 **양수 `int8`**이다.
같은 테스트가 `assert!(id > 0)`으로 이를 못 박는다(`engine/src/id.rs:103`).
이것은 미적 취향이 아니라 요구사항이다 — 클라이언트가 id를 부호 있는 64비트 정수로
받고(JSON, Bolt, JDBC), 음수가 나오면 라운드트립이 깨진다.

---

## 결정 1 — `type_id`를 id에 박는다

**무엇을 얻는가**: 어떤 id에 대해서든 타입을 **시프트와 마스크 한 번**으로 안다.
카탈로그 조인도, `og_node` 프로브도 필요 없다.

```rust
pub fn id_type(id: i64) -> i32 { ((id >> TYPE_SHIFT) & TYPE_MASK) as i32 }
```
(`engine/src/id.rs:47-50`)

이것이 실제로 쓰이는 곳:

| 호출부 | 무엇을 위해 |
|---|---|
| `set_node_props_inner` (`engine/src/storage/mod.rs:299`) | 어느 테이블에 UPDATE할지 결정 |
| `delete_edge_inner` (`engine/src/storage/mod.rs:502`) | 인접 세그먼트의 `etype` 결정 |
| `validate_roles` (`engine/src/storage/mod.rs:475`) | 역할 참여자의 타입 검증 |
| `og_similar` (`engine/src/vector/mod.rs:165`) | 임베딩 메타 조회 |
| TypeQL 소유권 유일성 검사 (`engine/src/typeql/write.rs:321-323`) | **SQL 안에서** `((e.src >> 36) & mask)::int4 = ANY($3)` |

마지막 항목이 중요하다. 타입 필터가 **조인 없이 산술식**으로 SQL에 들어간다.
`type_id`가 id 밖에 있었다면 저 자리는 `og_node`와의 조인이었을 것이다.

**대가**
1. **`type_id`는 절대 재사용할 수 없다.** 재사용하면 이미 저장된 id들이 다른 타입을
   가리키게 된다. 그래서 `og_catalog.type_id_seq`는 되감기지 않고,
   `og_drop_type()`은 id를 반납하지 않는다(`engine/src/catalog/types.rs:685-711`).
2. **타입 재분류가 불가능하다.** 노드의 타입을 바꾸려면 id가 바뀌어야 하고,
   id가 바뀌면 `og_adj`의 모든 이웃 배열과 `og_edge.src/dst`를 다시 써야 한다.
   이 저장소에 "노드의 타입을 바꾸는" 함수는 없다.
3. **타입 수의 상한이 하드하다** — 262,143. 아래 참조.

---

## 결정 2 — shard 비트는 미리 잡아두되 지금은 쓰지 않는다

```rust
id::make_id(0, type_id, local)
```
(`engine/src/storage/mod.rs:33`)

**모든 id의 shard 비트는 0이다.** 발급 경로가 상수 0을 넘긴다.
`with_shard()`(`engine/src/id.rs:64-67`)가 재배치용으로 존재하지만
`#[pg_extern]`으로 노출되지 않았고 호출부도 없다.

**왜 미리 잡아두는가**: spec 007(분산 클러스터)이 나중에 샤딩을 붙일 때
식별자 포맷을 바꾸지 않기 위해서다(`engine/src/id.rs:11-12`,
"the shard bits are reserved up front so spec 007 can distribute without rewriting identifiers").
스펙 상태표상 007은 "read replica만, 샤딩은 설계만"이다.

**현재 상태의 의미**: 이 9비트는 지금 **순수한 낭비**다.
지역 공간에 붙였다면 45비트(35조)가 되었을 것이다. 이는 의도된 선불이다.

---

## 사실 — 발급 경로

```rust
pub fn alloc_id(type_id: i32) -> i64 {
    let local = crate::spiu::one_mut::<i64>(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 2)
         ON CONFLICT (type_id) DO UPDATE SET next_id = og_id_alloc.next_id + 1
         RETURNING next_id - 1",
        &[type_id.into()],
    ).expect("id allocation failed").unwrap();
    id::make_id(0, type_id, local)
}
```
(`engine/src/storage/mod.rs:24-34`)

성질:

| 성질 | 값 | 근거 |
|---|---|---|
| 첫 지역 id | 1 (`local = 0`은 절대 발급되지 않음) | `VALUES ($1, 2) ... RETURNING next_id - 1` |
| 단조 증가 | 예 | `next_id + 1` |
| 삭제 시 반납 | **아니오** | 반납 코드 없음 |
| 롤백 시 반납 | **예** (UPDATE가 롤백됨) | 트랜잭션 안의 UPSERT |
| 동시성 | **행 단위 직렬화** | 같은 `type_id` 행에 대한 `DO UPDATE`가 커밋까지 락을 잡는다 → `DATA-01` |

`og_id_alloc` 행은 타입 생성 시 `next_id = 1`로 미리 만들어진다
(`engine/src/catalog/types.rs:444-449`, `engine/src/typeql/schema.rs:545-549`).

**주의**: 이 발급기는 "살아 있는 개수"가 아니라 **"지금까지 만든 총 개수"**를 센다.
1억 개를 만들고 1억 개를 지운 타입의 다음 id는 100,000,001이다.

---

## 사실 — 오버플로 동작

`make_id`는 세 필드를 전부 범위 검사하고, 벗어나면 `error!`로 즉시 중단한다
(pgrx에서 `ereport(ERROR)` → 트랜잭션 abort).

```rust
if !(0..=MAX_SHARD_ID).contains(&shard)  { error!("shard id {shard} out of range (0..{MAX_SHARD_ID})"); }
if !(0..=MAX_TYPE_ID).contains(&type_id) { error!("type id {type_id} out of range (0..{MAX_TYPE_ID})"); }
if !(0..=MAX_LOCAL_ID).contains(&local)  { error!("local id {local} exhausted the 36-bit space for this type"); }
```
(`engine/src/id.rs:34-45`)

주석이 이유를 명시한다: "Panics ... on overflow so a silently truncated id can never
reach storage"(`engine/src/id.rs:31-32`). **잘린 id가 저장되는 일은 없다.**

### 고갈 시나리오 1 — 지역 공간 (36비트, 타입당 687억)

`og_id_alloc.next_id`가 68,719,476,736에 도달하면 그 타입에 대한 다음 생성이
`error!`로 실패한다. 실패는 **타입 단위**이므로 다른 타입은 계속 동작한다.

**고갈되면 할 수 있는 일**: 없다. 재압축(compaction) 도구도, id 재발급 도구도 없다.
현실적인 해법은 새 타입을 만들어 옮기는 것이다.

**모니터링**: 전용 함수가 없다. 직접 봐야 한다.
```sql
SELECT a.type_id, t.name, a.next_id,
       round(a.next_id::numeric / 68719476736 * 100, 4) AS pct_used
  FROM og_data.og_id_alloc a
  LEFT JOIN og_catalog.type t USING (type_id)
 ORDER BY a.next_id DESC LIMIT 20;
```

### 고갈 시나리오 2 — 타입 공간 (18비트, 그래프 전체 합산 262,143)

`og_catalog.type_id_seq`는 `MAXVALUE 262143`이고 `CYCLE`이 없다(`engine/sql/bootstrap.sql:47`).
소진되면 `nextval()`이 `nextval: reached maximum value of sequence`로 실패하고,
`og_create_type` / 암묵적 라벨 생성이 전부 막힌다.

**이 공간은 DB 전체에서 공유된다** — 그래프별이 아니다. 시퀀스가 하나이기 때문이다.

**위험 요인**: Cypher 쓰기 경로가 **모르는 라벨을 자동으로 타입으로 만든다**
(`engine/src/catalog/types.rs:210-231`, `resolve_or_create_label_set`).
`CREATE INDEX FOR (n:Whatever)`도 마찬가지다(`engine/src/compat/ddl.rs:203`).
라벨을 프로그램적으로 생성하는 클라이언트(예: `CREATE (n:User_{uuid}))`)는
26만 개를 다 태울 수 있고, `og_drop_type()`은 그 id를 돌려주지 않는다.

**모니터링**:
```sql
SELECT last_value, 262143 - last_value AS remaining
  FROM og_catalog.type_id_seq;
```

### 고갈 시나리오 3 — shard (9비트, 512)

현재 항상 0이므로 발생하지 않는다.

---

## 사실 — SQL 표면

| 함수 | 시그니처 | 성질 |
|---|---|---|
| `og_id_type(id int8)` | → `int4` | `immutable, parallel_safe, strict` (`engine/src/id.rs:73-76`) |
| `og_id_shard(id int8)` | → `int4` | 동상 (`id.rs:78-81`) |
| `og_id_local(id int8)` | → `int8` | 동상 (`id.rs:83-86`) |
| `og_make_id(shard int4, type_id int4, local int8)` | → `int8` | 동상 (`id.rs:88-91`) |

`immutable`이라는 점이 중요하다 — **표현식 인덱스와 파티션 키에 쓸 수 있다.**
`og_map_table()`이 이미 이 성질을 이용해 매핑 뷰의 id를 만든다:
```sql
(og_make_id(0, {tid}, ({id_column})::int8)) AS id
```
(`engine/src/interop/mod.rs:74-76`)

벤치마크 하네스도 벌크 로드에서 같은 방식을 쓴다(`bench/harness.py:329`).

---

## 결정 3 — 시퀀스가 아니라 테이블 한 행으로 발급하는 이유와 그 비용

PostgreSQL 시퀀스(`nextval`)는 트랜잭션 락을 잡지 않는다. `og_id_alloc`의 `UPDATE`는
**커밋까지 행 락을 잡는다**. 따라서:

- 같은 타입의 노드를 만드는 두 트랜잭션은 **완전히 직렬화된다.**
  두 번째는 첫 번째가 커밋/롤백할 때까지 블록된다. 문장 단위가 아니라 트랜잭션 단위다.
- 대신 **롤백하면 id가 반납된다** — 시퀀스에는 없는 성질이다.

이 트레이드오프는 코드나 주석에 명시되어 있지 않다(`engine/src/storage/mod.rs:22-34`에
동시성에 대한 언급이 없다). 개선안은 [`10_improvements_data.md`](10_improvements_data.md) `DATA-01`.

---

## 금지 / 필수

**금지**
- `og_catalog.type_id_seq`를 되감는 것(`setval`). 이미 저장된 id가 다른 타입을 가리키게 된다.
- `og_data.og_id_alloc.next_id`를 감소시키는 것. 중복 id를 발급하게 된다.
- id를 애플리케이션이 직접 조립해 `og_data.*`에 넣는 것.
  **예외**: 벌크 로드는 `og_make_id()`를 써야 하며, 그때 `og_id_alloc.next_id`를
  로드한 최대 지역 id + 1로 **반드시 함께 갱신**해야 한다.
  (벤치 하네스는 읽기 전용 벤치라서 이 갱신을 하지 않는다 — `bench/harness.py:327-355`.
  운영 로더가 그대로 흉내 내면 id가 충돌한다.)
- 프로그램적으로 생성한 문자열을 Cypher 라벨로 쓰는 것. 18비트 타입 공간을 태운다.

**필수**
- 벌크 로드 후:
  ```sql
  INSERT INTO og_data.og_id_alloc (type_id, next_id)
  SELECT type_id, max(og_id_local(id)) + 1 FROM og_data.og_node GROUP BY type_id
  ON CONFLICT (type_id) DO UPDATE
     SET next_id = GREATEST(og_id_alloc.next_id, EXCLUDED.next_id);
  ```
- 대량 삽입을 병렬화하려면 **타입을 나누어** 병렬화할 것. 같은 타입이면 직렬화된다.

---

<!-- affects: data, backend, ops -->
<!-- requires-update: docs/06_data/08_data_lifecycle.md, docs/06_data/10_improvements_data.md -->
