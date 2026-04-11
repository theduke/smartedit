use std::path::Path;

use crate::Span;
use crate::error::{Result, SmartEditError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
    pub span: Option<Span>,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Result<Self> {
        if start > end {
            return Err(SmartEditError::InvalidRange { start, end });
        }

        Ok(Self {
            start,
            end,
            span: None,
        })
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RangeSet {
    ranges: Vec<TextRange>,
    pub span: Option<Span>,
}

impl RangeSet {
    pub fn new(ranges: Vec<TextRange>) -> Self {
        Self { ranges, span: None }
    }

    pub fn single(range: TextRange) -> Self {
        Self::new(vec![range])
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn ranges(&self) -> &[TextRange] {
        &self.ranges
    }

    pub(crate) fn extract_from(&self, path: &Path, content: &str) -> Result<String> {
        let ranges = self.resolve_against(path, content)?;
        let capacity = ranges.iter().map(TextRange::len).sum();
        let mut extracted = String::with_capacity(capacity);

        for range in &ranges {
            extracted.push_str(&content[range.start..range.end]);
        }

        Ok(extracted)
    }

    pub(crate) fn resolve_against(&self, path: &Path, content: &str) -> Result<Vec<TextRange>> {
        let line_starts = line_start_offsets(content);
        let line_count = line_starts.len().saturating_sub(1);
        let mut previous_end = 0;
        let mut has_previous = false;
        let mut resolved = Vec::with_capacity(self.ranges.len());

        for range in &self.ranges {
            if range.start > range.end {
                return Err(SmartEditError::InvalidRange {
                    start: range.start,
                    end: range.end,
                });
            }

            if range.end > line_count {
                return Err(SmartEditError::RangeOutOfBounds {
                    path: path.to_path_buf(),
                    start: range.start,
                    end: range.end,
                    len: line_count,
                });
            }

            if has_previous && range.start < previous_end {
                return Err(SmartEditError::RangesNotSortedOrDisjoint {
                    path: path.to_path_buf(),
                    previous_end,
                    next_start: range.start,
                });
            }

            previous_end = range.end;
            has_previous = true;
            resolved.push(TextRange {
                start: line_starts[range.start],
                end: line_starts[range.end],
                span: range.span,
            });
        }

        Ok(resolved)
    }
}

pub(crate) fn resolve_insertion_offset(path: &Path, content: &str, offset: usize) -> Result<usize> {
    let line_starts = line_start_offsets(content);
    let line_count = line_starts.len().saturating_sub(1);

    if offset > line_count {
        return Err(SmartEditError::InvalidInsertionOffset {
            path: path.to_path_buf(),
            offset,
            len: line_count,
        });
    }

    Ok(line_starts[offset])
}

pub(crate) fn resolve_matching_line_ranges<F>(content: &str, mut predicate: F) -> Vec<TextRange>
where
    F: FnMut(&str) -> bool,
{
    line_byte_ranges(content)
        .into_iter()
        .filter(|range| predicate(&content[range.start..range.end]))
        .collect()
}

fn line_start_offsets(content: &str) -> Vec<usize> {
    if content.is_empty() {
        return vec![0];
    }

    let mut starts = vec![0];
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' && index + 1 < content.len() {
            starts.push(index + 1);
        }
    }
    starts.push(content.len());
    starts
}

fn line_byte_ranges(content: &str) -> Vec<TextRange> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut line_start = 0usize;
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push(TextRange {
                start: line_start,
                end: index + 1,
                span: None,
            });
            line_start = index + 1;
        }
    }

    if line_start < content.len() {
        ranges.push(TextRange {
            start: line_start,
            end: content.len(),
            span: None,
        });
    }

    ranges
}
