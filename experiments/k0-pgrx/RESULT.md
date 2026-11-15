# K0: 공용 크레이트의 pgrx 함수를 두 확장이 각자 내보낼 수 있는가 (pgrx 0.19.2)

작성 2026-11-16. 가지 `seam/k0-pgrx`(`v2/main` 214e66c 에서 가름). 카드는 `reports/ontological-v2/P1-CARDS.md` 의 K0, 배경은 `DESIGN.md` 5.2·5.3절.

## 판정: (가) 작업 공간 + 크레이트 여럿

**된다.** 일반 라이브러리 크레이트(rlib)에 둔 `#[pg_extern]`, `extension_sql!`, `extension_sql_file!` 은 그 크레이트를 링크한 확장 크레이트(cdylib)마다 설치 SQL 에 그대로 들어간다. 두 확장은 한 DB 에 함께 설치되고, 같은 SQL 이름·같은 C 심볼 이름의 함수가 서로 다른 스키마에 놓여 각자의 `.so` 로 간다. PG16.15·PG17.12 모두에서 확인했다. 엔진 실제 코드(`engine/src` 를 고치지 않고 라이브러리로 컴파일)로도 같은 결과다.

「크레이트 하나 + cargo 기능 둘」은 만들지 않았다(가가 되므로). 아래 「(나) 를 버리는 이유」에 cargo-pgrx 가 그 길을 어렵게 만드는 곳을 적었다.

핵심 근거:

1. pgrx 0.19.2 는 SQL 엔티티를 **컴파일된 공유 객체의 링커 섹션**에서 읽는다. 매크로마다 `#[used]` 정적 바이트열을 `__DATA,__pgrxsc`(macOS)·`.pgrxsc`(ELF) 섹션에 둔다(`pgrx-sql-entity-graph-0.19.2/src/section.rs:188-196`). `cargo pgrx schema` 는 확장의 `.so` 를 읽어 그 섹션의 엔티티를 전부 쓴다(`cargo-pgrx-0.19.2/src/command/schema.rs:240-249`). 크레이트로 거르는 단계가 없다. 그래서 어느 크레이트에서 정의했든 링크되어 들어간 것은 모두 나온다.
2. 실제로: 장난감 확장 둘이 공용 크레이트의 함수 6개·SQL 2개·스키마 1개를 각자 설치 SQL 에 실었다(`results/pg16/0-schema.out`, `install-*.sql`). 엔진 라이브러리를 감싼 확장 둘이 엔진의 122개 엔티티(함수 119, SQL 파일 3)를 각자 실었다(`results/pg16/engine-0-build.out`).
3. v1 껍데기 확장이 낸 설치 SQL 은 지금 `engine/` 이 내는 것과 **주석 라인만 다르다**(3,233 라인 중 482 라인이 다르고 전부 `-- ` 주석, 순서도 같음). 위치 주석 두 종류를 지우면 바이트까지 같다.
4. 한 DB·한 백엔드에 두 `.so` 가 함께 올라가고(백엔드의 메모리 맵에 둘 다 보임), `public.og_cypher` 는 `$libdir/ontological`, `oga_catalog.og_cypher` 는 `$libdir/ontological_age` 로 간다.

## 재현

```bash
# PostgreSQL 은 쓸 수 있는 사본이어야 한다(확장 파일을 복사해 넣는다). PG16 은 Studio runtime/pg 를 복사해서 썼다.
cp -R <Studio>/runtime/pg  $SCRATCH/pg16
cp -R <p0b>/pg17           $SCRATCH/pg17          # pgvector 0.8.7 이 들어 있는 트리

# cargo-pgrx 0.19.2 가 PATH 에. 빌드 캐시를 다시 쓰려면 그 캐시를 만든 PGRX_HOME·pg_config 를 그대로 준다.
export PATH=<cargo-pgrx 가 있는 곳>:$PATH CARGO_TARGET_DIR=<p0a>/target-engine MACOSX_DEPLOYMENT_TARGET=13.0
export PGRX_HOME=<p0a>/pgrx-home                  # pg17 은 <p0b>/work-v1-pg17/pgrx-home

experiments/k0-pgrx/run.sh        --pg 16 --pg-dir $SCRATCH/pg16 --build-pg-config <Studio>/runtime/pg/bin/pg_config
experiments/k0-pgrx/run-engine.sh --pg 16 --pg-dir $SCRATCH/pg16 --build-pg-config <Studio>/runtime/pg/bin/pg_config
experiments/k0-pgrx/run.sh        --pg 17 --pg-dir $SCRATCH/pg17
experiments/k0-pgrx/run-engine.sh --pg 17 --pg-dir $SCRATCH/pg17
```

- 둘 다 포트 28820 에 임시 클러스터를 띄우고(TCP 만, 소켓 파일 없음) 끝나면 멈추고 데이터 폴더를 지운다. 기대 결과를 PASS/FAIL 로 찍는다. 이번 실행: 장난감 17/17, 엔진 15/15, PG16·PG17 모두.
- 걸린 시간(캐시가 있을 때): 장난감 약 20초, 엔진 약 25초~2분(엔진 라이브러리 컴파일 + fat LTO 두 번).
- 출력은 `results/pg16/`, `results/pg17/`. `0-schema-dev-profile.out` 은 dev 프로필(LTO 없음, rlib 를 아카이브로 링크)에서 같은 개수가 나오는지 따로 본 기록이다(명령은 파일 머리에).
- `engine/` 아래는 쓰지 않는다. `run-engine.sh` 는 `engine/` 에서 `cargo pgrx schema` 를 돌리지만 cargo 는 `CARGO_TARGET_DIR` 에만 쓴다.

## 장난감 크레이트로 본 것 (`run.sh`)

구성: `og_shared`(rlib, og-pg-common 자리) + `ext_a`(cdylib, v1 자리: control 에 `schema` 없음) + `ext_b`(cdylib, v2 자리: control `schema = 'ext_b_s'`).

| 확인 | 결과 |
|---|---|
| 공용 크레이트의 `#[pg_extern]` 이 두 확장의 설치 SQL 에 | 나온다. `ext_a`·`ext_b` 모두 13개 엔티티(공용 함수 6 + 자기 함수 2, SQL 4, 스키마 1) |
| 공용 크레이트의 `extension_sql!`·`extension_sql_file!` | 나온다. 확장 크레이트가 정의한 이름(`requires = ["k0_edition_boot"]`)과 공용 함수 경로(`requires = [k0_hello]`)를 크레이트를 넘어 참조해도 순서가 맞게 나온다 |
| 공용 SQL 파일 안의 `@extschema@` | 확장마다 제 스키마로 바뀐다(`schema=public`, `schema=ext_b_s`) |
| 공용 크레이트의 `#[pg_schema] mod k0_fixed` | 두 확장 모두 `CREATE SCHEMA IF NOT EXISTS k0_fixed` 를 낸다 → 둘째 `CREATE EXTENSION` 이 실패: `schema k0_fixed is not a member of extension "ext_b"` |
| 의존성으로 적기만 하고 코드에서 이름을 부르지 않은 공용 크레이트 | **링크되지 않는다.** 공용 엔티티 0개(`variant-unlinked`) |
| `extern crate og_shared as _;` 한 문장만 | 충분하다. 공용 엔티티 전부(`variant-extern-only`). release(fat LTO)·dev(LTO 없음) 모두 |
| 한 DB 에 두 확장, 다른 스키마 | 된다. `ext_a` → `public`, `ext_b` → `ext_b_s`. 설치 순서를 바꿔도 된다 |
| 같은 SQL 이름·같은 C 심볼(`k0_hello_wrapper` 등) | `pg_proc.probin` 이 `$libdir/ext_a`·`$libdir/ext_b` 로 갈리고 부를 때마다 제 `.so` 의 코드가 돈다(`dladdr` 로 확인) |
| 두 `.so` 가 한 백엔드에 | 둘 다 메모리 맵에 있다. 정적 변수도 `.so` 마다 따로다(공용 크레이트의 `EDITION` 이 `ext_a`/`ext_b` 로 각각 보임) |
| 두 확장을 같은 스키마에 | 충돌: `relation "k0_edition_boot" already exists` |
| 공용 코드가 SPI 로 **한정 없이** 부르는 함수 | `search_path` 를 따른다. `ext_b_s.k0_spi_unqualified()` 가 `public.k0_whoami`(ext_a 의 코드)를 불렀다. 스키마를 붙이면(`k0_spi_qualified`) 제 것을 부른다 |
| 공용 코드가 처음 쓸 때 정의하는 GUC | 둘째 `.so` 가 같은 이름을 다시 정의하려다 `ERROR: attempt to redefine parameter "k0.shared_probe"` |

C 심볼이 같아도 되는 까닭: PostgreSQL 은 `pg_proc.probin` 의 라이브러리 핸들로 `dlsym` 한다. Rust cdylib 가 내보내는 심볼은 `#[no_mangle]` 것뿐이다(`Pg_magic_func`, `_PG_init`, `*_wrapper`, `pg_finfo_*_wrapper`, `nm -gU` 로 확인). 나머지 Rust 심볼은 바깥에 보이지 않는다.

## 엔진 실제 코드로 본 것 (`run-engine.sh`)

`engine_lib` 는 `[lib] path = "../../../engine/src/lib.rs"` 로 `engine/src` 를 그대로 컴파일한 라이브러리다. `engine_v1` 은 `extern crate og_engine_lib as _;` 한 문장과 `engine/ontological.control` 사본(스크립트가 `cmp` 로 같은지 확인)뿐인 확장 `ontological`, `engine_age` 는 같은 한 문장과 `schema = 'oga_catalog'` control 의 확장 `ontological_age` 다. 엔진을 쪼갠 것은 아니다: 엔진 전체가 「공용 크레이트」 자리에 있다.

- `engine_v1`·`engine_age` 모두 엔진의 122개 엔티티를 낸다(`engine/` 직접 빌드와 같은 수).
- v1 설치 SQL: `engine/` 의 것과 3,233 라인으로 같고, 다른 482 라인은 모두 pgrx 가 엔티티 위에 쓰는 위치 주석이다. 예: `-- src/cypher/mod.rs:123` → `-- engine_lib/../../../engine/src/cypher/mod.rs:123`, `-- ontological::cypher::og_cypher` → `-- og_engine_lib::cypher::og_cypher`. 이 두 종류를 지우면 같다(`run-engine.sh` 의 `strip_locations`).
- `ontological_age` 의 설치 SQL 은 pgrx 함수 119개만 남기고 `engine/sql/*.sql` 세 덩어리를 뺐다. 그 셋은 `og_catalog`·`og_data`·`og_action` 스키마를 만드는 v1 고유 SQL 이라 두 번 실을 수 없다(K7 에서 v1 쪽에 남을 것).
- 한 DB: `ontological` 119개 C 함수 → `public`, `ontological_age` 119개 → `oga_catalog`. `og_create_graph`·`og_cypher`·`og_cypher_sql`·`ontological_version` 이 양쪽에서 답한다. 같은 쿼리의 `og_cypher_sql` 결과가 양쪽에서 같다.
- `oga_catalog.og_cypher_json` 은 안에서 SPI 로 `og_cypher($1,$2,$3)` 를 한정 없이 부른다(`engine/src/interop/mod.rs:46`). `track_functions = all` 로 세어 보니 그 호출은 `public.og_cypher`(v1 의 `.so`)로 갔다(`calls = 1`). 장난감에서 본 함정이 실제 코드에 있다.
- `oga_catalog.og_create_graph('k0_v2')` 도 v1 의 `og_catalog.graph` 에 썼다. 지금은 두 확장이 같은 v1 코드라 당연하다. v2 저장 계층(`og-store-age`)이 생기면 이 경로가 갈린다.

## pgrx 소스에서 본 근거

- 엔티티 운반: `pgrx-sql-entity-graph-0.19.2/src/section.rs:188-196` (`__pgrx_schema_entry!`: `#[used]` + `link_section`). 섹션 이름 상수는 같은 파일 68-71.
- 스키마 생성: `cargo-pgrx-0.19.2/src/command/schema.rs:212-249` (`generate_schema_implicit`: control 하나 + `.so` 섹션의 엔티티 전부), `301` (`load_section_data`: `CARGO_TARGET_DIR/<profile>/<lib 파일>`).
- 리눅스 빌드: `cargo-pgrx-0.19.2/src/cargo.rs:205-243` (`pgrx_cdylib_rustc_args`: GNU ld 계열이면 최종 cdylib 링크에 `-Wl,--no-gc-sections`). 링크 인자라 모든 크레이트의 섹션에 함께 적용된다.
- 순서 참조: `pgrx-sql-entity-graph-0.19.2/src/pgrx_sql.rs:1088-1130` (`find_positioning_ref_target`: 이름 또는 `module_path.ends_with`). 크레이트 경계를 가리지 않는다.
- `pg_module_magic!`: `pgrx-0.19.2/src/lib.rs:281-285, 320-346` (`#[no_mangle] Pg_magic_func` + 섹션의 표지 엔트리, `name`·`version` 은 **매크로가 펼쳐지는 크레이트의** `CARGO_PKG_NAME`·`CARGO_PKG_VERSION`).
- `_PG_init`: `pgrx-macros-0.19.2/src/rewriter.rs:89-95` (`#[pg_guard] fn _PG_init` 에 `no_mangle` 을 붙인다).
- control 과 이름: `cargo-pgrx-0.19.2/src/command/get.rs:88-113` (`find_control_file`: 매니페스트 폴더에서 **처음 만난** `*.control`, 확장 이름 = 그 파일 이름), `install.rs:199-213` (`module_pathname` 이 있으면 설치되는 `.so` 이름 = 확장 이름. cargo 의 lib 이름과 무관), `install.rs:395` (설치 SQL 이름 `{extname}--{version}.sql`).

## 지금 v1 의 함수는 어느 스키마에 만들어지나 (K8 이 쓸 것)

- `engine/ontological.control` 에 `schema` 가 없고 `relocatable = false` 다. 그래서 `CREATE EXTENSION ontological` 은 그 순간의 생성 스키마(`search_path` 의 첫 스키마, 기본 `public`)를 확장 스키마로 잡고, 한정 없이 쓴 객체를 거기에 만든다. `CREATE EXTENSION ontological SCHEMA x` 로 다른 곳에 둘 수도 있다.
- 설치 SQL 의 함수 152개 가운데 151개(pgrx 119 + 손으로 쓴 32)가 한정 없이 쓰여 확장 스키마로 가고, 1개(`og_catalog.og_refuse_source_write`)만 스키마가 붙어 있다. 스키마 `og_catalog`·`og_data`·`og_action` 은 bootstrap.sql 이 만든다.
- 실측: 시험 DB 에서 `pg_extension.extnamespace` = `public`, C 함수 119개가 `public`.
- 따라서 K8 의 기본값(한정 없이 부름)은 지금과 같고, v2 를 고를 때 `oga_catalog.og_cypher` 처럼 붙이면 된다. 두 판이 한 DB 에 있으면 한정 없는 이름은 `search_path` 를 따른다(보통 `public` 의 v1).

## K7 이 알아야 할 것

1. **확장 크레이트는 공용 크레이트를 이름으로 불러야 한다.** Cargo 의존성만으로는 링크되지 않아 그 크레이트의 함수·SQL 이 설치 SQL 에서 조용히 빠진다(오류 없음). 엔티티를 가진 라이브러리마다 확장 크레이트 `lib.rs` 에 `extern crate og_pg_common as _;` 같은 문장을 둔다. 관문으로 `cargo pgrx schema` 의 「Discovered 122 SQL entities … 119 functions … 3 sqls」 수를 본다.
2. **`pg_module_magic!` 은 `.so` 마다 한 번, 확장 크레이트에.** 라이브러리에 두면 PG18 의 모듈 이름이 라이브러리 이름이 되고, 확장 크레이트에도 두면 `Pg_magic_func` 가 겹친다. `_PG_init` 도 `.so` 마다 하나다.
3. **`bootstrap`·`finalize` 표시는 판마다 하나.** 판 전용 SQL(지금 `engine/sql/*.sql` 중 v1 몫)은 그 판의 확장 크레이트(또는 판 전용 저장 크레이트 하나)에서 `extension_sql_file!` 로 싣는다. 공용 SQL 은 공용 크레이트에서 `requires = ["bootstrap"]` 처럼 판이 정의한 이름을 가리키면 된다(이름이 두 판에서 같으면 각 판에서 제 것으로 풀린다). `extension_sql_file!` 의 경로는 매크로를 부른 소스 파일 기준이라, SQL 파일은 그것을 싣는 크레이트를 따라간다.
4. **공용 크레이트에 `#[pg_schema]` 를 쓰지 않는다.** 스키마 이름이 컴파일 때 정해져 두 판이 같은 스키마를 만들다 충돌한다. 판의 스키마는 control 의 `schema` 로 정한다(v1 은 지금처럼 없음, v2 는 `oga_catalog`). 엔진에는 지금 `#[pg_schema]` 가 없다(스키마 엔티티 0개).
5. **v1 설치 SQL 비교는 위치 주석을 빼고 한다.** 코드를 다른 크레이트로 옮기면 `-- <파일>:<라인>` 과 `-- <크레이트>::<경로>` 주석이 바뀐다. 저장소 루트에 작업 공간을 만들기만 해도 `file!()` 이 작업 공간 기준 경로가 되어 `-- src/…` 가 `-- engine/src/…` 로 바뀐다. 주석 말고는 내용·순서 모두 같았다. `run-engine.sh` 의 `strip_locations` 정규식을 그대로 쓸 수 있다.
6. **pg 버전 기능을 라이브러리도 가져야 한다.** 엔진 코드가 `feature = "pg13"…"pg15"` 를 직접 본다(`engine/src/spiu.rs:70-72`, `engine/src/fabric/query.rs:592, 609`). 라이브러리 크레이트에 `pg13 … pg17 = ["pgrx/pgNN"]` 을 두고 확장 크레이트가 `pg16 = ["pgrx/pg16", "og-pg-common/pg16"]` 처럼 넘긴다. 안 넘기면 rustc 가 `unexpected cfg condition value: pg13` 경고를 내고 늘 `not(any(pg13, pg14, pg15))` 쪽이 컴파일된다(16·17 에서는 우연히 맞고 13~15 에서는 틀린다). 실험의 `engine_lib/Cargo.toml` 이 이 꼴이다.
7. **이름과 control**: 설치되는 `.so` 이름은 control 파일 이름(확장 이름)에서 온다. cargo 패키지·lib 이름은 아무것이어도 된다(실험의 v1 껍데기는 lib 이름이 `ontological_k0v1` 인데 `ontological.dylib` 로 설치됐다). 확장 크레이트 폴더마다 `*.control` 은 하나만 둔다(cargo-pgrx 가 처음 만난 것을 쓴다).
8. **`cargo pgrx package -p <확장 크레이트>` 가 작업 공간에서 그대로 된다.** resolver 2 라 `-p` 로 고른 확장의 기능만 켜진다. 프로필(`panic = "unwind"`, release 의 `lto = "fat"`, `codegen-units = 1`)은 작업 공간 루트로 옮겨야 한다(멤버의 `[profile]` 은 무시된다).
9. **두 판을 한 DB 에 둘 때의 함정(P2 의 일이지만 K7 이 이음매를 남겨 둘 것)**:
   - 한정 없는 함수 이름: 엔진의 Rust 쪽 SQL 문자열에 `og_*` 함수를 한정 없이 부르는 곳이 적어도 30개 이름(`og_node_json` 10곳, `og_edge_json` 7, `og_add_property` 7, `og_vlp`, `og_valid_at`, `og_cypher` …, 한 라인짜리 문자열만 센 하한). v2 에서는 이것이 v1 함수로 간다. 고칠 꼴: 판이 자기 이름을 라이브러리에 알려 주고(실험의 `og_shared::set_edition`, 확장 크레이트의 `_PG_init` 에서 호출) 라이브러리가 `pg_extension.extnamespace` 로 자기 스키마를 찾아 붙인다(`k0_spi_qualified`). 이름을 내는 곳을 `Layout` 뒤로 모으는 K2~K5 와 같은 방향이다.
   - GUC 이름: 공용 코드가 정의하는 GUC(지금은 `ontological.fabric_inline_passwords`, `engine/src/fabric/mod.rs:370`)는 두 `.so` 가 각자 정의하려 해서 둘째가 `ERROR` 다. 판마다 접두어를 달리 하거나(`ontological_age.*`) 정의하기 전에 있는지 본다. `current_setting` 으로 읽기만 하는 설정은 상관없다.
   - 훅: `fabric/query.rs:535-538` 의 `ExecutorCheckPerms_hook`·`object_access_hook` 은 `.so` 마다 따로 걸려 사슬로 둘 다 돈다. 오류는 아니지만 두 판이 같은 질의를 두 번 검사한다.
   - v1 고유 SQL 이 만드는 `og_catalog`·`og_data`·`og_action` 은 v2 설치 SQL 에 들어가면 안 된다(두 번 만들 수 없다).
10. **control 의 `schema` 로 만들어진 스키마는 확장의 구성원이 아니다.** `DROP EXTENSION ontological_age` 뒤에도 `oga_catalog` 스키마는 남는다(장난감 4단계에서 따로 지웠다). 지금 v1 이 만드는 `og_catalog` 등은 확장 스크립트 안의 `CREATE SCHEMA` 라 구성원이다.

### K7 폴더 배치안 (가)

```
Cargo.toml                 작업 공간(멤버 아래 전부, [profile] 은 여기로, Cargo.lock 하나)
crates/og-core/            pgrx 없음: 파서·AST·컴파일러·Layout 트레이트·카탈로그 모델
crates/og-pg-common/       pgrx 라이브러리: 공용 #[pg_extern], 공용 SQL(sql/), #[pg_schema] 없음,
                           bootstrap/finalize 없음, 기능 pg13…pg17 → pgrx
crates/og-store-v1/        pgrx 라이브러리: LayoutV1 + v1 저장·인접·뷰·이력·권한·벡터·순회
crates/og-store-age/       빈 뼈대(P2)
engine/                    확장 크레이트 `ontological`: 패키지 이름·버전·control 그대로,
                           pg_module_magic!, extern crate og_pg_common as _; extern crate og_store_v1 as _;,
                           sql/bootstrap.sql·release.sql·access.sql 의 v1 몫(bootstrap/finalize 표시 포함)
engine-age/                확장 크레이트 `ontological_age`(빈 뼈대): control schema = 'oga_catalog'
bolt/, sql-gateway/        멤버로 넣기만
```

`engine/` 의 위치와 패키지 이름을 그대로 두면 Studio 빌드 스크립트가 `engine/` 에서 `cargo pgrx package` 를 부르는 것도 그대로 된다(`-p ontological` 또는 `engine/` 에서 실행). 바뀌는 것은 `CARGO_TARGET_DIR` 기본값이 작업 공간 루트의 `target/` 으로 옮겨 가는 것과, `Cargo.lock` 이 루트로 가는 것이다.

## (나) 를 버리는 이유

(가) 가 되므로 만들지 않았다. 다만 (나) 로 가려 했다면 막혔을 곳:

- cargo-pgrx 는 매니페스트 폴더에서 처음 만난 `*.control` 하나를 쓰고(`get.rs:88-113`, 폴더 읽기 순서는 정해져 있지 않다), 확장 이름·`.so` 이름·설치 SQL 파일 이름을 모두 그 파일 이름에서 얻는다. 한 패키지에서 기능으로 control 을 고르는 길이 없다. 빌드 전에 control 을 바꿔 끼우는 스크립트를 두거나 매니페스트 폴더를 둘로 나눠야 하는데, 뒤쪽은 결국 (가) 다.
- cargo 기능은 한 빌드 안에서 합쳐진다. `--workspace --all-features` 같은 흔한 명령이 두 판을 한 `.so` 에 함께 켠다.

## 확인하지 못한 것

- **Linux(ELF) 는 돌리지 않았다**(이 맥에서 Docker 를 쓰지 않는다는 조건). 이론상 차이는 없다: release 는 fat LTO 라 모든 크레이트가 한 모듈로 합쳐진 뒤 링크되고, cargo-pgrx 가 Linux 에서 `--no-gc-sections` 를 붙인다(`cargo.rs:205-243`). dev 빌드에서 rlib 를 아카이브로 링크해도 엔티티가 남는 것은 macOS(ld64 도 아카이브 구성원을 참조될 때만 끌어온다)에서 확인했다. 두 `.so` 의 같은 이름 심볼은 PostgreSQL 이 `RTLD_GLOBAL` 로 올려도 라이브러리 핸들로 `dlsym` 하고, 내보낸 심볼을 안에서 서로 부르지 않으므로 끼어들지 않는다. **K7 의 첫 CI 실행(P0-b 의 {v1, age} × {16, 17} 행렬, Linux)에서 엔티티 수와 두 확장 함께 설치를 다시 본다.** `run.sh`·`run-engine.sh` 는 Linux 에서도 돌게 썼다(`/proc/<pid>/maps` 를 먼저 본다).
- PG18 의 `pg_module_magic!(name, version)` 이 보여 주는 모듈 이름은 보지 않았다(16·17 만 대상).
- 엔진을 실제로 나눈 상태(`og-pg-common` 이 `og-store-v1` 의 함수를 트레이트로 부르는 꼴)는 K7 의 일이라 하지 않았다. 여기서는 엔진 전체를 한 라이브러리로 두었다.

## 카드와 다르게 한 것

조정자 지시를 따랐다: 가지 `seam/k0-pgrx`(카드는 `seam/k0-pgrx-spike`), 폴더 `experiments/k0-pgrx/`(카드는 `experiments/pgrx-shared/`), 보고서는 이 파일(카드는 `reports/ontological-v2/P1-K0-pgrx.md`). 기능 이름은 (나) 를 만들지 않아 쓰지 않았다.
