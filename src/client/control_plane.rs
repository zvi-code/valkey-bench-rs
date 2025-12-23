//! Control Plane trait for server communication
//!
//! This trait abstracts control plane operations (non-benchmark traffic)
//! such as cluster discovery, index management, and metrics collection.
//! 
//! The trait allows different implementations:
//! - `RawConnection`: Direct TCP/TLS with custom RESP codec
//! - Future: Glide-based implementation

use crate::utils::{RespEncoder, RespValue};
use std::io;

/// Control plane operations trait
/// 
/// Implementations handle the underlying protocol and connection management.
/// Higher-level operations (FT.CREATE, INFO parsing, etc.) are built on top.
pub trait ControlPlane {
    /// Execute a command with string arguments
    /// 
    /// # Arguments
    /// * `args` - Command and arguments as string slices
    /// 
    /// # Example
    /// ```ignore
    /// let response = conn.execute(&["PING"])?;
    /// let response = conn.execute(&["SET", "key", "value"])?;
    /// ```
    fn execute(&mut self, args: &[&str]) -> io::Result<RespValue>;
    
    /// Execute a command with binary arguments
    /// 
    /// Needed for commands with binary data (vectors, etc.)
    /// 
    /// # Arguments
    /// * `args` - Command and arguments as byte slices
    fn execute_binary(&mut self, args: &[&[u8]]) -> io::Result<RespValue>;
    
    /// Execute a pre-encoded RESP command
    /// 
    /// For cases where the caller has already encoded the command.
    /// This is the lowest-level execution method.
    fn execute_encoded(&mut self, encoder: &RespEncoder) -> io::Result<RespValue>;
}

/// Extension trait with common control plane operations
/// 
/// These are convenience methods built on top of the base `ControlPlane` trait.
pub trait ControlPlaneExt: ControlPlane {
    /// Send PING and verify PONG response
    fn ping(&mut self) -> io::Result<bool> {
        match self.execute(&["PING"])? {
            RespValue::SimpleString(s) => Ok(s == "PONG"),
            _ => Ok(false),
        }
    }
    
    /// Get CLUSTER NODES response as string
    fn cluster_nodes(&mut self) -> io::Result<String> {
        match self.execute(&["CLUSTER", "NODES"])? {
            RespValue::BulkString(data) => {
                String::from_utf8(data).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UTF-8: {}", e))
                })
            }
            RespValue::Error(e) => Err(io::Error::other(e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected CLUSTER NODES response: {:?}", other),
            )),
        }
    }
    
    /// Check if this is a cluster node
    fn is_cluster(&mut self) -> bool {
        self.cluster_nodes().is_ok()
    }
    
    /// Get INFO for a section (empty string or "all" returns all sections)
    fn info(&mut self, section: &str) -> io::Result<String> {
        // Build command - use just INFO for empty section, otherwise INFO <section>
        let response = if section.is_empty() {
            self.execute(&["INFO"])?
        } else {
            self.execute(&["INFO", section])?
        };
        
        match response {
            RespValue::BulkString(data) => {
                String::from_utf8(data).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UTF-8: {}", e))
                })
            }
            RespValue::Error(e) => Err(io::Error::other(e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected INFO response: {:?}", other),
            )),
        }
    }
    
    /// Send AUTH command
    fn authenticate(&mut self, password: &str, username: Option<&str>) -> io::Result<()> {
        let response = match username {
            Some(user) => self.execute(&["AUTH", user, password])?,
            None => self.execute(&["AUTH", password])?,
        };
        
        match response {
            RespValue::SimpleString(s) if s == "OK" => Ok(()),
            RespValue::Error(e) => Err(io::Error::new(io::ErrorKind::PermissionDenied, e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected AUTH response: {:?}", other),
            )),
        }
    }
    
    /// Send SELECT command
    fn select_db(&mut self, db: u32) -> io::Result<()> {
        let db_str = db.to_string();
        match self.execute(&["SELECT", &db_str])? {
            RespValue::SimpleString(s) if s == "OK" => Ok(()),
            RespValue::Error(e) => Err(io::Error::other(e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected SELECT response: {:?}", other),
            )),
        }
    }
    
    /// Send FLUSHDB command
    fn flushdb(&mut self) -> io::Result<()> {
        match self.execute(&["FLUSHDB"])? {
            RespValue::SimpleString(s) if s == "OK" => Ok(()),
            RespValue::Error(e) => Err(io::Error::other(e)),
            _ => Ok(()), // Accept any success
        }
    }
    
    /// Send DBSIZE command
    fn dbsize(&mut self) -> io::Result<i64> {
        match self.execute(&["DBSIZE"])? {
            RespValue::Integer(n) => Ok(n),
            RespValue::Error(e) => Err(io::Error::other(e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected DBSIZE response: {:?}", other),
            )),
        }
    }
}

// Blanket implementation: any ControlPlane automatically gets ControlPlaneExt
impl<T: ControlPlane> ControlPlaneExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    
    // Mock implementation for testing
    struct MockControlPlane {
        responses: Vec<RespValue>,
        call_count: usize,
    }
    
    impl MockControlPlane {
        fn new(responses: Vec<RespValue>) -> Self {
            Self { responses, call_count: 0 }
        }
    }
    
    impl ControlPlane for MockControlPlane {
        fn execute(&mut self, _args: &[&str]) -> io::Result<RespValue> {
            if self.call_count < self.responses.len() {
                let resp = self.responses[self.call_count].clone();
                self.call_count += 1;
                Ok(resp)
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "No more responses"))
            }
        }
        
        fn execute_binary(&mut self, _args: &[&[u8]]) -> io::Result<RespValue> {
            self.execute(&[])
        }
        
        fn execute_encoded(&mut self, _encoder: &RespEncoder) -> io::Result<RespValue> {
            self.execute(&[])
        }
    }
    
    #[test]
    fn test_ping() {
        let mut mock = MockControlPlane::new(vec![
            RespValue::SimpleString("PONG".to_string()),
        ]);
        assert!(mock.ping().unwrap());
    }
    
    #[test]
    fn test_dbsize() {
        let mut mock = MockControlPlane::new(vec![
            RespValue::Integer(12345),
        ]);
        assert_eq!(mock.dbsize().unwrap(), 12345);
    }
}
