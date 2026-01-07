//! Command template builder with placeholder support
//!
//! This module creates RESP-encoded command templates where placeholder
//! regions are marked for in-place replacement during the benchmark.

use crate::client::{CommandBuffer, PlaceholderOffset, PlaceholderType};
use crate::utils::RespEncoder;

/// Template argument (literal or placeholder)
#[derive(Debug, Clone)]
pub enum TemplateArg {
    /// Literal bytes (copied as-is)
    Literal(Vec<u8>),
    /// Placeholder with type and reserved length
    Placeholder {
        ph_type: PlaceholderType,
        len: usize,
    },
    /// Prefixed placeholder (prefix + placeholder in single arg)
    PrefixedPlaceholder {
        prefix: Vec<u8>,
        ph_type: PlaceholderType,
        len: usize,
    },
}

/// Command template definition
#[derive(Debug, Clone)]
pub struct CommandTemplate {
    /// Template arguments
    args: Vec<TemplateArg>,
    /// Command name for display
    name: String,
}

impl CommandTemplate {
    /// Create new command template
    pub fn new(name: &str) -> Self {
        Self {
            args: Vec::new(),
            name: name.to_string(),
        }
    }

    /// Add literal argument
    pub fn arg_literal(mut self, value: &[u8]) -> Self {
        self.args.push(TemplateArg::Literal(value.to_vec()));
        self
    }

    /// Add string literal argument
    pub fn arg_str(mut self, value: &str) -> Self {
        self.args
            .push(TemplateArg::Literal(value.as_bytes().to_vec()));
        self
    }

    /// Add key placeholder (fixed-width decimal)
    pub fn arg_key(mut self, width: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::Key,
            len: width,
        });
        self
    }

    /// Add vector placeholder (binary blob) - for database vectors
    pub fn arg_vector(mut self, byte_len: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::Vector,
            len: byte_len,
        });
        self
    }

    /// Add query vector placeholder (binary blob) - for FT.SEARCH queries
    pub fn arg_query_vector(mut self, byte_len: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::QueryVector,
            len: byte_len,
        });
        self
    }

    /// Add cluster tag placeholder
    pub fn arg_cluster_tag(mut self) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::ClusterTag,
            len: 5, // {xxx}
        });
        self
    }

    /// Add random integer placeholder
    pub fn arg_rand_int(mut self, width: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::RandInt,
            len: width,
        });
        self
    }

    /// Add tag field placeholder (variable length, padded with commas)
    ///
    /// Tags are stored as comma-separated values. The placeholder is
    /// padded to max_len with trailing commas for fixed-size templates.
    pub fn arg_tag_placeholder(mut self, max_len: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::Tag,
            len: max_len,
        });
        self
    }

    /// Add numeric field placeholder (fixed-width decimal) - backward compatibility
    ///
    /// Used for numeric attributes like timestamps or scores.
    /// Format: 12-digit decimal number.
    pub fn arg_numeric_placeholder(mut self) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::Numeric,
            len: 12, // Fixed 12-digit width for numeric values
        });
        self
    }

    /// Add indexed numeric field placeholder with specified max length
    ///
    /// Used for numeric fields with configurable type and distribution.
    /// The index references the field in the NumericFieldSet.
    pub fn arg_numeric_field(mut self, field_idx: usize, max_len: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::NumericField(field_idx),
            len: max_len,
        });
        self
    }

    /// Add hash field placeholder (variable length, padded with spaces)
    ///
    /// Field names are filled from AddressableSpace at runtime.
    /// The placeholder is padded to max_len with trailing spaces.
    pub fn arg_field(mut self, max_len: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::Field,
            len: max_len,
        });
        self
    }

    /// Add JSON path placeholder (variable length, padded with spaces)
    ///
    /// JSON paths are filled from AddressableSpace at runtime.
    /// The placeholder is padded to max_len with trailing spaces.
    pub fn arg_json_path_placeholder(mut self, max_len: usize) -> Self {
        self.args.push(TemplateArg::Placeholder {
            ph_type: PlaceholderType::JsonPath,
            len: max_len,
        });
        self
    }

    /// Add prefixed key placeholder (prefix + fixed-width decimal in single arg)
    /// The key will be: prefix + 0-padded decimal number
    pub fn arg_prefixed_key(mut self, prefix: &str, width: usize) -> Self {
        // We need a special TemplateArg that has both a prefix and placeholder
        self.args.push(TemplateArg::PrefixedPlaceholder {
            prefix: prefix.as_bytes().to_vec(),
            ph_type: PlaceholderType::Key,
            len: width,
        });
        self
    }

    /// Get template name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Build RESP-encoded command buffer for given pipeline size
    pub fn build(&self, pipeline: usize) -> CommandBuffer {
        // Build single command first to get byte offsets
        let (single_bytes, single_placeholders) = self.build_single();
        let single_len = single_bytes.len();

        // Replicate for pipeline
        let total_len = single_len * pipeline;
        let mut bytes = Vec::with_capacity(total_len);
        let mut all_placeholders = Vec::with_capacity(pipeline);

        for _cmd_idx in 0..pipeline {
            bytes.extend_from_slice(&single_bytes);

            // Adjust placeholder offsets for this command position
            let cmd_placeholders: Vec<PlaceholderOffset> = single_placeholders
                .iter()
                .map(|ph| PlaceholderOffset {
                    offset: ph.offset, // Keep relative offset
                    len: ph.len,
                    placeholder_type: ph.placeholder_type,
                })
                .collect();

            all_placeholders.push(cmd_placeholders);
        }

        let mut buf = CommandBuffer::new(bytes, pipeline);
        buf.placeholders = all_placeholders;
        buf.command_len = single_len;
        buf
    }

    /// Build single command and return (bytes, placeholder_offsets)
    fn build_single(&self) -> (Vec<u8>, Vec<PlaceholderOffset>) {
        let mut encoder = RespEncoder::with_capacity(4096);
        let mut placeholders = Vec::new();

        // Build array header: *<count>\r\n
        encoder.buffer_mut().push(b'*');
        let count_str = self.args.len().to_string();
        encoder.buffer_mut().extend_from_slice(count_str.as_bytes());
        encoder.buffer_mut().extend_from_slice(b"\r\n");

        // Build each argument
        for arg in &self.args {
            match arg {
                TemplateArg::Literal(data) => {
                    // $<len>\r\n<data>\r\n
                    encoder.buffer_mut().push(b'$');
                    let len_str = data.len().to_string();
                    encoder.buffer_mut().extend_from_slice(len_str.as_bytes());
                    encoder.buffer_mut().extend_from_slice(b"\r\n");
                    encoder.buffer_mut().extend_from_slice(data);
                    encoder.buffer_mut().extend_from_slice(b"\r\n");
                }
                TemplateArg::Placeholder { ph_type, len } => {
                    // $<len>\r\n<placeholder>\r\n
                    encoder.buffer_mut().push(b'$');
                    let len_str = len.to_string();
                    encoder.buffer_mut().extend_from_slice(len_str.as_bytes());
                    encoder.buffer_mut().extend_from_slice(b"\r\n");

                    // Record offset before writing placeholder
                    let offset = encoder.as_bytes().len();
                    placeholders.push(PlaceholderOffset {
                        offset,
                        len: *len,
                        placeholder_type: *ph_type,
                    });

                    // Write placeholder bytes (zeros)
                    encoder.buffer_mut().extend(std::iter::repeat_n(b'0', *len));
                    encoder.buffer_mut().extend_from_slice(b"\r\n");
                }
                TemplateArg::PrefixedPlaceholder { prefix, ph_type, len } => {
                    // $<total_len>\r\n<prefix><placeholder>\r\n
                    let total_len = prefix.len() + len;
                    encoder.buffer_mut().push(b'$');
                    let len_str = total_len.to_string();
                    encoder.buffer_mut().extend_from_slice(len_str.as_bytes());
                    encoder.buffer_mut().extend_from_slice(b"\r\n");

                    // Write prefix first
                    encoder.buffer_mut().extend_from_slice(prefix);

                    // Record offset for placeholder (after prefix)
                    let offset = encoder.as_bytes().len();
                    placeholders.push(PlaceholderOffset {
                        offset,
                        len: *len,
                        placeholder_type: *ph_type,
                    });

                    // Write placeholder bytes (zeros)
                    encoder.buffer_mut().extend(std::iter::repeat_n(b'0', *len));
                    encoder.buffer_mut().extend_from_slice(b"\r\n");
                }
            }
        }

        (encoder.into_bytes(), placeholders)
    }
}

/// Template for key with prefix (e.g., "key:0000000001")
pub fn key_with_prefix(prefix: &str, width: usize) -> CommandTemplate {
    CommandTemplate::new("KEY").arg_literal(format!("{}{}", prefix, "0".repeat(width)).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command_template() {
        let template = CommandTemplate::new("PING").arg_str("PING");
        let buf = template.build(1);

        assert_eq!(buf.bytes, b"*1\r\n$4\r\nPING\r\n");
        assert_eq!(buf.pipeline_size, 1);
        assert!(buf.placeholders[0].is_empty());
    }

    #[test]
    fn test_command_with_key_placeholder() {
        let template = CommandTemplate::new("GET").arg_str("GET").arg_key(10); // 10-digit key placeholder

        let buf = template.build(1);

        // *2\r\n$3\r\nGET\r\n$10\r\n0000000000\r\n
        assert_eq!(buf.pipeline_size, 1);
        assert_eq!(buf.placeholders[0].len(), 1);
        assert_eq!(buf.placeholders[0][0].len, 10);
        assert_eq!(
            buf.placeholders[0][0].placeholder_type,
            PlaceholderType::Key
        );
    }

    #[test]
    fn test_set_command_template() {
        let template = CommandTemplate::new("SET")
            .arg_str("SET")
            .arg_key(10)
            .arg_str("value");

        let buf = template.build(1);

        // Verify structure
        assert_eq!(buf.placeholders[0].len(), 1);
        let ph = &buf.placeholders[0][0];
        assert_eq!(ph.placeholder_type, PlaceholderType::Key);
        assert_eq!(ph.len, 10);
    }

    #[test]
    fn test_pipeline_template() {
        let template = CommandTemplate::new("GET").arg_str("GET").arg_key(10);

        let buf = template.build(3);

        assert_eq!(buf.pipeline_size, 3);
        assert_eq!(buf.placeholders.len(), 3);

        // Each command should have the same relative placeholder offset
        let first_offset = buf.placeholders[0][0].offset;
        for cmd_phs in buf.placeholders.iter() {
            assert_eq!(cmd_phs.len(), 1);
            assert_eq!(cmd_phs[0].offset, first_offset); // Same relative offset
            assert_eq!(cmd_phs[0].len, 10);
        }

        // Total bytes should be 3x single command
        assert_eq!(buf.bytes.len(), buf.command_len * 3);
    }

    #[test]
    fn test_vector_placeholder() {
        let template = CommandTemplate::new("HSET")
            .arg_str("HSET")
            .arg_key(12)
            .arg_str("embedding")
            .arg_vector(512); // 128 floats * 4 bytes

        let buf = template.build(1);

        assert_eq!(buf.placeholders[0].len(), 2);

        // First placeholder is key
        assert_eq!(
            buf.placeholders[0][0].placeholder_type,
            PlaceholderType::Key
        );
        assert_eq!(buf.placeholders[0][0].len, 12);

        // Second placeholder is vector
        assert_eq!(
            buf.placeholders[0][1].placeholder_type,
            PlaceholderType::Vector
        );
        assert_eq!(buf.placeholders[0][1].len, 512);
    }
}
