# iterator-core

Unified iterator core traits and implementations for high-performance iteration over coordinate spaces with bitmap-based filtering and various iteration strategies.

## Overview

This crate provides a unified abstraction layer that combines:

- **Coordinate Spaces**: Define what coordinates to iterate over (1D, 2D, N-D)
- **Iteration Strategies**: Define how to traverse the space (scan, sample, adaptive)
- **Bitmap Filtering**: Optional filtering based on bitmap state (set/unset bits)

## Core Traits

### `CoordinateSpace`

Defines a coordinate space for iteration:

- `SingleDim` - 1-dimensional space (simple ID tracking)
- `TwoDim` - 2-dimensional space ((id, sub_id) pairs)
- `MultiDim` - N-dimensional space (arbitrary Cartesian products)

### `IterationStrategy<B: BitmapOps>`

Strategy for traversing a coordinate space:

- `ScanStrategy` - SIMD-accelerated bitmap scanning (find_next_set/unset)
- `SampleStrategy` - Distribution-based sampling with retry on filter mismatch
- `AdaptiveStrategy` - Auto-selects scan or sample based on density
- `SequentialStrategy` - Simple sequential iteration (0, 1, 2, ...)

### `BitmapOps`

Trait for atomic bitmap operations:

- `AtomicBitmap` - Lock-free concurrent bitmap with SIMD acceleration
- `NoBitmap` - No-op implementation for unfiltered iteration

### `Distribution`

Statistical distributions for sampling:

- `Sequential`, `Uniform`, `Zipfian`, `Normal`, `Exponential`, `Hotspot`, `Latest`

## Usage

```rust
use iterator_core::{IterBuilder, SingleDim, MultiDim, AtomicBitmap, Dist};
use std::sync::Arc;

// Simple 1D iteration without bitmap
let iter = IterBuilder::new(SingleDim::new(100))
    .with_limit(50)
    .sequential();

for id in iter {
    println!("ID: {}", id);
}

// 1D with bitmap filtering - only iterate over set bits
let bitmap = Arc::new(AtomicBitmap::with_capacity(1000));
bitmap.set(10);
bitmap.set(20);
bitmap.set(30);

let iter = IterBuilder::new(SingleDim::new(1000))
    .with_bitmap(bitmap)
    .set_only()
    .scan();

for id in iter {
    println!("Set bit at: {}", id);
}

// Multi-dimensional with Zipfian distribution
let iter = IterBuilder::new(MultiDim::new(&[100, 200]))
    .with_limit(5000)
    .zipfian(0.99);

for coords in iter {
    println!("Coords: {:?}", &coords[..]);
}
```

## Strategy Selection Guide

| Strategy | Best For | Bitmap Density | Distribution |
|----------|----------|----------------|--------------|
| `sequential()` | Simple enumeration | Any | Sequential only |
| `scan()` | Write/delete operations, cursor-based | Any | Sequential only |
| `sample(dist)` | Random access patterns | >10% | Any distribution |
| `adaptive(dist)` | Unknown workloads | Auto-selects | Any distribution |

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        GenericIter<C, S, B>                     │
│                                                                 │
│  Combines: CoordinateSpace + IterationStrategy + BitmapOps     │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────────────┐ │
│  │ SingleDim   │  │ ScanStrategy    │  │ AtomicBitmap        │ │
│  │ TwoDim      │  │ SampleStrategy  │  │ NoBitmap            │ │
│  │ MultiDim    │  │ AdaptiveStrategy│  │ (custom BitmapOps)  │ │
│  └─────────────┘  └─────────────────┘  └─────────────────────┘ │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Features

- **SIMD-accelerated bitmap operations** (ARM NEON/SVE, x86 AVX2/AVX-512)
- **Lock-free concurrent access** via atomic operations
- **Compile-time specialization** for common cases (1D, 2D)
- **SmallVec optimization** for N-D coordinates (stack-allocated for ≤4 dimensions)
- **Distribution support** for realistic workload generation

## Integration

This crate is designed to unify the iteration patterns from:

- `keyspace_tracker`: Key existence tracking with concurrent operations
- `iterators-rs`: N-dimensional workload generation with distributions

Both can depend on `iterator-core` for shared abstractions while maintaining their specialized APIs.

## License

BSD-3-Clause
