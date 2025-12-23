//! Shared parsing utilities
//!
//! Provides common parsing functions used across the codebase:
//! - Memory value parsing with K/M/G/T suffixes
//! - Enum string conversion macro

/// Parse memory value with K/M/G/T suffixes (case-insensitive)
///
/// Supports both raw bytes and human-readable formats:
/// - "1024" -> 1024
/// - "1K" or "1k" or "1kb" -> 1024
/// - "1M" or "1m" or "1mb" -> 1048576
/// - "1.5G" -> 1610612736
///
/// Returns None if parsing fails.
pub fn parse_memory_value(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Find where the numeric part ends
    let s_lower = s.to_lowercase();
    let (num_str, suffix) = if s_lower.ends_with("kb") {
        (&s[..s.len() - 2], 1024i64)
    } else if s_lower.ends_with("mb") {
        (&s[..s.len() - 2], 1024 * 1024)
    } else if s_lower.ends_with("gb") {
        (&s[..s.len() - 2], 1024 * 1024 * 1024)
    } else if s_lower.ends_with("tb") {
        (&s[..s.len() - 2], 1024 * 1024 * 1024 * 1024)
    } else {
        match s.chars().last()? {
            'K' | 'k' => (&s[..s.len() - 1], 1024i64),
            'M' | 'm' => (&s[..s.len() - 1], 1024 * 1024),
            'G' | 'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
            'T' | 't' => (&s[..s.len() - 1], 1024 * 1024 * 1024 * 1024),
            _ => (s, 1),
        }
    };

    // Try float first (handles "1.5G"), then integer
    if let Ok(f) = num_str.parse::<f64>() {
        Some((f * suffix as f64) as i64)
    } else {
        num_str.parse::<i64>().ok().map(|v| v * suffix)
    }
}

/// Parse memory value as u64 (for config values that must be positive)
pub fn parse_memory_value_u64(s: &str) -> Option<u64> {
    parse_memory_value(s).and_then(|v| if v >= 0 { Some(v as u64) } else { None })
}

/// Macro for creating enums with FromStr and Display implementations
///
/// This macro generates an enum with:
/// - `from_str(&str) -> Option<Self>` method (case-insensitive)
/// - `name(&self) -> &'static str` method
/// - `Display` implementation using name()
/// - `FromStr` implementation using from_str()
///
/// # Example
///
/// ```ignore
/// enum_str! {
///     /// Distance metric
///     pub enum DistanceMetric {
///         L2 => "l2" | "L2",
///         InnerProduct => "ip" | "innerproduct" | "inner_product",
///         Cosine => "cosine",
///     }
/// }
/// ```
#[macro_export]
macro_rules! enum_str {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $canonical:literal $(| $alias:literal)*
            ),* $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )*
        }

        impl $name {
            /// Get the canonical string name
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant => $canonical,)*
                }
            }

            /// Parse from string (case-insensitive)
            pub fn from_str_opt(s: &str) -> Option<Self> {
                let lower = s.to_lowercase();
                match lower.as_str() {
                    $($canonical $(| $alias)* => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::from_str_opt(s).ok_or_else(|| {
                    let variants = vec![$($canonical),*];
                    format!(
                        "Unknown {}: '{}'. Valid: {}",
                        stringify!($name), s, variants.join(", ")
                    )
                })
            }
        }
    };
}

/// Helper for parsing colon-separated formats like "name:value:value2"
///
/// This is commonly used for CLI argument parsing.
pub struct ColonParser<'a> {
    parts: Vec<&'a str>,
    context: &'a str,
}

impl<'a> ColonParser<'a> {
    /// Create a new parser for the given input
    pub fn new(input: &'a str, context: &'a str) -> Self {
        let parts: Vec<&str> = input.split(':').collect();
        Self { parts, context }
    }

    /// Get the number of parts
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Require exactly N parts, returning an error message if not
    pub fn require_parts(&self, n: usize) -> Result<(), String> {
        if self.parts.len() != n {
            Err(format!(
                "Invalid {} format: expected {} parts separated by ':', got {}",
                self.context, n, self.parts.len()
            ))
        } else {
            Ok(())
        }
    }

    /// Require at least N parts
    pub fn require_min_parts(&self, n: usize) -> Result<(), String> {
        if self.parts.len() < n {
            Err(format!(
                "Invalid {} format: expected at least {} parts separated by ':', got {}",
                self.context, n, self.parts.len()
            ))
        } else {
            Ok(())
        }
    }

    /// Get part at index (0-based)
    pub fn part(&self, index: usize) -> Option<&'a str> {
        self.parts.get(index).copied()
    }

    /// Parse part at index as a type
    pub fn parse<T: std::str::FromStr>(&self, index: usize) -> Result<T, String>
    where
        T::Err: std::fmt::Display,
    {
        self.parts
            .get(index)
            .ok_or_else(|| format!("Missing part {} in {}", index + 1, self.context))?
            .parse::<T>()
            .map_err(|e| format!("Invalid part {} in {}: {}", index + 1, self.context, e))
    }

    /// Parse part at index with custom error context
    pub fn parse_with_context<T: std::str::FromStr>(
        &self,
        index: usize,
        field_name: &str,
    ) -> Result<T, String>
    where
        T::Err: std::fmt::Display,
    {
        self.parts
            .get(index)
            .ok_or_else(|| format!("Missing {} in {}", field_name, self.context))?
            .parse::<T>()
            .map_err(|e| format!("Invalid {} '{}': {}", field_name, self.parts[index], e))
    }

    /// Get iterator over all parts
    pub fn iter(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.parts.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_value_bytes() {
        assert_eq!(parse_memory_value("1024"), Some(1024));
        assert_eq!(parse_memory_value("0"), Some(0));
        assert_eq!(parse_memory_value("-100"), Some(-100));
    }

    #[test]
    fn test_parse_memory_value_suffixes() {
        assert_eq!(parse_memory_value("1K"), Some(1024));
        assert_eq!(parse_memory_value("1k"), Some(1024));
        assert_eq!(parse_memory_value("1kb"), Some(1024));
        assert_eq!(parse_memory_value("1KB"), Some(1024));

        assert_eq!(parse_memory_value("1M"), Some(1024 * 1024));
        assert_eq!(parse_memory_value("1m"), Some(1024 * 1024));
        assert_eq!(parse_memory_value("1mb"), Some(1024 * 1024));

        assert_eq!(parse_memory_value("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_value("1g"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_value("1gb"), Some(1024 * 1024 * 1024));

        assert_eq!(parse_memory_value("10G"), Some(10 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_memory_value_float() {
        assert_eq!(parse_memory_value("1.5G"), Some((1.5 * 1024.0 * 1024.0 * 1024.0) as i64));
        assert_eq!(parse_memory_value("0.5M"), Some((0.5 * 1024.0 * 1024.0) as i64));
    }

    #[test]
    fn test_parse_memory_value_edge_cases() {
        assert_eq!(parse_memory_value(""), None);
        assert_eq!(parse_memory_value("  "), None);
        assert_eq!(parse_memory_value("  1024  "), Some(1024));
    }

    enum_str! {
        /// Test enum for macro testing
        pub enum TestMetric {
            Qps => "qps" | "throughput",
            Recall => "recall",
            P99Ms => "p99_ms" | "p99",
        }
    }

    #[test]
    fn test_enum_str_macro() {
        assert_eq!(TestMetric::Qps.name(), "qps");
        assert_eq!(TestMetric::from_str_opt("qps"), Some(TestMetric::Qps));
        assert_eq!(TestMetric::from_str_opt("THROUGHPUT"), Some(TestMetric::Qps));
        assert_eq!(TestMetric::from_str_opt("P99"), Some(TestMetric::P99Ms));
        assert_eq!(TestMetric::from_str_opt("invalid"), None);

        assert_eq!(format!("{}", TestMetric::Recall), "recall");
    }
}
