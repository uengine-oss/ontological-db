# TypeQL reference

What this release supports, precisely — and what it does not. Unsupported syntax
fails at parse or compile time with the construct named. It is never silently
reinterpreted, and never silently ignored.

The dialect is **TypeDB 3.x TypeQL**. TypeDB 2.x keywords (`get`, `rule`,
`when`/`then`) are recognised only so they can be rejected by name.

```sql
SELECT og_typeql(graph, query);            -- SETOF jsonb, one object per row
SELECT og_typeql_script(graph, script);    -- run a whole .tql file, returns block count
SELECT og_typeql_sql(graph, query);        -- the compiled SQL for a read query
SELECT og_typeql_check(query);             -- parse only
SELECT og_typeql_schema(graph);            -- dump the schema back out as TypeQL
```

Everything runs in the caller's transaction. There is no schema/write/read
transaction split: PostgreSQL's transaction is the transaction.

---

## How TypeQL maps onto the graph

This is the part worth understanding, because it is what makes the same graph
answerable in two languages.

| TypeQL | stored as |
|---|---|
| entity type | a type in `og_catalog.type`, instances in `og_data.n_<id>` |
| relation type | a type, instances **reified as nodes** in `og_data.n_<id>` |
| attribute type | a type, instances in `og_data.a_<id>` with a **UNIQUE value** |
| `sub` | `og_catalog.type_parent` + the interval labels of spec 002 |
| `relates` / `relates X as Y` | `og_catalog.role`, with `parent_role_id` for specialisation |
| `owns` / `plays` | `og_catalog.og_constraint` |
| ownership (`has`) | an edge of the internal `$has` type, in the adjacency segments |
| role assignment | a row in `og_data.og_role_player` |

Two consequences follow, and both are deliberate:

**Attributes are shared, not copied.** Two books with `has genre "fiction"` own
*the same* genre instance — that is TypeDB's semantics, and it is why
`$a has genre $g; $b has genre $g;` finds books that share a genre by traversal
rather than by string comparison.

**Relations are first-class.** A relation can carry three or more roles, own
attributes, and play a role in another relation. A two-endpoint edge cannot
express any of those, so relation instances are nodes.

Read the mapping directly:

```sql
SELECT * FROM og_typeql_attribute;   -- owner, attribute type, value
SELECT * FROM og_typeql_role;        -- relation, role, player
```

A TypeQL graph is therefore visible from Cypher — as entity nodes, reified
relation nodes, attribute nodes, and `$has` edges:

```sql
SELECT og_cypher('bookstore', $$ MATCH (b:ebook)-[:`$has`]->(t:title) RETURN t.val $$);
```

---

## Supported

### define

```typeql
define
entity book @abstract,
    owns isbn @card(0..2),
    owns isbn-13 @key,
    owns isbn-10 @unique,
    owns genre @card(0..),
    plays contribution:work;

entity hardback sub book, owns stock;

relation contribution, relates contributor, relates work;
relation authoring sub contribution, relates author as contributor;
relation order-line, relates order, relates item, owns quantity;

attribute isbn @abstract, value string;
attribute isbn-13 sub isbn;
attribute status, value string @values("pending", "paid", "dispatched");
attribute loyalty-tier, value integer @range(0..5);
```

- kinds: `entity`, `relation`, `attribute`
- `sub`, `owns`, `relates`, `relates X as Y`, `plays R:r`
- value types: `string`, `integer`, `double`, `decimal`, `boolean`, `date`,
  `datetime`, `datetime-tz`, `duration`
- annotations: `@abstract`, `@key`, `@unique`, `@card(n..m)`, `@values(...)`,
  `@range(a..b)`. `@distinct`, `@cascade`, `@independent` and `@subkey` are
  accepted and ignored — they constrain nothing this engine materialises.
- **forward references are fine**: `plays contribution:work` may appear before
  `relation contribution` is declared. `define` runs in passes.
- **idempotent**: re-running the same `define` is a no-op.

Enforced at write time: `@key`, `@unique`, the *upper* bound of `@card`,
`@values`, `@range`, and the refusal to instantiate an `@abstract` type.

### insert / put / match ... insert

```typeql
insert $c isa country, has name "United States";

match $city isa city, has name "London";
insert
  $book isa paperback, has isbn-13 "9780553212150", has title "Pride and Prejudice";
  (published: $book, publisher: $p, publication: $pub) isa publishing;
put $contributor isa contributor, has name "Austen, Jane";
insert (work: $book, author: $contributor) isa authoring;
end;
```

`put` reuses an existing match or inserts — the idiom every TypeDB loading
script depends on. `match ... insert` runs the insert once per binding row.
Stages chain: `match`, then any number of `insert` / `put` / `update` / `delete`.

### match

```typeql
match
  $book isa book, has genre "science fiction";       # subtypes answer too
  $b isa! paperback;                                  # exact type only
  $p isa promotion, has start-timestamp <= $time;     # comparison predicates
  authoring (work: $book, author: $author);           # named roles
  ($review, $rated) isa rating;                       # positional roles
  $line isa order-line, links ($order, $item);        # links
  $t contains "Hobbit";                               # like / contains
  let $time = 2023-12-02T00:00:00;
  not { $book has genre "fiction"; };
  { $b has genre "fiction"; } or { $b has genre "history"; };
  $t sub book;                                        # type variables
```

- `isa` includes subtypes via one interval-index lookup — never a recursive walk.
- `has` follows attribute subtyping: `has isbn $x` finds `isbn-13` and `isbn-10`.
- A supertype role reaches every role that specialises it: matching
  `contribution (contributor: $c, ...)` finds `authoring`, `editing` and
  `illustrating` instances.
- Unnamed role players are **unordered**, so `($a, $b) isa rating` matches each
  rating twice, once per assignment. Constrain the variables' types to pin it.

### Pipeline stages

`select`, `distinct`, `sort $x asc|desc`, `limit`, `offset`,
`reduce $n = count|sum|max|min|mean|median|std|list [groupby $k]`, `fetch`.

Stages apply in the order written — `sort` then `limit` is not `limit` then
`sort`, and the compiled SQL reflects that by wrapping, not by normalising.

### fetch

```typeql
fetch {
  "title": $book.title,
  "price": $book.price,
  "genres": [$book.genre],
  "delivery": { "courier": $courier.name },
  "authors": [
    match authoring (work: $book, author: $author);
    fetch { "name": $author.name };
  ]
};
```

Sub-fetches compile to correlated aggregates, so a book with no authors comes
back with `"authors": []` rather than disappearing from the result.

Expressions support `+ - * / %`, parentheses, and `round`, `abs`, `floor`,
`ceil`, `length`.

### delete / update

```typeql
match $b isa book, has title "Obsolete"; delete $b;
match $o isa order, has id "o0001"; update $o has status "paid";
```

Deleting an instance also removes its ownerships and the relation instances it
participated in — a relation missing a player is not a smaller relation, it is
a broken one.

---

## Not supported

Each of these raises an error naming the construct. None of them silently
returns a partial answer.

| Construct | Status |
|---|---|
| `fun` **evaluation** (`let $x = f(...)`, `let $x in f(...)`) | declarations are parsed, stored and reproduced by `og_typeql_schema()`; **calling one errors** |
| recursive functions | out of scope — this is a recursive-query layer, and a separate piece of work |
| `with fun` (query-local functions) | errors |
| `undefine` | parsed, errors on execution |
| `@card` **lower** bound | not checked; the upper bound is |
| disjunctions that export new variables | `or` compiles to correlated `EXISTS`, so a branch cannot bind a variable used outside it |
| TypeDB 2.x syntax (`get`, `rule`) | rejected by name, with the 3.x replacement suggested |
| TypeDB wire protocol / drivers | out of scope — this is **language** compatibility, not wire compatibility |

---

## Conformance

The acceptance evidence is not self-authored. `examples/typedb/bookstore/`
holds the upstream TypeDB bookstore example verbatim, and

```bash
python3 tests/typeql/run.py
```

loads its `schema.tql` and `data.tql` unmodified, runs the queries printed in
that example's own README, and compares the results to the output printed
alongside them. Unsupported constructs are reported as unsupported rather than
counted as passes.
