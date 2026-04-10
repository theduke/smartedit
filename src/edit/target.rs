use std::path::PathBuf;

use super::RangeSet;
use crate::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRangeSelection {
    pub path: PathBuf,
    pub ranges: RangeSet,
    pub span: Option<Span>,
}

impl FileRangeSelection {
    pub fn new(path: impl Into<PathBuf>, ranges: RangeSet) -> Self {
        Self {
            path: path.into(),
            ranges,
            span: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInsertion {
    pub path: PathBuf,
    pub offset: usize,
    pub span: Option<Span>,
}

impl FileInsertion {
    pub fn new(path: impl Into<PathBuf>, offset: usize) -> Self {
        Self {
            path: path.into(),
            offset,
            span: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatternMatch {
    pub path: PathBuf,
    pub pattern: String,
    pub span: Option<Span>,
}

impl FilePatternMatch {
    pub fn new(path: impl Into<PathBuf>, pattern: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            pattern: pattern.into(),
            span: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextPattern {
    Literal { text: String, span: Option<Span> },
    Regex { pattern: String, span: Option<Span> },
}

impl TextPattern {
    pub fn literal(text: impl Into<String>) -> Self {
        Self::Literal {
            text: text.into(),
            span: None,
        }
    }

    pub fn regex(pattern: impl Into<String>) -> Self {
        Self::Regex {
            pattern: pattern.into(),
            span: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        match &mut self {
            Self::Literal { span: target, .. } | Self::Regex { span: target, .. } => {
                *target = Some(span);
            }
        }
        self
    }
}
