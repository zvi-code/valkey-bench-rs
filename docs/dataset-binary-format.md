# Dataset Binary Format Specification

This document defines the exact binary layout for schema-driven datasets and how mmap direct memory access works.

## Core Principle: Computed Offsets

The schema provides ALL information needed to compute byte offsets at load time:

```
byte_offset(record_idx, field_name) =
    section_offset[records] +
    (record_idx * record_size) +
    field_offset[field_name]
```

**No runtime parsing needed** - just pointer arithmetic.

---

## Section 1: Schema to Layout Computation

### Step 1: Compute Field Sizes

```python
def compute_field_size(field: dict) -> int:
    """Compute byte size for a field from schema."""
    field_type = field['type']

    if field_type == 'vector':
        dim = field['dimensions']
        dtype_sizes = {'float32': 4, 'float16': 2, 'uint8': 1, 'int8': 1}
        return dim * dtype_sizes[field.get('dtype', 'float32')]

    elif field_type == 'text' or field_type == 'tag':
        length_type = field.get('length', 'fixed')
        max_bytes = field['max_bytes']
        if length_type == 'variable':
            return 4 + max_bytes  # u32 length prefix + data
        return max_bytes  # fixed: exact size

    elif field_type == 'numeric':
        dtype_sizes = {'int32': 4, 'int64': 8, 'float32': 4, 'float64': 8, 'u32': 4, 'u64': 8}
        return dtype_sizes[field.get('dtype', 'float64')]

    elif field_type == 'blob':
        length_type = field.get('length', 'fixed')
        max_bytes = field['max_bytes']
        if length_type == 'variable':
            return 4 + max_bytes
        return max_bytes

    raise ValueError(f"Unknown field type: {field_type}")
```

### Step 2: Compute Record Layout

```python
def compute_record_layout(schema: dict) -> dict:
    """Compute field offsets and record size from schema."""
    fields = schema['record']['fields']
    layout = {
        'fields': [],
        'field_by_name': {},
        'record_size': 0
    }

    offset = 0
    for idx, field in enumerate(fields):
        size = compute_field_size(field)
        layout['fields'].append({
            'name': field['name'],
            'offset': offset,
            'size': size,
            'type': field['type'],
            'definition': field
        })
        layout['field_by_name'][field['name']] = idx
        offset += size

    layout['record_size'] = offset
    return layout
```

### Step 3: Compute Section Offsets

```python
def compute_section_layout(schema: dict, record_layout: dict) -> dict:
    """Compute section offsets in binary file."""
    sections = schema['sections']
    record_count = sections['records']['count']
    record_size = record_layout['record_size']

    layout = {
        'records': {'offset': 0, 'size': record_count * record_size},
    }

    next_offset = layout['records']['size']

    # Keys section (if present)
    if sections.get('keys', {}).get('present', False):
        keys_config = sections['keys']
        key_size = keys_config['max_bytes']
        if keys_config.get('length') == 'variable':
            key_size += 4  # u32 length prefix
        layout['keys'] = {
            'offset': next_offset,
            'size': record_count * key_size,
            'entry_size': key_size
        }
        next_offset += layout['keys']['size']

    # Queries section (if present)
    if sections.get('queries', {}).get('present', False):
        query_count = sections['queries']['count']
        query_fields = sections['queries'].get('query_fields', [])
        # Query record only contains specified fields
        query_size = sum(
            record_layout['fields'][record_layout['field_by_name'][f]]['size']
            for f in query_fields
        )
        layout['queries'] = {
            'offset': next_offset,
            'size': query_count * query_size,
            'entry_size': query_size,
            'count': query_count
        }
        next_offset += layout['queries']['size']

    # Ground truth section (if present)
    if sections.get('ground_truth', {}).get('present', False):
        gt = sections['ground_truth']
        query_count = sections['queries']['count']
        neighbors = gt['neighbors_per_query']
        id_size = 8 if gt.get('id_type', 'u64') == 'u64' else 4
        layout['ground_truth'] = {
            'offset': next_offset,
            'size': query_count * neighbors * id_size,
            'neighbors_per_query': neighbors,
            'id_size': id_size
        }
        next_offset += layout['ground_truth']['size']

    layout['total_size'] = next_offset
    return layout
```

---

## Section 2: Concrete Examples with Binary Layout

### Example A: Simple String Dataset

**Schema: `string-simple.yaml`**
```yaml
version: 1
metadata:
  name: "string-simple"

record:
  fields:
    - name: value
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 32

sections:
  records:
    count: 3
  keys:
    present: false
    pattern: "str:%012d"
```

**Layout Computation:**
```
Field 'value': offset=0, size=32
Record size: 32 bytes

Section 'records': offset=0, size=3*32=96 bytes
Total file size: 96 bytes
```

**Binary File: `string-simple.bin`**
```
Offset  | Hex                                              | ASCII
--------|--------------------------------------------------|------------------
0x0000  | 68 65 6C 6C 6F 00 00 00 00 00 00 00 00 00 00 00 | hello...........
0x0010  | 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 | ................
0x0020  | 77 6F 72 6C 64 00 00 00 00 00 00 00 00 00 00 00 | world...........
0x0030  | 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 | ................
0x0040  | 74 65 73 74 00 00 00 00 00 00 00 00 00 00 00 00 | test............
0x0050  | 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 | ................
```

**mmap Access:**
```rust
fn get_value(mmap: &[u8], record_idx: usize) -> &str {
    let offset = record_idx * 32;  // record_size = 32
    let bytes = &mmap[offset..offset + 32];
    // Find null terminator or use full length
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    std::str::from_utf8(&bytes[..len]).unwrap()
}

// Usage:
// get_value(&mmap, 0) -> "hello"
// get_value(&mmap, 1) -> "world"
// get_value(&mmap, 2) -> "test"
```

---

### Example B: Vector Dataset (4-dim)

**Schema: `vector-4dim.yaml`**
```yaml
version: 1
metadata:
  name: "vector-4dim"

record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 4

sections:
  records:
    count: 3

  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 24

  queries:
    present: true
    count: 2
    query_fields:
      - embedding

  ground_truth:
    present: true
    neighbors_per_query: 3
    id_type: u64
```

**Layout Computation:**
```
Field 'embedding': offset=0, size=4*4=16 bytes
Record size: 16 bytes

Section 'records':     offset=0,   size=3*16=48 bytes
Section 'keys':        offset=48,  size=3*24=72 bytes
Section 'queries':     offset=120, size=2*16=32 bytes
Section 'ground_truth': offset=152, size=2*3*8=48 bytes
Total file size: 200 bytes
```

**Binary File: `vector-4dim.bin`**
```
=== SECTION: Records (offset 0x0000, 48 bytes) ===
Record 0 (offset 0x0000): embedding = [1.0, 2.0, 3.0, 4.0]
0x0000  | 00 00 80 3F 00 00 00 40 00 00 40 40 00 00 80 40 |

Record 1 (offset 0x0010): embedding = [5.0, 6.0, 7.0, 8.0]
0x0010  | 00 00 A0 40 00 00 C0 40 00 00 E0 40 00 00 00 41 |

Record 2 (offset 0x0020): embedding = [0.1, 0.2, 0.3, 0.4]
0x0020  | CD CC CC 3D CD CC 4C 3E 9A 99 99 3E CD CC CC 3E |

=== SECTION: Keys (offset 0x0030, 72 bytes) ===
Key 0 (offset 0x0030): "vec:000000000001"
0x0030  | 76 65 63 3A 30 30 30 30 30 30 30 30 30 30 30 31 | vec:000000000001
0x0040  | 00 00 00 00 00 00 00 00                         | ........

Key 1 (offset 0x0048): "vec:000000000002"
0x0048  | 76 65 63 3A 30 30 30 30 30 30 30 30 30 30 30 32 | vec:000000000002
0x0058  | 00 00 00 00 00 00 00 00                         | ........

Key 2 (offset 0x0060): "vec:000000000003"
0x0060  | 76 65 63 3A 30 30 30 30 30 30 30 30 30 30 30 33 | vec:000000000003
0x0070  | 00 00 00 00 00 00 00 00                         | ........

=== SECTION: Queries (offset 0x0078, 32 bytes) ===
Query 0 (offset 0x0078): [1.5, 2.5, 3.5, 4.5]
0x0078  | 00 00 C0 3F 00 00 20 40 00 00 60 40 00 00 90 40 |

Query 1 (offset 0x0088): [0.5, 0.6, 0.7, 0.8]
0x0088  | 00 00 00 3F 9A 99 19 3F 33 33 33 3F CD CC 4C 3F |

=== SECTION: Ground Truth (offset 0x0098, 48 bytes) ===
Query 0 neighbors: [0, 1, 2] (record indices)
0x0098  | 00 00 00 00 00 00 00 00  # neighbor 0: record 0
0x00A0  | 01 00 00 00 00 00 00 00  # neighbor 1: record 1
0x00A8  | 02 00 00 00 00 00 00 00  # neighbor 2: record 2

Query 1 neighbors: [2, 0, 1]
0x00B0  | 02 00 00 00 00 00 00 00  # neighbor 0: record 2
0x00B8  | 00 00 00 00 00 00 00 00  # neighbor 1: record 0
0x00C0  | 01 00 00 00 00 00 00 00  # neighbor 2: record 1

=== END OF FILE (0x00C8 = 200 bytes) ===
```

**mmap Access Implementation:**
```rust
struct DatasetContext {
    mmap: Mmap,
    record_size: usize,        // 16
    record_count: usize,       // 3

    // Section offsets
    records_offset: usize,     // 0
    keys_offset: usize,        // 48
    keys_entry_size: usize,    // 24
    queries_offset: usize,     // 120
    query_size: usize,         // 16
    query_count: usize,        // 2
    ground_truth_offset: usize, // 152
    neighbors_per_query: usize, // 3

    // Field layout
    fields: Vec<FieldLayout>,  // [{name: "embedding", offset: 0, size: 16}]
}

impl DatasetContext {
    /// Get embedding vector for record
    fn get_embedding(&self, record_idx: usize) -> &[f32] {
        let offset = self.records_offset + (record_idx * self.record_size);
        let bytes = &self.mmap[offset..offset + 16];
        // Safe because we know layout
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, 4) }
    }

    /// Get key for record
    fn get_key(&self, record_idx: usize) -> &str {
        let offset = self.keys_offset + (record_idx * self.keys_entry_size);
        let bytes = &self.mmap[offset..offset + self.keys_entry_size];
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(self.keys_entry_size);
        std::str::from_utf8(&bytes[..len]).unwrap()
    }

    /// Get query vector
    fn get_query(&self, query_idx: usize) -> &[f32] {
        let offset = self.queries_offset + (query_idx * self.query_size);
        let bytes = &self.mmap[offset..offset + 16];
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, 4) }
    }

    /// Get ground truth neighbors for query
    fn get_ground_truth(&self, query_idx: usize) -> &[u64] {
        let offset = self.ground_truth_offset + (query_idx * self.neighbors_per_query * 8);
        let bytes = &self.mmap[offset..offset + self.neighbors_per_query * 8];
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u64, self.neighbors_per_query) }
    }
}
```

---

### Example C: HASH with Multiple Fields

**Schema: `hash-multi.yaml`**
```yaml
version: 1
metadata:
  name: "hash-multi"

record:
  fields:
    - name: field1
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 16

    - name: field2
      type: numeric
      dtype: float64

    - name: field3
      type: text
      encoding: utf8
      length: variable    # Variable length!
      max_bytes: 32

sections:
  records:
    count: 2
  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 16
```

**Layout Computation:**
```
Field 'field1': offset=0,  size=16 bytes (fixed text)
Field 'field2': offset=16, size=8 bytes (float64)
Field 'field3': offset=24, size=4+32=36 bytes (variable: u32 len + data)
Record size: 16 + 8 + 36 = 60 bytes

Section 'records': offset=0,   size=2*60=120 bytes
Section 'keys':    offset=120, size=2*16=32 bytes
Total file size: 152 bytes
```

**Binary File: `hash-multi.bin`**
```
=== SECTION: Records (offset 0x0000, 120 bytes) ===

Record 0 (offset 0x0000, 60 bytes):
  field1 (offset 0, 16 bytes): "hello"
  0x0000  | 68 65 6C 6C 6F 00 00 00 00 00 00 00 00 00 00 00 |

  field2 (offset 16, 8 bytes): 3.14159
  0x0010  | 6E 86 1B F0 F9 21 09 40 |

  field3 (offset 24, 36 bytes): variable "world" (len=5)
  0x0018  | 05 00 00 00                                     | len=5 (u32 LE)
  0x001C  | 77 6F 72 6C 64 00 00 00 00 00 00 00 00 00 00 00 | "world" + padding
  0x002C  | 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 |

Record 1 (offset 0x003C, 60 bytes):
  field1 (offset 0, 16 bytes): "test"
  0x003C  | 74 65 73 74 00 00 00 00 00 00 00 00 00 00 00 00 |

  field2 (offset 16, 8 bytes): 2.71828
  0x004C  | 90 F7 AA 95 09 BF 05 40 |

  field3 (offset 24, 36 bytes): variable "longer string here" (len=18)
  0x0054  | 12 00 00 00                                     | len=18 (u32 LE)
  0x0058  | 6C 6F 6E 67 65 72 20 73 74 72 69 6E 67 20 68 65 | "longer string he"
  0x0068  | 72 65 00 00 00 00 00 00 00 00 00 00 00 00 00 00 | "re" + padding

=== SECTION: Keys (offset 0x0078, 32 bytes) ===
Key 0: "hash:001"
0x0078  | 68 61 73 68 3A 30 30 31 00 00 00 00 00 00 00 00 |

Key 1: "hash:002"
0x0088  | 68 61 73 68 3A 30 30 32 00 00 00 00 00 00 00 00 |

=== END OF FILE (0x0098 = 152 bytes) ===
```

**mmap Access for Variable-Length Fields:**
```rust
impl DatasetContext {
    /// Get variable-length field value
    fn get_variable_text(&self, record_idx: usize, field_offset: usize) -> &str {
        let record_offset = self.records_offset + (record_idx * self.record_size);
        let field_start = record_offset + field_offset;

        // Read length prefix (u32 little-endian)
        let len_bytes = &self.mmap[field_start..field_start + 4];
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;

        // Read actual data (after length prefix)
        let data_start = field_start + 4;
        let bytes = &self.mmap[data_start..data_start + len];
        std::str::from_utf8(bytes).unwrap()
    }

    /// Get field3 for record (variable-length text at offset 24)
    fn get_field3(&self, record_idx: usize) -> &str {
        self.get_variable_text(record_idx, 24)  // field3 offset = 24
    }
}

// Usage:
// ctx.get_field3(0) -> "world"
// ctx.get_field3(1) -> "longer string here"
```

---

### Example D: SET with Fixed Max Members

**Schema: `set-fixed.yaml`**
```yaml
version: 1
metadata:
  name: "set-fixed"

record:
  # Collection type
  collection:
    type: set
    max_members: 4
    member:
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 8

sections:
  records:
    count: 2
  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 12
```

**Layout Computation:**
```
Member size: 8 bytes
Collection header: 4 bytes (u32 member_count)
Collection data: 4 * 8 = 32 bytes (max_members * member_size)
Record size: 4 + 32 = 36 bytes

Section 'records': offset=0,  size=2*36=72 bytes
Section 'keys':    offset=72, size=2*12=24 bytes
Total file size: 96 bytes
```

**Binary File: `set-fixed.bin`**
```
=== SECTION: Records (offset 0x0000, 72 bytes) ===

Record 0 (offset 0x0000, 36 bytes):
  member_count: 2
  0x0000  | 02 00 00 00 |  # count = 2 (u32 LE)

  member[0]: "apple"
  0x0004  | 61 70 70 6C 65 00 00 00 |

  member[1]: "banana"
  0x000C  | 62 61 6E 61 6E 61 00 00 |

  member[2]: (unused padding)
  0x0014  | 00 00 00 00 00 00 00 00 |

  member[3]: (unused padding)
  0x001C  | 00 00 00 00 00 00 00 00 |

Record 1 (offset 0x0024, 36 bytes):
  member_count: 3
  0x0024  | 03 00 00 00 |  # count = 3 (u32 LE)

  member[0]: "cherry"
  0x0028  | 63 68 65 72 72 79 00 00 |

  member[1]: "date"
  0x0030  | 64 61 74 65 00 00 00 00 |

  member[2]: "fig"
  0x0038  | 66 69 67 00 00 00 00 00 |

  member[3]: (unused padding)
  0x0040  | 00 00 00 00 00 00 00 00 |

=== SECTION: Keys (offset 0x0048, 24 bytes) ===
Key 0: "fruits:set1"
0x0048  | 66 72 75 69 74 73 3A 73 65 74 31 00 |

Key 1: "fruits:set2"
0x0054  | 66 72 75 69 74 73 3A 73 65 74 32 00 |

=== END OF FILE (0x0060 = 96 bytes) ===
```

**mmap Access for Collections:**
```rust
impl DatasetContext {
    /// Get SET members for a record
    fn get_set_members(&self, record_idx: usize) -> Vec<&str> {
        let record_offset = self.records_offset + (record_idx * self.record_size);

        // Read member count
        let count_bytes = &self.mmap[record_offset..record_offset + 4];
        let count = u32::from_le_bytes(count_bytes.try_into().unwrap()) as usize;

        // Read each member
        let member_size = 8;  // From schema: max_bytes = 8
        let members_start = record_offset + 4;

        (0..count)
            .map(|i| {
                let member_offset = members_start + (i * member_size);
                let bytes = &self.mmap[member_offset..member_offset + member_size];
                let len = bytes.iter().position(|&b| b == 0).unwrap_or(member_size);
                std::str::from_utf8(&bytes[..len]).unwrap()
            })
            .collect()
    }
}

// Usage:
// ctx.get_set_members(0) -> ["apple", "banana"]
// ctx.get_set_members(1) -> ["cherry", "date", "fig"]
```

---

### Example E: ZSET with Score+Member Pairs

**Schema: `zset-scores.yaml`**
```yaml
version: 1
metadata:
  name: "zset-scores"

record:
  collection:
    type: zset
    max_members: 3
    member:
      fields:
        - name: score
          type: numeric
          dtype: float64
        - name: value
          type: text
          encoding: utf8
          length: fixed
          max_bytes: 12

sections:
  records:
    count: 2
  keys:
    present: false
    pattern: "zset:%06d"
```

**Layout Computation:**
```
Member entry size: 8 (score) + 12 (value) = 20 bytes
Collection header: 4 bytes (u32 member_count)
Collection data: 3 * 20 = 60 bytes
Record size: 4 + 60 = 64 bytes

Section 'records': offset=0, size=2*64=128 bytes
Total file size: 128 bytes
```

**Binary File: `zset-scores.bin`**
```
=== SECTION: Records (offset 0x0000, 128 bytes) ===

Record 0 (offset 0x0000, 64 bytes):
  member_count: 2
  0x0000  | 02 00 00 00 |  # count = 2

  member[0]: score=1.5, value="alice"
  0x0004  | 00 00 00 00 00 00 F8 3F |  # 1.5 (f64 LE)
  0x000C  | 61 6C 69 63 65 00 00 00 00 00 00 00 |  # "alice"

  member[1]: score=2.5, value="bob"
  0x0018  | 00 00 00 00 00 00 04 40 |  # 2.5 (f64 LE)
  0x0020  | 62 6F 62 00 00 00 00 00 00 00 00 00 |  # "bob"

  member[2]: (unused padding, 20 bytes)
  0x002C  | 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 |
  0x003C  | 00 00 00 00 |

Record 1 (offset 0x0040, 64 bytes):
  member_count: 3
  0x0040  | 03 00 00 00 |  # count = 3

  member[0]: score=10.0, value="x"
  0x0044  | 00 00 00 00 00 00 24 40 |  # 10.0
  0x004C  | 78 00 00 00 00 00 00 00 00 00 00 00 |  # "x"

  member[1]: score=20.0, value="y"
  0x0058  | 00 00 00 00 00 00 34 40 |  # 20.0
  0x0060  | 79 00 00 00 00 00 00 00 00 00 00 00 |  # "y"

  member[2]: score=30.0, value="z"
  0x006C  | 00 00 00 00 00 00 3E 40 |  # 30.0
  0x0074  | 7A 00 00 00 00 00 00 00 00 00 00 00 |  # "z"

=== END OF FILE (0x0080 = 128 bytes) ===
```

**mmap Access for ZSET:**
```rust
struct ZSetMember<'a> {
    score: f64,
    value: &'a str,
}

impl DatasetContext {
    fn get_zset_members(&self, record_idx: usize) -> Vec<ZSetMember> {
        let record_offset = self.records_offset + (record_idx * self.record_size);

        // Read member count
        let count_bytes = &self.mmap[record_offset..record_offset + 4];
        let count = u32::from_le_bytes(count_bytes.try_into().unwrap()) as usize;

        let member_entry_size = 8 + 12;  // score + value
        let members_start = record_offset + 4;

        (0..count)
            .map(|i| {
                let entry_offset = members_start + (i * member_entry_size);

                // Read score (f64)
                let score_bytes = &self.mmap[entry_offset..entry_offset + 8];
                let score = f64::from_le_bytes(score_bytes.try_into().unwrap());

                // Read value (text)
                let value_offset = entry_offset + 8;
                let value_bytes = &self.mmap[value_offset..value_offset + 12];
                let len = value_bytes.iter().position(|&b| b == 0).unwrap_or(12);
                let value = std::str::from_utf8(&value_bytes[..len]).unwrap();

                ZSetMember { score, value }
            })
            .collect()
    }
}

// Usage:
// ctx.get_zset_members(0) -> [{score: 1.5, value: "alice"}, {score: 2.5, value: "bob"}]
// ctx.get_zset_members(1) -> [{score: 10.0, value: "x"}, {score: 20.0, value: "y"}, {score: 30.0, value: "z"}]
```

---

## Section 3: Python Script to Generate Binary Files

```python
#!/usr/bin/env python3
"""
generate_binary.py - Generate binary dataset from schema and data.

Usage:
    python generate_binary.py schema.yaml data.json output.bin
"""

import yaml
import json
import struct
import sys
from pathlib import Path


def compute_field_size(field):
    """Compute byte size for a field."""
    ftype = field['type']

    if ftype == 'vector':
        dim = field['dimensions']
        dtype_sizes = {'float32': 4, 'float16': 2, 'uint8': 1}
        return dim * dtype_sizes.get(field.get('dtype', 'float32'), 4)

    elif ftype in ('text', 'tag'):
        max_bytes = field['max_bytes']
        if field.get('length') == 'variable':
            return 4 + max_bytes  # u32 prefix + data
        return max_bytes

    elif ftype == 'numeric':
        dtype_sizes = {'int32': 4, 'int64': 8, 'float32': 4, 'float64': 8, 'u32': 4, 'u64': 8}
        return dtype_sizes.get(field.get('dtype', 'float64'), 8)

    elif ftype == 'blob':
        max_bytes = field['max_bytes']
        if field.get('length') == 'variable':
            return 4 + max_bytes
        return max_bytes

    raise ValueError(f"Unknown field type: {ftype}")


def write_field(f, field, value):
    """Write a single field value to file."""
    ftype = field['type']

    if ftype == 'vector':
        import array
        arr = array.array('f', value)  # float32
        f.write(arr.tobytes())

    elif ftype in ('text', 'tag'):
        data = value.encode('utf-8') if isinstance(value, str) else value
        max_bytes = field['max_bytes']

        if field.get('length') == 'variable':
            # Write length prefix + data + padding
            f.write(struct.pack('<I', len(data)))
            f.write(data[:max_bytes])
            padding = max_bytes - min(len(data), max_bytes)
            f.write(b'\x00' * padding)
        else:
            # Fixed length: data + null padding
            f.write(data[:max_bytes])
            padding = max_bytes - min(len(data), max_bytes)
            f.write(b'\x00' * padding)

    elif ftype == 'numeric':
        dtype = field.get('dtype', 'float64')
        fmt_map = {'int32': '<i', 'int64': '<q', 'float32': '<f', 'float64': '<d', 'u32': '<I', 'u64': '<Q'}
        f.write(struct.pack(fmt_map[dtype], value))


def write_collection(f, collection_def, members):
    """Write a collection (SET, LIST, ZSET) to file."""
    max_members = collection_def['max_members']
    member_def = collection_def['member']

    # Write member count
    f.write(struct.pack('<I', len(members)))

    # Calculate member size
    if 'fields' in member_def:
        # Compound member (ZSET)
        member_size = sum(compute_field_size(fd) for fd in member_def['fields'])
    else:
        # Simple member
        member_size = compute_field_size(member_def)

    # Write each member
    for i, member in enumerate(members[:max_members]):
        if 'fields' in member_def:
            # Compound member
            for fd in member_def['fields']:
                write_field(f, fd, member[fd['name']])
        else:
            # Simple member
            write_field(f, member_def, member)

    # Padding for unused member slots
    unused = max_members - len(members)
    f.write(b'\x00' * (unused * member_size))


def generate_binary(schema_path, data_path, output_path):
    """Generate binary file from schema and data."""

    # Load schema
    with open(schema_path) as sf:
        schema = yaml.safe_load(sf)

    # Load data
    with open(data_path) as df:
        data = json.load(df)

    # Compute layout
    record_def = schema['record']

    with open(output_path, 'wb') as out:
        # Write records section
        for record in data.get('records', []):
            if 'collection' in record_def:
                # Collection type (SET, LIST, ZSET)
                write_collection(out, record_def['collection'], record['members'])
            else:
                # Field-based record
                for field in record_def['fields']:
                    write_field(out, field, record.get(field['name'], ''))

        # Write keys section (if present in schema)
        sections = schema.get('sections', {})
        if sections.get('keys', {}).get('present'):
            key_config = sections['keys']
            max_bytes = key_config['max_bytes']
            for key in data.get('keys', []):
                key_bytes = key.encode('utf-8')
                out.write(key_bytes[:max_bytes])
                out.write(b'\x00' * (max_bytes - len(key_bytes)))

        # Write queries section (if present)
        if sections.get('queries', {}).get('present'):
            query_fields = sections['queries'].get('query_fields', [])
            for query in data.get('queries', []):
                for fname in query_fields:
                    field = next(f for f in record_def['fields'] if f['name'] == fname)
                    write_field(out, field, query[fname])

        # Write ground truth section (if present)
        if sections.get('ground_truth', {}).get('present'):
            gt_config = sections['ground_truth']
            id_type = gt_config.get('id_type', 'u64')
            fmt = '<Q' if id_type == 'u64' else '<I'
            for gt in data.get('ground_truth', []):
                for neighbor_id in gt:
                    out.write(struct.pack(fmt, neighbor_id))


if __name__ == '__main__':
    if len(sys.argv) != 4:
        print(f"Usage: {sys.argv[0]} schema.yaml data.json output.bin")
        sys.exit(1)

    generate_binary(sys.argv[1], sys.argv[2], sys.argv[3])
    print(f"Generated: {sys.argv[3]}")
```

---

## Section 4: Complete Working Example

### Files to Create

**1. `example-vector.yaml`** (Schema)
```yaml
version: 1
metadata:
  name: "example-vector"

record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 4

sections:
  records:
    count: 3
  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 16
  queries:
    present: true
    count: 2
    query_fields:
      - embedding
  ground_truth:
    present: true
    neighbors_per_query: 2
    id_type: u64
```

**2. `example-vector.json`** (Data)
```json
{
  "records": [
    {"embedding": [1.0, 2.0, 3.0, 4.0]},
    {"embedding": [5.0, 6.0, 7.0, 8.0]},
    {"embedding": [0.1, 0.2, 0.3, 0.4]}
  ],
  "keys": [
    "vec:000001",
    "vec:000002",
    "vec:000003"
  ],
  "queries": [
    {"embedding": [1.1, 2.1, 3.1, 4.1]},
    {"embedding": [5.1, 6.1, 7.1, 8.1]}
  ],
  "ground_truth": [
    [0, 2],
    [1, 0]
  ]
}
```

**3. Generate Binary**
```bash
python generate_binary.py example-vector.yaml example-vector.json example-vector.bin
```

**4. Verify with hexdump**
```bash
xxd example-vector.bin
```

Expected output:
```
00000000: 0000 803f 0000 0040 0000 4040 0000 8040  ...?...@..@@...@
00000010: 0000 a040 0000 c040 0000 e040 0000 0041  ...@...@...@...A
00000020: cdcc cc3d cdcc 4c3e 9a99 993e cdcc cc3e  ...=..L>...>...>
00000030: 7665 633a 3030 3030 3031 0000 0000 0000  vec:000001......
00000040: 7665 633a 3030 3030 3032 0000 0000 0000  vec:000002......
00000050: 7665 633a 3030 3030 3033 0000 0000 0000  vec:000003......
00000060: cdcc 8c3f 6666 0640 3333 4340 cdcc 8240  ...?ff.@33C@...@
00000070: cdcc a440 3333 c340 6666 e340 cdcc 0241  ...@33.@ff.@...A
00000080: 0000 0000 0000 0000 0200 0000 0000 0000  ................
00000090: 0100 0000 0000 0000 0000 0000 0000 0000  ................
```

---

## Summary

| Schema Element | Binary Layout | Access Pattern |
|---------------|---------------|----------------|
| Fixed field | `[data bytes]` | `mmap[record_offset + field_offset..+size]` |
| Variable field | `[u32 len][data][padding]` | Read len first, then data |
| Collection | `[u32 count][member0][member1]...[padding]` | Read count, iterate members |
| Keys section | `[key0][key1]...` | `mmap[keys_offset + idx * key_size..+key_size]` |
| Ground truth | `[q0_neighbors][q1_neighbors]...` | `mmap[gt_offset + qidx * k * id_size..+k*id_size]` |

**Key Properties:**
1. All offsets computable from schema at load time
2. O(1) access to any record/field
3. No runtime parsing of binary format
4. mmap provides zero-copy access
