use std::path::PathBuf;

use crate::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSpec {
    pub kind: PathSpecKind,
    pub span: Option<Span>,
}

impl PathSpec {
    pub fn new(kind: PathSpecKind) -> Self {
        Self { kind, span: None }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn exact_file(path: impl Into<PathBuf>) -> Self {
        Self::new(PathSpecKind::ExactFile { path: path.into() })
    }

    pub fn files_in_directory(root: impl Into<PathBuf>) -> Self {
        Self::new(PathSpecKind::FilesInDirectory {
            root: root.into(),
            recursive: true,
        })
    }

    pub fn files_in_directory_with_depth(root: impl Into<PathBuf>, recursive: bool) -> Self {
        Self::new(PathSpecKind::FilesInDirectory {
            root: root.into(),
            recursive,
        })
    }

    pub fn glob(root: impl Into<PathBuf>, pattern: impl Into<String>) -> Self {
        Self::new(PathSpecKind::Glob {
            root: root.into(),
            pattern: pattern.into(),
        })
    }

    pub fn regex(root: impl Into<PathBuf>, pattern: impl Into<String>) -> Self {
        Self::new(PathSpecKind::Regex {
            root: root.into(),
            pattern: pattern.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSpecKind {
    ExactFile { path: PathBuf },
    FilesInDirectory { root: PathBuf, recursive: bool },
    Glob { root: PathBuf, pattern: String },
    Regex { root: PathBuf, pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathDestination {
    pub kind: PathDestinationKind,
    pub span: Option<Span>,
}

impl PathDestination {
    pub fn new(kind: PathDestinationKind) -> Self {
        Self { kind, span: None }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn directory(path: impl Into<PathBuf>) -> Self {
        Self::new(PathDestinationKind::Directory { path: path.into() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathDestinationKind {
    Directory { path: PathBuf },
}
