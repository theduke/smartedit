use std::path::PathBuf;

use super::{
    FileInsertion, FilePatternMatch, FileRangeSelection, PathDestination, PathSpec, TextPattern,
};
use crate::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modification {
    Generic(GenericModification),
    Language(LanguageModification),
}

impl From<GenericModification> for Modification {
    fn from(value: GenericModification) -> Self {
        Self::Generic(value)
    }
}

impl From<LanguageModification> for Modification {
    fn from(value: LanguageModification) -> Self {
        Self::Language(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenericModification {
    CreateDirectory {
        path: PathBuf,
        span: Option<Span>,
    },
    CreateFile {
        path: PathBuf,
        content: String,
        overwrite: bool,
        span: Option<Span>,
    },
    DeleteFiles {
        targets: PathSpec,
        missing_matches_ok: bool,
        span: Option<Span>,
    },
    DeleteRanges {
        target: FileRangeSelection,
        span: Option<Span>,
    },
    DeleteLinesMatching {
        target: FilePatternMatch,
        span: Option<Span>,
    },
    MoveFiles {
        sources: PathSpec,
        destination_dir: PathDestination,
        create_destination_dir: bool,
        overwrite: bool,
        span: Option<Span>,
    },
    MoveRanges {
        source: FileRangeSelection,
        destination: FileInsertion,
        create_destination_if_missing: bool,
        span: Option<Span>,
    },
    InsertLines {
        target: FileInsertion,
        content: String,
        create_destination_if_missing: bool,
        span: Option<Span>,
    },
    ReplaceRanges {
        target: FileRangeSelection,
        content: String,
        create_destination_if_missing: bool,
        span: Option<Span>,
    },
    TextReplace {
        targets: PathSpec,
        pattern: TextPattern,
        replacement: String,
        span: Option<Span>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageModification {}
