mod operations;
mod path;
mod program;
mod range;
mod target;

pub use operations::{GenericModification, LanguageModification, Modification};
pub use path::{PathDestination, PathDestinationKind, PathSpec, PathSpecKind};
pub use program::{EditProgram, EditStage, ProgramMode};
pub use range::{RangeSet, TextRange};
pub(crate) use range::{resolve_insertion_offset, resolve_matching_line_ranges};
pub use target::{FileInsertion, FilePatternMatch, FileRangeSelection, TextPattern};
