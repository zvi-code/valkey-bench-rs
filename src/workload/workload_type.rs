//! Workload type definitions

/// Supported benchmark workload types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadType {
    // === Standard benchmarks ===
    Ping,
    Set,
    Get,
    Incr,
    Lpush,
    Rpush,
    Lpop,
    Rpop,
    Sadd,
    Spop,
    Hset,
    Zadd,
    Zpopmin,
    Lrange100,
    Lrange300,
    Lrange500,
    Lrange600,
    Mset,

    // === Vector search workloads ===
    /// Load vectors with HSET
    VecLoad,
    /// Load only ground truth vectors
    VecGtLoad,
    /// Query vectors with FT.SEARCH
    VecQuery,
    /// Delete vectors with GT protection (simple bitmap-based approach)
    VecDel,
    /// Update existing vectors
    VecUpdate,
}

impl WorkloadType {
    /// Parse workload type from string (case-insensitive)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ping" => Some(Self::Ping),
            "set" => Some(Self::Set),
            "get" => Some(Self::Get),
            "incr" => Some(Self::Incr),
            "lpush" => Some(Self::Lpush),
            "rpush" => Some(Self::Rpush),
            "lpop" => Some(Self::Lpop),
            "rpop" => Some(Self::Rpop),
            "sadd" => Some(Self::Sadd),
            "spop" => Some(Self::Spop),
            "hset" => Some(Self::Hset),
            "zadd" => Some(Self::Zadd),
            "zpopmin" => Some(Self::Zpopmin),
            "lrange" | "lrange_100" | "lrange100" => Some(Self::Lrange100),
            "lrange_300" | "lrange300" => Some(Self::Lrange300),
            "lrange_500" | "lrange500" => Some(Self::Lrange500),
            "lrange_600" | "lrange600" => Some(Self::Lrange600),
            "mset" => Some(Self::Mset),
            "vecload" | "vec-load" | "vec_load" => Some(Self::VecLoad),
            "vecgtload" | "vec-gt-load" | "vec_gt_load" => Some(Self::VecGtLoad),
            "vecquery" | "vec-query" | "vec_query" => Some(Self::VecQuery),
            "vecdelprotected" | "vec-del" | "vec_del_protected" => Some(Self::VecDel),
            "vecupdate" | "vec-update" | "vec_update" => Some(Self::VecUpdate),
            _ => None,
        }
    }

    /// Get display name
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ping => "PING",
            Self::Set => "SET",
            Self::Get => "GET",
            Self::Incr => "INCR",
            Self::Lpush => "LPUSH",
            Self::Rpush => "RPUSH",
            Self::Lpop => "LPOP",
            Self::Rpop => "RPOP",
            Self::Sadd => "SADD",
            Self::Spop => "SPOP",
            Self::Hset => "HSET",
            Self::Zadd => "ZADD",
            Self::Zpopmin => "ZPOPMIN",
            Self::Lrange100 => "LRANGE_100",
            Self::Lrange300 => "LRANGE_300",
            Self::Lrange500 => "LRANGE_500",
            Self::Lrange600 => "LRANGE_600",
            Self::Mset => "MSET",
            Self::VecLoad => "VEC-LOAD",
            Self::VecGtLoad => "VEC-GT-LOAD",
            Self::VecQuery => "VEC-QUERY",
            Self::VecDel => "VEC-DEL",
            Self::VecUpdate => "VEC-UPDATE",
        }
    }

    /// Check if workload requires dataset
    pub fn requires_dataset(&self) -> bool {
        matches!(self, Self::VecLoad | Self::VecGtLoad | Self::VecQuery | Self::VecUpdate | Self::VecDel)
    }

    /// Check if workload is a vector search operation
    pub fn is_vector_search(&self) -> bool {
        matches!(
            self,
            Self::VecLoad | Self::VecGtLoad | Self::VecQuery | Self::VecDel | Self::VecUpdate
        )
    }

    /// Check if workload modifies data (for read-from-replica routing)
    pub fn is_write(&self) -> bool {
        !matches!(
            self,
            Self::Ping
                | Self::Get
                | Self::VecQuery
                | Self::Lrange100
                | Self::Lrange300
                | Self::Lrange500
                | Self::Lrange600
        )
    }
}

impl std::fmt::Display for WorkloadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_workload_types() {
        assert_eq!(WorkloadType::parse("ping"), Some(WorkloadType::Ping));
        assert_eq!(WorkloadType::parse("PING"), Some(WorkloadType::Ping));
        assert_eq!(WorkloadType::parse("vecload"), Some(WorkloadType::VecLoad));
        assert_eq!(WorkloadType::parse("vec-load"), Some(WorkloadType::VecLoad));
        assert_eq!(WorkloadType::parse("unknown"), None);
    }

    #[test]
    fn test_requires_dataset() {
        assert!(WorkloadType::VecLoad.requires_dataset());
        assert!(WorkloadType::VecQuery.requires_dataset());
        assert!(!WorkloadType::Ping.requires_dataset());
        assert!(!WorkloadType::Set.requires_dataset());
    }

    #[test]
    fn test_is_write() {
        assert!(!WorkloadType::Ping.is_write());
        assert!(!WorkloadType::Get.is_write());
        assert!(WorkloadType::Set.is_write());
        assert!(WorkloadType::VecLoad.is_write());
        assert!(!WorkloadType::VecQuery.is_write());
    }
}
