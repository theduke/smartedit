use std::fmt;
use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

pub type Result<T> = std::result::Result<T, SmartEditError>;

#[derive(Debug)]
pub enum SmartEditError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    FileAlreadyExists {
        path: PathBuf,
    },
    NoFilesMatched {
        description: String,
    },
    InvalidGlobPattern {
        pattern: String,
        message: String,
    },
    InvalidRegexPattern {
        pattern: String,
        message: String,
    },
    ConflictingActionTargets {
        path: PathBuf,
        first_modification: usize,
        second_modification: usize,
    },
    MissingFile {
        path: PathBuf,
    },
    ExpectedFileButFoundDirectory {
        path: PathBuf,
    },
    ExpectedDirectoryButFoundFile {
        path: PathBuf,
    },
    InvalidRange {
        start: usize,
        end: usize,
    },
    EmptyTextPattern,
    InvalidUtf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
    UnsupportedAstLanguage {
        path: PathBuf,
    },
    AstParseSetupFailed {
        language: &'static str,
        message: String,
    },
    AstParseFailed {
        language: &'static str,
        message: String,
    },
    InvalidAstSelectorPattern {
        pattern: String,
        message: String,
    },
    NoAstItemsMatched {
        selector: String,
    },
    RangeOutOfBounds {
        path: PathBuf,
        start: usize,
        end: usize,
        len: usize,
    },
    RangeNotOnCharBoundary {
        path: PathBuf,
        offset: usize,
    },
    RangesNotSortedOrDisjoint {
        path: PathBuf,
        previous_end: usize,
        next_start: usize,
    },
    InvalidInsertionOffset {
        path: PathBuf,
        offset: usize,
        len: usize,
    },
    InsertionPointInsideMovedRange {
        path: PathBuf,
        offset: usize,
        range_start: usize,
        range_end: usize,
    },
    InsertionPointInsideDeletedRange {
        path: PathBuf,
        offset: usize,
        range_start: usize,
        range_end: usize,
    },
    UnsupportedLanguageModification,
}

impl fmt::Display for SmartEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmartEditError::Io {
                operation,
                path,
                source,
            } => {
                write!(f, "{operation} failed for {}: {source}", path.display())
            }
            SmartEditError::FileAlreadyExists { path } => {
                write!(f, "file already exists: {}", path.display())
            }
            SmartEditError::NoFilesMatched { description } => {
                write!(f, "no files matched {description}")
            }
            SmartEditError::InvalidGlobPattern { pattern, message } => {
                write!(f, "invalid glob pattern `{pattern}`: {message}")
            }
            SmartEditError::InvalidRegexPattern { pattern, message } => {
                write!(f, "invalid regex pattern `{pattern}`: {message}")
            }
            SmartEditError::ConflictingActionTargets {
                path,
                first_modification,
                second_modification,
            } => {
                write!(
                    f,
                    "modification {second_modification} conflicts with modification {first_modification} on {}",
                    path.display()
                )
            }
            SmartEditError::MissingFile { path } => {
                write!(f, "file does not exist: {}", path.display())
            }
            SmartEditError::ExpectedFileButFoundDirectory { path } => {
                write!(
                    f,
                    "expected a file but found a directory: {}",
                    path.display()
                )
            }
            SmartEditError::ExpectedDirectoryButFoundFile { path } => {
                write!(
                    f,
                    "expected a directory but found a file: {}",
                    path.display()
                )
            }
            SmartEditError::InvalidRange { start, end } => {
                write!(f, "invalid range [{start}, {end})")
            }
            SmartEditError::EmptyTextPattern => {
                write!(f, "text match pattern must not be empty")
            }
            SmartEditError::InvalidUtf8 { path, .. } => {
                write!(f, "file is not valid UTF-8: {}", path.display())
            }
            SmartEditError::UnsupportedAstLanguage { path } => {
                write!(
                    f,
                    "cannot infer a supported AST language for {}",
                    path.display()
                )
            }
            SmartEditError::AstParseSetupFailed { language, message } => {
                write!(f, "failed to initialize {language} parser: {message}")
            }
            SmartEditError::AstParseFailed { language, message } => {
                write!(f, "failed to parse {language} source: {message}")
            }
            SmartEditError::InvalidAstSelectorPattern { pattern, message } => {
                write!(f, "invalid AST selector pattern `{pattern}`: {message}")
            }
            SmartEditError::NoAstItemsMatched { selector } => {
                write!(f, "no AST items matched selector {selector}")
            }
            SmartEditError::RangeOutOfBounds {
                path,
                start,
                end,
                len,
            } => {
                write!(
                    f,
                    "range [{start}, {end}) is out of bounds for {} (len {len})",
                    path.display()
                )
            }
            SmartEditError::RangeNotOnCharBoundary { path, offset } => {
                write!(
                    f,
                    "offset {offset} is not on a UTF-8 character boundary in {}",
                    path.display()
                )
            }
            SmartEditError::RangesNotSortedOrDisjoint {
                path,
                previous_end,
                next_start,
            } => {
                write!(
                    f,
                    "ranges for {} must be sorted and non-overlapping (previous end {previous_end}, next start {next_start})",
                    path.display()
                )
            }
            SmartEditError::InvalidInsertionOffset { path, offset, len } => {
                write!(
                    f,
                    "insertion offset {offset} is out of bounds for {} (len {len})",
                    path.display()
                )
            }
            SmartEditError::InsertionPointInsideMovedRange {
                path,
                offset,
                range_start,
                range_end,
            } => {
                write!(
                    f,
                    "insertion offset {offset} falls inside moved range [{range_start}, {range_end}) in {}",
                    path.display()
                )
            }
            SmartEditError::InsertionPointInsideDeletedRange {
                path,
                offset,
                range_start,
                range_end,
            } => {
                write!(
                    f,
                    "insertion offset {offset} falls inside deleted range [{range_start}, {range_end}) in {}",
                    path.display()
                )
            }
            SmartEditError::UnsupportedLanguageModification => {
                write!(f, "language-aware modifications are not implemented yet")
            }
        }
    }
}

impl std::error::Error for SmartEditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SmartEditError::Io { source, .. } => Some(source),
            SmartEditError::InvalidUtf8 { source, .. } => Some(source),
            _ => None,
        }
    }
}
