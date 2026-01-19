//! Unified iterator core traits and implementations.
//!
//! This crate provides foundational abstractions for building efficient iterators
//! over coordinate spaces with bitmap-based filtering and various iteration strategies.
//!
//! # Core Traits
//!
//! - [`BitmapOps`]: Operations on atomic bitmaps (test, set, clear, find)
//! - [`CoordinateSpace`]: Defines coordinate spaces (1D, 2D, N-D)
//! - [`IterationStrategy`]: How to traverse a space (scan, sample, adaptive)
//! - [`Distribution`]: Statistical distributions for sampling
//!
//! # Key Types
//!
//! - [`GenericIter`]: Unified iterator combining space, strategy, and bitmap
//! - [`SingleDim`], [`TwoDim`], [`MultiDim`]: Coordinate space implementations
//! - [`ScanStrategy`], [`SampleStrategy`], [`AdaptiveStrategy`]: Iteration strategies
//!
//! # Example
//!
//! ```rust,ignore
//! use iterator_core::{GenericIter, SingleDim, ScanStrategy, AtomicBitmap};
//! use std::sync::Arc;
//!
//! let bitmap = Arc::new(AtomicBitmap::with_capacity(1000));
//! for i in 0..100 { bitmap.set(i); }
//!
//! // Scan-based iteration over set bits
//! let iter = GenericIter::builder(SingleDim::new(1000))
//!     .with_bitmap(bitmap)
//!     .set_only()
//!     .scan();
//!
//! for id in iter {
//!     println!("Found set bit at: {}", id);
//! }
//! ```

#![warn(missing_docs)]

// Core modules
mod bitmap;
mod coords;
mod distribution;
mod filter;
mod iterator;
mod strategy;
mod strides;

// Re-exports
pub use bitmap::AtomicBitmap;
#[cfg(target_arch = "aarch64")]
pub use bitmap::ArmCapabilities;
#[cfg(target_arch = "x86_64")]
pub use bitmap::X86Capabilities;
pub use bitmap::print_cpu_capabilities;

pub use coords::{CoordinateSpace, Coords, MultiDim, SingleDim, TwoDim};

pub use distribution::{
    Dist, DistState, Distribution, ExponentialDist, HotspotDist, LatestDist, NormalDist,
    PermuteDist, ReverseDist, SemiSequentialDist, SequentialDist, UniformDist, ZipfianDist,
};

pub use filter::{
    BitmapFilterConfig, BitmapOps, FilterCondition, NoBitmap, SecondaryFilter, UpdateAction,
};

pub use iterator::{GenericIter, IterBuilder};

pub use strategy::{
    AdaptiveStrategy, IterationStrategy, SampleStrategy, ScanStrategy, StrategyState,
};

pub use strides::{DynamicStrides, OptimizedStrides, StrideArray, Strides};
