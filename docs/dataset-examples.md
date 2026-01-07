# Dataset Schema Examples

Concrete examples for each Redis/Valkey data type to validate the unified dataset design.

## Data Type Analysis

| Type | Structure | Current Design Support |
|------|-----------|----------------------|
| STRING | key -> value | Needs new model |
| HASH | key -> {field: value, ...} | Supported (record = fields) |
| SET | key -> {member, ...} | Needs collection model |
| LIST | key -> [elem, ...] | Needs collection model |
| ZSET | key -> {score: member, ...} | Needs collection model |
| VECTOR | key -> {embedding, metadata...} | Supported |

**Key Insight**: The current design is **field-oriented** (fixed fields per record). Collection types need a **member-oriented** model (variable members per key).

---

## Example 1: STRING Keys

**Data to benchmark:**
```
SET "key:000000000001" "dummy-test-string-value-1"
SET "key:000000000002" "dummy-test-string-value-2"
...
```

### Schema: `string-dataset.yaml`

```yaml
version: 1

metadata:
  name: "string-benchmark"
  description: "Simple string key-value pairs"

# Workload type hint
workload:
  type: string           # STRING, HASH, SET, LIST, ZSET, VECTOR
  command: SET           # Primary command for load

record:
  fields:
    - name: value
      type: text
      encoding: utf8
      length: variable
      max_bytes: 1024

sections:
  records:
    count: 1000000

  keys:
    present: false        # Generate keys with pattern
    pattern: "key:%012d"   # Key pattern
```

### Binary Layout: `string-dataset.bin`

```
Record 0: [4-byte len=25]["dummy-test-string-value-1"][padding to 1028 bytes]
Record 1: [4-byte len=25]["dummy-test-string-value-2"][padding to 1028 bytes]
...
```

**Total record size**: 4 (length) + 1024 (max_bytes) = 1028 bytes

### Generated Command

```
SET key:000000000001 "dummy-test-string-value-1"
```

---

## Example 2: VECTOR Dataset (4-dim)

**Data to benchmark:**
```
HSET "vec:000000000001" embedding [0.1, 0.2, 0.3, 0.4]
FT.SEARCH idx "*=>[KNN 10 @embedding $BLOB]" PARAMS 2 BLOB <query_vector>
```

### Schema: `vector-4dim.yaml`

```yaml
version: 1

metadata:
  name: "vector-4dim"
  description: "Simple 4-dimensional vector dataset for testing"

workload:
  type: vector
  command: HSET

record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 4
      # Size: 4 * 4 = 16 bytes

sections:
  records:
    count: 100           # 100 vectors

  keys:
    present: false
    pattern: "vec:%012d"

  queries:
    present: true
    count: 10            # 10 query vectors
    query_fields:
      - embedding

  ground_truth:
    present: true
    neighbors_per_query: 10
    id_type: u64

field_metadata:
  embedding:
    distance_metric: l2
```

### Binary Layout: `vector-4dim.bin`

```
Section: Records (100 * 16 = 1600 bytes)
┌────────────────────────────────────────────────┐
│ Record 0: [0.1f32][0.2f32][0.3f32][0.4f32]     │  16 bytes
│ Record 1: [0.5f32][0.6f32][0.7f32][0.8f32]     │  16 bytes
│ ...                                             │
│ Record 99: [...]                                │  16 bytes
└────────────────────────────────────────────────┘

Section: Queries (10 * 16 = 160 bytes)
┌────────────────────────────────────────────────┐
│ Query 0: [q0_d0][q0_d1][q0_d2][q0_d3]          │  16 bytes
│ Query 1: [...]                                  │
│ ...                                             │
│ Query 9: [...]                                  │  16 bytes
└────────────────────────────────────────────────┘

Section: Ground Truth (10 * 10 * 8 = 800 bytes)
┌────────────────────────────────────────────────┐
│ Query 0 neighbors: [id0][id1]...[id9]          │  80 bytes (10 * u64)
│ Query 1 neighbors: [...]                        │
│ ...                                             │
│ Query 9 neighbors: [...]                        │  80 bytes
└────────────────────────────────────────────────┘

Total: 1600 + 160 + 800 = 2560 bytes
```

### Concrete Data Example

```
Records section (hex dump, little-endian float32):
  Offset 0x0000: CD CC CC 3D CD CC 4C 3E 9A 99 99 3E CD CC CC 3E  # [0.1, 0.2, 0.3, 0.4]
  Offset 0x0010: 00 00 00 3F CD CC 4C 3F 9A 99 99 3F CD CC CC 3F  # [0.5, 0.75, 0.9, 1.0]
  ...
```

---

## Example 3: HASH Keys

**Data to benchmark:**
```
HSET "hash:000000000001" field1 "value1" field2 "value2"
HSET "hash:000000000002" field1 "value1" field2 "value2"
```

### Schema: `hash-dataset.yaml`

```yaml
version: 1

metadata:
  name: "hash-benchmark"
  description: "HASH keys with multiple fields"

workload:
  type: hash
  command: HSET

record:
  fields:
    - name: field1
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64

    - name: field2
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64

sections:
  records:
    count: 2

  keys:
    present: true         # Keys stored in file
    encoding: utf8
    length: fixed
    max_bytes: 32
```

### Binary Layout: `hash-dataset.bin`

```
Section: Records (2 * 128 = 256 bytes)
┌────────────────────────────────────────────────────────────────────┐
│ Record 0:                                                           │
│   field1: "value1" + padding (64 bytes)                             │
│   field2: "value2" + padding (64 bytes)                             │
├────────────────────────────────────────────────────────────────────┤
│ Record 1:                                                           │
│   field1: "value1" + padding (64 bytes)                             │
│   field2: "value2" + padding (64 bytes)                             │
└────────────────────────────────────────────────────────────────────┘

Section: Keys (2 * 32 = 64 bytes)
┌────────────────────────────────────────────────────────────────────┐
│ Key 0: "hash1" + padding (32 bytes)                                 │
│ Key 1: "hash2" + padding (32 bytes)                                 │
└────────────────────────────────────────────────────────────────────┘

Total: 256 + 64 = 320 bytes
```

### Concrete Hex Dump

```
# Records section
0x0000: 76 61 6C 75 65 31 00 00 ... (64 bytes) # "value1" + nulls
0x0040: 76 61 6C 75 65 32 00 00 ... (64 bytes) # "value2" + nulls
0x0080: 76 61 6C 75 65 31 00 00 ... (64 bytes) # "value1" + nulls
0x00C0: 76 61 6C 75 65 32 00 00 ... (64 bytes) # "value2" + nulls

# Keys section
0x0100: 68 61 73 68 31 00 00 00 ... (32 bytes) # "hash1" + nulls
0x0120: 68 61 73 68 32 00 00 00 ... (32 bytes) # "hash2" + nulls
```

---

## Example 4: SET Keys

**Data to benchmark:**
```
SADD "set:000000000001" "member1" "member2"
SADD "set:000000000002" "member1" "member2"
```

### PROBLEM: Variable Members Per Key

The current design assumes **fixed fields per record**. SETs have **variable members per key**.

### Solution A: Fixed Member Count (Padding)

```yaml
version: 1

metadata:
  name: "set-benchmark"

workload:
  type: set
  command: SADD

record:
  # Collection type with fixed max members
  collection:
    type: set
    max_members: 10        # Max 10 members per SET
    member:
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64

  # Each record stores: [member_count: u32][member0][member1]...[member9]
  # Size: 4 + (10 * 64) = 644 bytes

sections:
  records:
    count: 2

  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 32
```

### Binary Layout (Solution A)

```
Section: Records (2 * 644 = 1288 bytes)
┌────────────────────────────────────────────────────────────────────┐
│ Record 0:                                                           │
│   member_count: 2 (u32, 4 bytes)                                    │
│   member[0]: "member1" + padding (64 bytes)                         │
│   member[1]: "member2" + padding (64 bytes)                         │
│   member[2..9]: unused padding (8 * 64 = 512 bytes)                 │
├────────────────────────────────────────────────────────────────────┤
│ Record 1: (same structure)                                          │
└────────────────────────────────────────────────────────────────────┘

Section: Keys (2 * 32 = 64 bytes)
┌────────────────────────────────────────────────────────────────────┐
│ Key 0: "set1" + padding                                             │
│ Key 1: "set2" + padding                                             │
└────────────────────────────────────────────────────────────────────┘
```

### Solution B: Separate Members Section (Offset-Based)

```yaml
version: 1

metadata:
  name: "set-benchmark-variable"

workload:
  type: set
  command: SADD

record:
  # Minimal record - just references into members section
  collection:
    type: set
    variable_count: true

  fields:
    - name: member_offset
      type: numeric
      dtype: u64          # Offset into members section

    - name: member_count
      type: numeric
      dtype: u32          # Number of members

sections:
  records:
    count: 2

  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 32

  # NEW: Members section for variable-length collections
  members:
    present: true
    encoding: utf8
    length: variable
    max_bytes: 128
```

### Binary Layout (Solution B)

```
Section: Records (2 * 12 = 24 bytes)
┌────────────────────────────────────────────────────────────────────┐
│ Record 0: [member_offset: 0 (u64)][member_count: 2 (u32)]          │
│ Record 1: [member_offset: 18 (u64)][member_count: 2 (u32)]         │
└────────────────────────────────────────────────────────────────────┘

Section: Keys (2 * 32 = 64 bytes)
┌────────────────────────────────────────────────────────────────────┐
│ Key 0: "set1" + padding                                             │
│ Key 1: "set2" + padding                                             │
└────────────────────────────────────────────────────────────────────┘

Section: Members (variable)
┌────────────────────────────────────────────────────────────────────┐
│ Offset 0: [len:7]["member1"]                                        │  # Record 0, member 0
│ Offset 11: [len:7]["member2"]                                       │  # Record 0, member 1
│ Offset 22: [len:7]["member1"]                                       │  # Record 1, member 0
│ Offset 33: [len:7]["member2"]                                       │  # Record 1, member 1
└────────────────────────────────────────────────────────────────────┘
```

### Recommendation

**Solution A (fixed max members)** is simpler and maintains O(1) access. Use when:
- Member counts are relatively uniform
- Max members is known and reasonable

**Solution B (offset-based)** is more flexible. Use when:
- Member counts vary widely
- Storage efficiency is critical

---

## Example 5: LIST Keys

**Data to benchmark:**
```
RPUSH "list:000000000001" "elem1" "elem2" "elem3"
RPUSH "list:000000000002" "elem1" "elem2"
```

### Schema: `list-dataset.yaml`

```yaml
version: 1

metadata:
  name: "list-benchmark"

workload:
  type: list
  command: RPUSH

record:
  collection:
    type: list
    max_elements: 100      # Max 100 elements per list
    element:
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64

sections:
  records:
    count: 2

  keys:
    present: false
    pattern: "list:%012d"
```

### Binary Layout

```
Record size: 4 (count) + 100 * 64 (elements) = 6404 bytes

Section: Records
┌────────────────────────────────────────────────────────────────────┐
│ Record 0:                                                           │
│   element_count: 3 (u32)                                            │
│   element[0]: "elem1" (64 bytes)                                    │
│   element[1]: "elem2" (64 bytes)                                    │
│   element[2]: "elem3" (64 bytes)                                    │
│   element[3..99]: padding (97 * 64 bytes)                           │
├────────────────────────────────────────────────────────────────────┤
│ Record 1:                                                           │
│   element_count: 2 (u32)                                            │
│   element[0]: "elem1" (64 bytes)                                    │
│   element[1]: "elem2" (64 bytes)                                    │
│   element[2..99]: padding (98 * 64 bytes)                           │
└────────────────────────────────────────────────────────────────────┘
```

---

## Example 6: ZSET Keys (Sorted Sets)

**Data to benchmark:**
```
ZADD "zset:000000000001" 1.5 "member1" 2.5 "member2"
ZADD "zset:000000000002" 10.0 "member1" 20.0 "member2"
```

### Schema: `zset-dataset.yaml`

```yaml
version: 1

metadata:
  name: "zset-benchmark"

workload:
  type: zset
  command: ZADD

record:
  collection:
    type: zset
    max_members: 50
    member:
      # ZSET member has score + value
      fields:
        - name: score
          type: numeric
          dtype: float64      # 8 bytes

        - name: value
          type: text
          encoding: utf8
          length: fixed
          max_bytes: 64       # 64 bytes

      # Per-member size: 8 + 64 = 72 bytes

sections:
  records:
    count: 2

  keys:
    present: false
    pattern: "zset:%012d"
```

### Binary Layout

```
Member size: 8 (score) + 64 (value) = 72 bytes
Record size: 4 (count) + 50 * 72 (members) = 3604 bytes

Section: Records
┌────────────────────────────────────────────────────────────────────┐
│ Record 0:                                                           │
│   member_count: 2 (u32)                                             │
│   member[0]: [score: 1.5 (f64)][value: "member1" (64 bytes)]        │
│   member[1]: [score: 2.5 (f64)][value: "member2" (64 bytes)]        │
│   member[2..49]: padding                                            │
├────────────────────────────────────────────────────────────────────┤
│ Record 1:                                                           │
│   member_count: 2 (u32)                                             │
│   member[0]: [score: 10.0 (f64)][value: "member1" (64 bytes)]       │
│   member[1]: [score: 20.0 (f64)][value: "member2" (64 bytes)]       │
│   member[2..49]: padding                                            │
└────────────────────────────────────────────────────────────────────┘
```

---

## Example 7: STREAM Keys

**Data to benchmark:**
```
XADD "stream:1" * field1 value1 field2 value2
XADD "stream:1" * field1 value1 field2 value2
```

Streams are more complex - each stream has multiple entries, each entry has an ID and fields.

### Schema: `stream-dataset.yaml`

```yaml
version: 1

metadata:
  name: "stream-benchmark"

workload:
  type: stream
  command: XADD

record:
  # For streams, one record = one XADD command
  # The stream key may be shared across multiple records
  fields:
    - name: stream_key_idx
      type: numeric
      dtype: u32          # Index into keys section (for key reuse)

    - name: field1
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64

    - name: field2
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64

sections:
  records:
    count: 1000000        # 1M XADD commands

  keys:
    present: true
    count: 100            # Only 100 unique stream keys
    encoding: utf8
    length: fixed
    max_bytes: 64
```

---

## Design Gap Summary

| Gap | Current Design | Proposed Extension |
|-----|---------------|-------------------|
| Collection types | Not supported | Add `collection` block with `max_members` |
| Variable member count | Not supported | `member_count` field + padding or offset-based |
| ZSET score+member | Not supported | Nested `fields` in collection member |
| Key reuse (streams) | Not supported | `key_idx` field referencing keys section |
| Workload hint | Implicit | Add `workload.type` and `workload.command` |

---

## Proposed Schema Extensions

### Extension 1: Collection Block

```yaml
record:
  # For collection types (SET, LIST, ZSET)
  collection:
    type: set | list | zset
    max_members: 100              # Fixed max for O(1) access
    member:
      type: text                  # Simple member
      # OR
      fields:                     # Compound member (ZSET)
        - name: score
          type: numeric
        - name: value
          type: text
```

### Extension 2: Workload Hints

```yaml
workload:
  type: string | hash | set | list | zset | stream | vector
  command: SET | HSET | SADD | RPUSH | ZADD | XADD | FT.SEARCH

  # Command-specific options
  options:
    # For SADD: how many members per command
    batch_size: 10

    # For FT.SEARCH: which field to query
    query_field: embedding
```

### Extension 3: Key Reuse (for Streams)

```yaml
sections:
  keys:
    present: true
    count: 100              # Number of unique keys
    reuse: true             # Records reference keys by index
```

---

## Complete Example: Vector + Metadata + Query

```yaml
version: 1

metadata:
  name: "product-vectors"
  description: "Product embeddings with category tags"

workload:
  type: vector
  command: HSET
  options:
    query_field: embedding
    index_name: product_idx

record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 4

    - name: category
      type: tag
      encoding: utf8
      max_bytes: 32

    - name: price
      type: numeric
      dtype: float64

sections:
  records:
    count: 5

  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 32

  queries:
    present: true
    count: 2
    query_fields:
      - embedding

  ground_truth:
    present: true
    neighbors_per_query: 3
    id_type: u64

field_metadata:
  embedding:
    distance_metric: l2
  category:
    index_type: tag
```

### Binary File Layout

```
=== SECTION: Records (5 records) ===
Offset: 0x0000
Record size: 16 (embedding) + 32 (category) + 8 (price) = 56 bytes

Record 0 (offset 0x0000):
  embedding: [0.1, 0.2, 0.3, 0.4]     # 16 bytes
  category:  "electronics\0..."        # 32 bytes
  price:     99.99                     # 8 bytes

Record 1 (offset 0x0038):
  embedding: [0.5, 0.6, 0.7, 0.8]
  category:  "clothing\0..."
  price:     49.99

...

Record 4 (offset 0x0118):
  embedding: [...]
  category:  "..."
  price:     ...

Records section size: 5 * 56 = 280 bytes

=== SECTION: Keys (5 keys) ===
Offset: 0x0118
Key size: 32 bytes (fixed)

Key 0: "prod:000000000001\0..."
Key 1: "prod:000000000002\0..."
...
Key 4: "prod:000000000005\0..."

Keys section size: 5 * 32 = 160 bytes

=== SECTION: Queries (2 queries) ===
Offset: 0x01B8
Query size: 16 bytes (only embedding field)

Query 0: [0.15, 0.25, 0.35, 0.45]
Query 1: [0.55, 0.65, 0.75, 0.85]

Queries section size: 2 * 16 = 32 bytes

=== SECTION: Ground Truth ===
Offset: 0x01D8
Entry size: 3 * 8 = 24 bytes (3 neighbors per query, u64 IDs)

Query 0 neighbors: [0, 1, 2]    # Record IDs
Query 1 neighbors: [3, 4, 1]

Ground truth size: 2 * 24 = 48 bytes

=== TOTAL FILE SIZE ===
280 + 160 + 32 + 48 = 520 bytes
```

### Hex Dump of Binary File

```
# Records section (offset 0x0000)
0x0000: CD CC CC 3D CD CC 4C 3E 9A 99 99 3E CD CC CC 3E  # vec[0.1,0.2,0.3,0.4]
0x0010: 65 6C 65 63 74 72 6F 6E 69 63 73 00 00 00 00 00  # "electronics" + pad
0x0020: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  # padding
0x0030: 71 3D 0A D7 A3 F0 58 40                          # 99.99 (f64)
# ... more records

# Keys section (offset 0x0118)
0x0118: 70 72 6F 64 3A 30 30 30 30 30 30 30 30 30 30 30  # "prod:00000000000"
0x0128: 30 30 30 30 30 30 31 00 00 00 00 00 00 00 00 00  # "0000001" + pad
# ... more keys

# Queries section (offset 0x01B8)
0x01B8: 9A 99 19 3E CD CC 80 3E 33 33 B3 3E 66 66 E6 3E  # query vec 0
# ... more queries

# Ground truth section (offset 0x01D8)
0x01D8: 00 00 00 00 00 00 00 00  # neighbor 0: record 0
0x01E0: 01 00 00 00 00 00 00 00  # neighbor 1: record 1
0x01E8: 02 00 00 00 00 00 00 00  # neighbor 2: record 2
# ... more ground truth
```

---

## Next Steps

1. **Update unified-dataset-design.md** with:
   - Collection type support (`collection` block)
   - Workload hints (`workload` block)
   - Key reuse for streams

2. **Create Python script** to generate example binary files from these schemas

3. **Implement Rust parser** for extended schema format
