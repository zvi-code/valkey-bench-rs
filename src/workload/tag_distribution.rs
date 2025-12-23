//! Tag distribution for vector search benchmarks
//!
//! This module provides tag generation with configurable probability distributions.
//! Tags are added to vectors during vec-load and filtered during vec-query.
//!
//! Format: "pattern:probability,pattern:probability,..."
//! Example: "electronics:30,clothing:25,home:20,sports:15,other:10"
//!
//! Each pattern can contain placeholders:
//! - `__rand_int__` - Replaced with a random integer 0-999999
//!
//! Probabilities are independent - each tag has its own chance of being included.
//! A vector may have 0, 1, or multiple tags based on the probabilities.

/// Single tag distribution entry
#[derive(Debug, Clone)]
pub struct TagDistribution {
    /// Tag pattern (may contain __rand_int__ placeholder)
    pub pattern: String,
    /// Probability percentage (0-100) for this tag to be included
    pub percentage: f64,
}

/// Collection of tag distributions for generating vector tags
#[derive(Debug, Clone, Default)]
pub struct TagDistributionSet {
    /// List of tag distributions
    distributions: Vec<TagDistribution>,
    /// Maximum tag field length (for fixed-size templates)
    max_tag_len: usize,
}

impl TagDistributionSet {
    /// Create empty tag distribution set
    pub fn new() -> Self {
        Self {
            distributions: Vec::new(),
            max_tag_len: 128,
        }
    }

    /// Create from a distribution string
    ///
    /// Format: "tag1:50,tag2:30,tag3__rand_int__:20"
    /// Each entry is "pattern:percentage"
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut distributions = Vec::new();

        for entry in input.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            // Find last colon (pattern may contain colons)
            let colon_pos = entry.rfind(':').ok_or_else(|| {
                format!(
                    "Invalid tag distribution format: '{}' (expected 'pattern:percentage')",
                    entry
                )
            })?;

            let pattern = entry[..colon_pos].to_string();
            let percentage_str = &entry[colon_pos + 1..];
            let percentage: f64 = percentage_str.parse().map_err(|_| {
                format!(
                    "Invalid percentage '{}' in tag distribution",
                    percentage_str
                )
            })?;

            if !(0.0..=100.0).contains(&percentage) {
                return Err(format!("Percentage must be 0-100, got {}", percentage));
            }

            distributions.push(TagDistribution {
                pattern,
                percentage,
            });
        }

        Ok(Self {
            distributions,
            max_tag_len: 128,
        })
    }

    /// Set maximum tag field length
    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_tag_len = max_len;
        self
    }

    /// Get maximum tag field length
    pub fn max_tag_len(&self) -> usize {
        self.max_tag_len
    }

    /// Check if any distributions are configured
    pub fn is_empty(&self) -> bool {
        self.distributions.is_empty()
    }

    /// Number of tag distribution entries
    pub fn len(&self) -> usize {
        self.distributions.len()
    }

    /// Select tags based on independent probabilities
    ///
    /// Each tag has its own probability of being included.
    /// Returns comma-separated list of selected tags, or None if no tags selected.
    ///
    /// Uses thread-local RNG for simplicity. For seeded RNG, use `select_tags_with_rng`.
    pub fn select_tags(&self) -> Option<String> {
        if self.distributions.is_empty() {
            return None;
        }

        let mut selected = Vec::new();

        for dist in &self.distributions {
            let roll: f64 = fastrand::f64() * 100.0;
            if roll <= dist.percentage {
                // This tag is selected
                let tag = self.process_pattern(&dist.pattern);
                selected.push(tag);
            }
        }

        if selected.is_empty() {
            None
        } else {
            Some(selected.join(","))
        }
    }

    /// Select tags with a specific seeded RNG
    pub fn select_tags_seeded(&self, seed: u64) -> Option<String> {
        if self.distributions.is_empty() {
            return None;
        }

        let mut rng = fastrand::Rng::with_seed(seed);
        let mut selected = Vec::new();

        for dist in &self.distributions {
            let roll: f64 = rng.f64() * 100.0;
            if roll <= dist.percentage {
                // This tag is selected
                let tag = self.process_pattern_rng(&dist.pattern, &mut rng);
                selected.push(tag);
            }
        }

        if selected.is_empty() {
            None
        } else {
            Some(selected.join(","))
        }
    }

    /// Process pattern placeholders using thread-local RNG
    fn process_pattern(&self, pattern: &str) -> String {
        let mut result = pattern.to_string();

        // Replace __rand_int__ with random integer
        while let Some(pos) = result.find("__rand_int__") {
            let rand_val: u32 = fastrand::u32(0..1_000_000);
            result.replace_range(pos..pos + 12, &rand_val.to_string());
        }

        result
    }

    /// Process pattern placeholders with specific RNG
    fn process_pattern_rng(&self, pattern: &str, rng: &mut fastrand::Rng) -> String {
        let mut result = pattern.to_string();

        // Replace __rand_int__ with random integer
        while let Some(pos) = result.find("__rand_int__") {
            let rand_val: u32 = rng.u32(0..1_000_000);
            result.replace_range(pos..pos + 12, &rand_val.to_string());
        }

        result
    }

    /// Get iterator over distributions
    pub fn iter(&self) -> impl Iterator<Item = &TagDistribution> {
        self.distributions.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let set = TagDistributionSet::parse("electronics:50,clothing:30").unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.distributions[0].pattern, "electronics");
        assert_eq!(set.distributions[0].percentage, 50.0);
        assert_eq!(set.distributions[1].pattern, "clothing");
        assert_eq!(set.distributions[1].percentage, 30.0);
    }

    #[test]
    fn test_parse_with_placeholder() {
        let set = TagDistributionSet::parse("cat_id__rand_int__:100").unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.distributions[0].pattern, "cat_id__rand_int__");
    }

    #[test]
    fn test_parse_invalid_format() {
        assert!(TagDistributionSet::parse("no_percentage").is_err());
    }

    #[test]
    fn test_parse_invalid_percentage() {
        assert!(TagDistributionSet::parse("tag:abc").is_err());
        assert!(TagDistributionSet::parse("tag:150").is_err());
        assert!(TagDistributionSet::parse("tag:-10").is_err());
    }

    #[test]
    fn test_select_tags_100_percent() {
        let set = TagDistributionSet::parse("always:100").unwrap();

        // Should always be selected with seeded RNG
        for i in 0..10 {
            let tags = set.select_tags_seeded(42 + i);
            assert!(tags.is_some());
            assert_eq!(tags.unwrap(), "always");
        }
    }

    #[test]
    fn test_select_tags_0_percent() {
        let set = TagDistributionSet::parse("never:0").unwrap();

        // Should never be selected
        for i in 0..10 {
            let tags = set.select_tags_seeded(42 + i);
            assert!(tags.is_none());
        }
    }

    #[test]
    fn test_select_multiple_tags() {
        let set = TagDistributionSet::parse("a:100,b:100,c:100").unwrap();

        let tags = set.select_tags_seeded(42).unwrap();
        assert!(tags.contains('a'));
        assert!(tags.contains('b'));
        assert!(tags.contains('c'));
        assert_eq!(tags, "a,b,c");
    }

    #[test]
    fn test_rand_int_placeholder() {
        let set = TagDistributionSet::parse("id__rand_int__:100").unwrap();

        let tags = set.select_tags_seeded(42).unwrap();
        assert!(tags.starts_with("id"));
        assert!(!tags.contains("__rand_int__"));

        // Should be numeric after "id"
        assert!(tags.chars().skip(2).all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_empty_set() {
        let set = TagDistributionSet::new();
        assert!(set.is_empty());
        assert!(set.select_tags().is_none());
    }
}
