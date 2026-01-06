//! Cluster topology and node management
//!
//! This module provides cluster support including:
//! - Topology discovery via CLUSTER NODES
//! - Slot mapping and CRC16 calculation
//! - Read-from-replica strategies
//! - Node selection
//! - Dynamic topology refresh on MOVED/ASK errors
//! - Backend abstraction for different engine types (ElastiCache, MemoryDB, Valkey OSS)
//!
//! # Keyspace Tracking
//!
//! Vector existence tracking and protected IDs have been moved to the `keyspace` module
//! which uses the `keyspace_tracker` crate for high-performance bitmap operations.

pub mod backend;
pub mod node;
pub mod topology;
pub mod topology_manager;

pub use backend::{
    create_backend, AutoDetectBackend, ClusterBackend, ConnectionConfig, ElastiCacheBackend,
    MemoryDBBackend, UnknownBackend, ValkeyOSSBackend,
};
pub use node::ClusterNode;
pub use topology::{truncate_node_address, ClusterTopology};
pub use topology_manager::{RedirectInfo, TopologyManager};

