use std::path::PathBuf;

use chumsky::prelude::*;

use crate::Span;
use crate::edit::{
    EditProgram, FileInsertion, FilePatternMatch, FileRangeSelection, GenericModification,
    Modification, PathDestination, PathSpec, ProgramMode, RangeSet, TextPattern, TextRange,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedPathToken {
    Plain(String),
    Regex(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

pub fn parse_edit_program(input: &str) -> std::result::Result<EditProgram, Vec<ParseError>> {
    program_parser()
        .parse(input)
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|error| ParseError {
                    message: error.to_string(),
                    span: Span::new(error.span().start, error.span().end),
                })
                .collect()
        })
        .and_then(build_program)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedStatement {
    Modification(Modification),
    Apply { span: Span },
    Mode { mode: ProgramMode, span: Span },
}

fn build_program(
    statements: Vec<ParsedStatement>,
) -> std::result::Result<EditProgram, Vec<ParseError>> {
    let mut program = EditProgram::new();
    let mut saw_non_mode_statement = false;
    let mut saw_mode = false;

    for statement in statements {
        match statement {
            ParsedStatement::Modification(modification) => {
                saw_non_mode_statement = true;
                program.push(modification);
            }
            ParsedStatement::Apply { .. } => {
                saw_non_mode_statement = true;
                program.apply();
            }
            ParsedStatement::Mode { mode, span } => {
                if saw_non_mode_statement {
                    return Err(vec![ParseError {
                        message: "`mode` must appear before any operations".to_owned(),
                        span,
                    }]);
                }
                if saw_mode {
                    return Err(vec![ParseError {
                        message: "only one `mode` directive is allowed".to_owned(),
                        span,
                    }]);
                }
                saw_mode = true;
                program = program.with_mode(mode);
            }
        }
    }

    Ok(program)
}

fn program_parser<'src>()
-> impl Parser<'src, &'src str, Vec<ParsedStatement>, extra::Err<Rich<'src, char>>> {
    let newline = choice((just("\r\n").ignored(), just('\n').ignored()));
    let horizontal_ws = one_of(" \t").repeated().ignored();
    let comment = just('#')
        .then(
            any()
                .filter(|character: &char| !matches!(character, '\r' | '\n'))
                .repeated(),
        )
        .ignored();
    let line_end = horizontal_ws
        .then(comment.or_not())
        .ignored()
        .then_ignore(choice((newline, end())));

    let empty_line = choice((
        horizontal_ws
            .ignore_then(comment)
            .then_ignore(choice((newline, end())))
            .ignored(),
        horizontal_ws.then_ignore(newline).ignored(),
    ))
    .to(None::<ParsedStatement>);

    let statement = choice((
        mode_parser().map(Some),
        apply_parser().map(Some),
        line_move_parser().map(Some),
        line_replace_parser().map(Some),
        line_insert_parser().map(Some),
        line_delete_match_parser().map(Some),
        line_delete_parser().map(Some),
        text_replace_parser().map(Some),
        move_files_parser().map(Some),
        remove_files_parser().map(Some),
    ))
    .then_ignore(line_end);

    empty_line
        .or(statement)
        .repeated()
        .collect::<Vec<_>>()
        .map(|items| items.into_iter().flatten().collect())
        .then_ignore(end())
}

fn mode_parser<'src>() -> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>>
{
    just("mode")
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(choice((
            just("snapshot").to(ProgramMode::Snapshot),
            just("incremental").to(ProgramMode::Incremental),
        )))
        .map_with(
            |mode: ProgramMode,
             e: &mut chumsky::input::MapExtra<
                'src,
                '_,
                &'src str,
                extra::Err<Rich<'src, char>>,
            >| ParsedStatement::Mode {
                mode,
                span: Span::new(e.span().start, e.span().end),
            },
        )
}

fn apply_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    just("apply")
        .ignored()
        .map_with(|(), e: &mut chumsky::input::MapExtra<'src, '_, &'src str, extra::Err<Rich<'src, char>>>| ParsedStatement::Apply {
            span: Span::new(e.span().start, e.span().end),
        })
}

fn move_files_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("move"), just("m")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(path_spec_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(path_destination_parser())
        .map_with(|(sources, destination_dir), e| {
            ParsedStatement::Modification(Modification::from(GenericModification::MoveFiles {
                sources,
                destination_dir,
                create_destination_dir: true,
                overwrite: false,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn remove_files_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("remove"), just("r")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(path_spec_parser())
        .map_with(|targets, e| {
            ParsedStatement::Modification(Modification::from(GenericModification::DeleteFiles {
                targets,
                missing_matches_ok: false,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn line_move_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("linemove"), just("lm")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(file_range_selection_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(file_insertion_parser())
        .map_with(|(source, destination), e| {
            ParsedStatement::Modification(Modification::from(GenericModification::MoveRanges {
                source,
                destination,
                create_destination_if_missing: false,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn line_delete_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("linedelete"), just("ld")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(file_range_selection_parser())
        .map_with(|target, e| {
            ParsedStatement::Modification(Modification::from(GenericModification::DeleteRanges {
                target,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn line_delete_match_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("linedeletematch"), just("ldm")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(path_text_target_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(regex_literal_parser())
        .map_with(|(path, pattern), e| {
            ParsedStatement::Modification(Modification::from(
                GenericModification::DeleteLinesMatching {
                    target: FilePatternMatch::new(PathBuf::from(path), pattern)
                        .with_span(Span::new(e.span().start, e.span().end)),
                    span: Some(Span::new(e.span().start, e.span().end)),
                },
            ))
        })
}

fn line_insert_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("lineinsert"), just("li")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(file_insertion_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(string_literal_parser())
        .map_with(|(target, content), e| {
            ParsedStatement::Modification(Modification::from(GenericModification::InsertLines {
                target,
                content,
                create_destination_if_missing: false,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn line_replace_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("linereplace"), just("lr")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(file_range_selection_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(string_literal_parser())
        .map_with(|(target, content), e| {
            ParsedStatement::Modification(Modification::from(GenericModification::ReplaceRanges {
                target,
                content,
                create_destination_if_missing: false,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn text_replace_parser<'src>()
-> impl Parser<'src, &'src str, ParsedStatement, extra::Err<Rich<'src, char>>> {
    choice((just("textreplace"), just("tr")))
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .ignore_then(path_spec_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(text_pattern_parser())
        .then_ignore(one_of(" \t").repeated().at_least(1))
        .then(string_literal_parser())
        .map_with(|((targets, pattern), replacement), e| {
            ParsedStatement::Modification(Modification::from(GenericModification::TextReplace {
                targets,
                pattern,
                replacement,
                span: Some(Span::new(e.span().start, e.span().end)),
            }))
        })
}

fn path_spec_parser<'src>() -> impl Parser<'src, &'src str, PathSpec, extra::Err<Rich<'src, char>>>
{
    choice((
        regex_literal_parser().map(ParsedPathToken::Regex),
        path_operand_parser().map(ParsedPathToken::Plain),
    ))
    .map_with(|token, e| match token {
        ParsedPathToken::Plain(token) => {
            path_spec_from_plain_token(&token, Span::new(e.span().start, e.span().end))
        }
        ParsedPathToken::Regex(pattern) => {
            let (root, pattern) = split_regex_root(&pattern);
            PathSpec::regex(root, pattern).with_span(Span::new(e.span().start, e.span().end))
        }
    })
}

fn path_destination_parser<'src>()
-> impl Parser<'src, &'src str, PathDestination, extra::Err<Rich<'src, char>>> {
    path_operand_parser().map_with(|token, e| {
        PathDestination::directory(PathBuf::from(token))
            .with_span(Span::new(e.span().start, e.span().end))
    })
}

fn file_range_selection_parser<'src>()
-> impl Parser<'src, &'src str, FileRangeSelection, extra::Err<Rich<'src, char>>> {
    path_text_target_parser()
        .then_ignore(just(':'))
        .then(range_set_parser())
        .map_with(|(path, ranges), e| {
            FileRangeSelection::new(PathBuf::from(path), ranges)
                .with_span(Span::new(e.span().start, e.span().end))
        })
}

fn file_insertion_parser<'src>()
-> impl Parser<'src, &'src str, FileInsertion, extra::Err<Rich<'src, char>>> {
    path_text_target_parser()
        .then_ignore(just(':'))
        .then(number_parser())
        .map_with(|(path, offset), e| {
            FileInsertion::new(PathBuf::from(path), offset)
                .with_span(Span::new(e.span().start, e.span().end))
        })
}

fn range_set_parser<'src>() -> impl Parser<'src, &'src str, RangeSet, extra::Err<Rich<'src, char>>>
{
    text_range_parser()
        .separated_by(just(','))
        .at_least(1)
        .collect::<Vec<_>>()
        .map_with(|ranges, e| {
            RangeSet::new(ranges).with_span(Span::new(e.span().start, e.span().end))
        })
}

fn text_range_parser<'src>() -> impl Parser<'src, &'src str, TextRange, extra::Err<Rich<'src, char>>>
{
    number_parser()
        .then_ignore(just('-'))
        .then(number_parser())
        .try_map_with(|(start, end), e| {
            TextRange::new(start, end)
                .map(|range| range.with_span(Span::new(e.span().start, e.span().end)))
                .map_err(|error| Rich::custom(e.span(), error.to_string()))
        })
}

fn number_parser<'src>() -> impl Parser<'src, &'src str, usize, extra::Err<Rich<'src, char>>> {
    text::int(10).try_map(|digits: &str, span| {
        digits
            .parse::<usize>()
            .map_err(|_| Rich::custom(span, format!("invalid integer `{digits}`")))
    })
}

fn regex_literal_parser<'src>() -> impl Parser<'src, &'src str, String, extra::Err<Rich<'src, char>>>
{
    just("r\"")
        .ignore_then(
            choice((just("\\\"").to('"'), any().and_is(just('"').not())))
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('"'))
}

fn text_pattern_parser<'src>()
-> impl Parser<'src, &'src str, TextPattern, extra::Err<Rich<'src, char>>> {
    choice((
        regex_literal_parser().map_with(|pattern, e| {
            TextPattern::regex(pattern).with_span(Span::new(e.span().start, e.span().end))
        }),
        string_literal_parser().map_with(|text, e| {
            TextPattern::literal(text).with_span(Span::new(e.span().start, e.span().end))
        }),
    ))
}

fn string_literal_parser<'src>()
-> impl Parser<'src, &'src str, String, extra::Err<Rich<'src, char>>> {
    just('"')
        .ignore_then(
            choice((
                just('\\').ignore_then(choice((
                    just('n').to('\n'),
                    just('r').to('\r'),
                    just('t').to('\t'),
                    just('"').to('"'),
                    just('\\').to('\\'),
                ))),
                any().filter(|c: &char| !matches!(c, '"' | '\\')),
            ))
            .repeated()
            .collect::<String>(),
        )
        .then_ignore(just('"'))
}

fn path_token_parser<'src>() -> impl Parser<'src, &'src str, String, extra::Err<Rich<'src, char>>> {
    any()
        .filter(|c: &char| !matches!(c, ' ' | '\t' | '\n' | '\r' | '#' | '"'))
        .repeated()
        .at_least(1)
        .collect::<String>()
}

fn path_text_target_parser<'src>()
-> impl Parser<'src, &'src str, String, extra::Err<Rich<'src, char>>> {
    let path_tail = any()
        .filter(|c: &char| !matches!(c, ' ' | '\t' | '\n' | '\r' | '#' | ':' | '"'))
        .repeated()
        .at_least(1)
        .collect::<String>();
    let drive_path = any()
        .filter(char::is_ascii_alphabetic)
        .then_ignore(just(':'))
        .then(path_tail)
        .map(|(drive, tail)| format!("{drive}:{tail}"));

    choice((string_literal_parser(), drive_path, path_tail))
}

fn path_operand_parser<'src>() -> impl Parser<'src, &'src str, String, extra::Err<Rich<'src, char>>>
{
    choice((string_literal_parser(), path_token_parser()))
}

fn path_spec_from_plain_token(token: &str, span: Span) -> PathSpec {
    if looks_like_glob(token) {
        let (root, pattern) = split_glob_root(token);
        return PathSpec::glob(root, pattern).with_span(span);
    }

    if token.ends_with(['/', '\\']) {
        let root = token.trim_end_matches(['/', '\\']);
        let root = if root.is_empty() || (root.ends_with(':') && root.len() + 1 == token.len()) {
            token
        } else {
            root
        };
        return PathSpec::files_in_directory(normalize_empty_path(root)).with_span(span);
    }

    PathSpec::exact_file(PathBuf::from(token)).with_span(span)
}

fn looks_like_glob(token: &str) -> bool {
    token.contains('*') || token.contains('?') || token.contains('[')
}

fn split_glob_root(token: &str) -> (PathBuf, String) {
    let wildcard_index = token
        .char_indices()
        .find_map(|(index, character)| matches!(character, '*' | '?' | '[').then_some(index))
        .unwrap_or(token.len());
    let prefix = &token[..wildcard_index];
    let Some(separator_index) = prefix.rfind(['/', '\\']) else {
        return (PathBuf::from("."), token.to_owned());
    };
    let separator_is_drive_root = separator_index == 2 && token.as_bytes().get(1) == Some(&b':');
    let root = if separator_index == 0 || separator_is_drive_root {
        &token[..=separator_index]
    } else {
        &token[..separator_index]
    };
    let pattern = &token[separator_index + 1..];

    (normalize_empty_path(root), pattern.replace('\\', "/"))
}

fn split_regex_root(pattern: &str) -> (PathBuf, String) {
    if has_top_level_regex_alternation(pattern) {
        return (PathBuf::from("."), pattern.to_owned());
    }

    let mut prefix = String::new();
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.peek().copied() {
        if matches!(
            ch,
            '.' | '+' | '*' | '?' | '[' | '(' | '{' | '^' | '$' | '|'
        ) {
            break;
        }
        if ch == '\\' {
            break;
        }
        prefix.push(ch);
        chars.next();
    }

    let Some(separator_index) = prefix.rfind('/') else {
        return (PathBuf::from("."), pattern.to_owned());
    };

    let root = &prefix[..separator_index];
    let relative_pattern = &pattern[separator_index + 1..];

    (normalize_empty_path(root), relative_pattern.to_owned())
}

fn has_top_level_regex_alternation(pattern: &str) -> bool {
    let mut escaped = false;
    let mut in_class = false;
    let mut group_depth = 0usize;

    for ch in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => group_depth += 1,
            ')' if !in_class => group_depth = group_depth.saturating_sub(1),
            '|' if !in_class && group_depth == 0 => return true,
            _ => {}
        }
    }
    false
}

fn normalize_empty_path(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::edit::{
        GenericModification, Modification, PathDestinationKind, PathSpecKind, ProgramMode,
        TextPattern,
    };

    use super::parse_edit_program;

    #[test]
    fn parses_multiple_operations_and_comments() {
        let source = r#"
mode incremental
# comment
m a/*.rs dst/
apply
lm a.txt:1-3,5-7 b.txt:2 # inline
ld c.txt:0-1,3-4
li d.txt:1 "x\ny\n"
lr e.txt:2-4 "replacement\n"
ldm f.txt r"^use "
tr src/*.rs "foo" "bar"
tr Cargo.toml r"^(name = )\"([^\"]+)\"" "$1\"smartedit\""
r r"a/[a-z]+\.rs"
"#;

        let program = parse_edit_program(source).unwrap();
        assert_eq!(program.mode, ProgramMode::Incremental);
        assert_eq!(program.stages().len(), 2);
        assert_eq!(program.modifications().len(), 9);

        match &program.modifications()[0] {
            Modification::Generic(GenericModification::MoveFiles {
                sources,
                destination_dir,
                ..
            }) => {
                assert!(matches!(sources.kind, PathSpecKind::Glob { .. }));
                assert!(matches!(
                    destination_dir.kind,
                    PathDestinationKind::Directory { ref path } if path == &PathBuf::from("dst/")
                ));
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[1] {
            Modification::Generic(GenericModification::MoveRanges {
                source,
                destination,
                ..
            }) => {
                assert_eq!(source.path, PathBuf::from("a.txt"));
                assert_eq!(source.ranges.ranges().len(), 2);
                assert_eq!(destination.path, PathBuf::from("b.txt"));
                assert_eq!(destination.offset, 2);
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[2] {
            Modification::Generic(GenericModification::DeleteRanges { target, .. }) => {
                assert_eq!(target.path, PathBuf::from("c.txt"));
                assert_eq!(target.ranges.ranges().len(), 2);
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[3] {
            Modification::Generic(GenericModification::InsertLines {
                target, content, ..
            }) => {
                assert_eq!(target.path, PathBuf::from("d.txt"));
                assert_eq!(target.offset, 1);
                assert_eq!(content, "x\ny\n");
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[4] {
            Modification::Generic(GenericModification::ReplaceRanges {
                target, content, ..
            }) => {
                assert_eq!(target.path, PathBuf::from("e.txt"));
                assert_eq!(target.ranges.ranges().len(), 1);
                assert_eq!(content, "replacement\n");
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[5] {
            Modification::Generic(GenericModification::DeleteLinesMatching { target, .. }) => {
                assert_eq!(target.path, PathBuf::from("f.txt"));
                assert_eq!(target.pattern, "^use ");
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[6] {
            Modification::Generic(GenericModification::TextReplace {
                targets,
                pattern,
                replacement,
                ..
            }) => {
                assert!(matches!(targets.kind, PathSpecKind::Glob { .. }));
                assert!(matches!(pattern, TextPattern::Literal { text, .. } if text == "foo"));
                assert_eq!(replacement, "bar");
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[7] {
            Modification::Generic(GenericModification::TextReplace {
                targets,
                pattern,
                replacement,
                ..
            }) => {
                assert!(matches!(
                    targets.kind,
                    PathSpecKind::ExactFile { ref path } if path == &PathBuf::from("Cargo.toml")
                ));
                assert!(matches!(
                    pattern,
                    TextPattern::Regex { pattern, .. } if pattern == r#"^(name = )"([^"]+)""#
                ));
                assert_eq!(replacement, "$1\"smartedit\"");
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        match &program.modifications()[8] {
            Modification::Generic(GenericModification::DeleteFiles { targets, .. }) => {
                assert!(matches!(targets.kind, PathSpecKind::Regex { .. }));
            }
            other => panic!("unexpected modification: {other:?}"),
        }
    }

    #[test]
    fn parses_crlf_and_mixed_line_endings() {
        let source = "# windows\r\nld a.txt:0-1\r\n\r\n# unix next\nli b.txt:0 \"x\\n\"\r\n";

        let program = parse_edit_program(source).unwrap();

        assert_eq!(program.modification_count(), 2);
    }

    #[test]
    fn parses_a_final_comment_without_a_newline() {
        let program = parse_edit_program("ld a.txt:0-1\n# final comment").unwrap();

        assert_eq!(program.modification_count(), 1);
    }

    #[test]
    fn regex_source_prefix_is_removed_from_the_root_relative_pattern() {
        let program = parse_edit_program(r#"r r"src/generated/[a-z_]+\.rs""#).unwrap();

        match &program.modifications()[0] {
            Modification::Generic(GenericModification::DeleteFiles { targets, .. }) => {
                assert!(matches!(
                    &targets.kind,
                    PathSpecKind::Regex { root, pattern }
                        if root == &PathBuf::from("src/generated")
                            && pattern == r#"[a-z_]+\.rs"#
                ));
            }
            other => panic!("unexpected modification: {other:?}"),
        }
    }

    #[test]
    fn top_level_regex_alternation_keeps_mixed_prefixes() {
        let program = parse_edit_program(r#"r r"src/.*\.rs|README\.md""#).unwrap();
        let Modification::Generic(GenericModification::DeleteFiles { targets, .. }) =
            &program.stages()[0].modifications()[0]
        else {
            panic!("expected remove operation");
        };
        assert_eq!(
            targets.kind,
            PathSpecKind::Regex {
                root: PathBuf::from("."),
                pattern: r"src/.*\.rs|README\.md".to_owned(),
            }
        );
    }

    #[test]
    fn directory_and_glob_roots_are_not_reduced_to_drive_relative_paths() {
        let directory = parse_edit_program(r"r C:\").unwrap();
        let Modification::Generic(GenericModification::DeleteFiles { targets, .. }) =
            &directory.stages()[0].modifications()[0]
        else {
            panic!("expected remove operation");
        };
        assert_eq!(
            targets.kind,
            PathSpecKind::FilesInDirectory {
                root: PathBuf::from(r"C:\"),
                recursive: true,
            }
        );

        let glob = parse_edit_program(r"r C:\*.rs").unwrap();
        let Modification::Generic(GenericModification::DeleteFiles { targets, .. }) =
            &glob.stages()[0].modifications()[0]
        else {
            panic!("expected remove operation");
        };
        assert_eq!(
            targets.kind,
            PathSpecKind::Glob {
                root: PathBuf::from(r"C:\"),
                pattern: "*.rs".to_owned(),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_root_directory_and_glob_remain_absolute() {
        let directory = parse_edit_program("r /").unwrap();
        let Modification::Generic(GenericModification::DeleteFiles { targets, .. }) =
            &directory.stages()[0].modifications()[0]
        else {
            panic!("expected remove operation");
        };
        assert_eq!(
            targets.kind,
            PathSpecKind::FilesInDirectory {
                root: PathBuf::from("/"),
                recursive: true,
            }
        );

        let glob = parse_edit_program("r /*.rs").unwrap();
        let Modification::Generic(GenericModification::DeleteFiles { targets, .. }) =
            &glob.stages()[0].modifications()[0]
        else {
            panic!("expected remove operation");
        };
        assert_eq!(
            targets.kind,
            PathSpecKind::Glob {
                root: PathBuf::from("/"),
                pattern: "*.rs".to_owned(),
            }
        );
    }

    #[test]
    fn anchored_regex_source_keeps_its_full_pattern() {
        let program = parse_edit_program(r#"r r"^src/[a-z_]+\.rs$""#).unwrap();

        match &program.modifications()[0] {
            Modification::Generic(GenericModification::DeleteFiles { targets, .. }) => {
                assert!(matches!(
                    &targets.kind,
                    PathSpecKind::Regex { root, pattern }
                        if root == &PathBuf::from(".") && pattern == r#"^src/[a-z_]+\.rs$"#
                ));
            }
            other => panic!("unexpected modification: {other:?}"),
        }
    }

    #[test]
    fn parses_windows_drive_letter_line_targets() {
        let program = parse_edit_program(
            r#"ld C:\work\src\main.rs:0-2
li D:/output.txt:1 "line\n""#,
        )
        .unwrap();

        match &program.modifications()[0] {
            Modification::Generic(GenericModification::DeleteRanges { target, .. }) => {
                assert_eq!(target.path, PathBuf::from(r"C:\work\src\main.rs"));
            }
            other => panic!("unexpected modification: {other:?}"),
        }
        match &program.modifications()[1] {
            Modification::Generic(GenericModification::InsertLines { target, .. }) => {
                assert_eq!(target.path, PathBuf::from("D:/output.txt"));
            }
            other => panic!("unexpected modification: {other:?}"),
        }
    }

    #[test]
    fn parses_native_separator_glob_and_directory_roots() {
        let program = parse_edit_program(
            r#"r C:\work\src\*.rs
r C:\work\cache\"#,
        )
        .unwrap();

        assert!(matches!(
            &program.modifications()[0],
            Modification::Generic(GenericModification::DeleteFiles { targets, .. })
                if matches!(
                    &targets.kind,
                    PathSpecKind::Glob { root, pattern }
                        if root == &PathBuf::from(r"C:\work\src") && pattern == "*.rs"
                )
        ));
        assert!(matches!(
            &program.modifications()[1],
            Modification::Generic(GenericModification::DeleteFiles { targets, .. })
                if matches!(
                    &targets.kind,
                    PathSpecKind::FilesInDirectory { root, .. }
                        if root == &PathBuf::from(r"C:\work\cache")
                )
        ));
    }

    #[test]
    fn parses_quoted_path_specs_and_destinations() {
        let program = parse_edit_program(
            r#"m "source dir/*.rs" "destination #1/"
r "recursive #1/"
tr "exact #1.txt" "old" "new""#,
        )
        .unwrap();

        assert!(matches!(
            &program.modifications()[0],
            Modification::Generic(GenericModification::MoveFiles {
                sources,
                destination_dir,
                ..
            }) if matches!(
                &sources.kind,
                PathSpecKind::Glob { root, pattern }
                    if root == &PathBuf::from("source dir") && pattern == "*.rs"
            ) && matches!(
                &destination_dir.kind,
                PathDestinationKind::Directory { path }
                    if path == &PathBuf::from("destination #1/")
            )
        ));
        assert!(matches!(
            &program.modifications()[1],
            Modification::Generic(GenericModification::DeleteFiles { targets, .. })
                if matches!(
                    &targets.kind,
                    PathSpecKind::FilesInDirectory { root, .. }
                        if root == &PathBuf::from("recursive #1")
                )
        ));
        assert!(matches!(
            &program.modifications()[2],
            Modification::Generic(GenericModification::TextReplace { targets, .. })
                if matches!(
                    &targets.kind,
                    PathSpecKind::ExactFile { path }
                        if path == &PathBuf::from("exact #1.txt")
                )
        ));
    }

    #[test]
    fn parses_quoted_paths_for_every_text_target() {
        let program = parse_edit_program(
            r#"lm "C:\\Program Files\\source #1.txt":0-2 "D:\\Output\\target #1.txt":1
ld "delete #1.txt":2-3
li "insert #1.txt":4 "line\n"
lr "replace #1.txt":5-6 "line\n"
ldm "match #1.txt" r"^drop$""#,
        )
        .unwrap();

        match &program.modifications()[0] {
            Modification::Generic(GenericModification::MoveRanges {
                source,
                destination,
                ..
            }) => {
                assert_eq!(
                    source.path,
                    PathBuf::from(r"C:\Program Files\source #1.txt")
                );
                assert_eq!(destination.path, PathBuf::from(r"D:\Output\target #1.txt"));
            }
            other => panic!("unexpected modification: {other:?}"),
        }

        for (index, expected) in [
            (1, "delete #1.txt"),
            (2, "insert #1.txt"),
            (3, "replace #1.txt"),
        ] {
            let actual = match &program.modifications()[index] {
                Modification::Generic(GenericModification::DeleteRanges { target, .. }) => {
                    &target.path
                }
                Modification::Generic(GenericModification::InsertLines { target, .. }) => {
                    &target.path
                }
                Modification::Generic(GenericModification::ReplaceRanges { target, .. }) => {
                    &target.path
                }
                other => panic!("unexpected modification: {other:?}"),
            };
            assert_eq!(actual, &PathBuf::from(expected));
        }

        assert!(matches!(
            &program.modifications()[4],
            Modification::Generic(GenericModification::DeleteLinesMatching { target, .. })
                if target.path.as_path() == Path::new("match #1.txt")
                    && target.pattern == "^drop$"
        ));
    }

    #[test]
    fn quoted_paths_use_standard_string_escapes_without_changing_unquoted_paths_or_regexes() {
        let program = parse_edit_program(
            r#"r plain/path.txt
r C:\work\src\*.rs
r r"src/[a-z_]+\.rs"
ld "quote\" and tab\t.txt":0-1"#,
        )
        .unwrap();

        assert!(matches!(
            &program.modifications()[0],
            Modification::Generic(GenericModification::DeleteFiles { targets, .. })
                if matches!(
                    &targets.kind,
                    PathSpecKind::ExactFile { path }
                        if path == &PathBuf::from("plain/path.txt")
                )
        ));
        assert!(matches!(
            &program.modifications()[1],
            Modification::Generic(GenericModification::DeleteFiles { targets, .. })
                if matches!(
                    &targets.kind,
                    PathSpecKind::Glob { root, pattern }
                        if root == &PathBuf::from(r"C:\work\src") && pattern == "*.rs"
                )
        ));
        assert!(matches!(
            &program.modifications()[2],
            Modification::Generic(GenericModification::DeleteFiles { targets, .. })
                if matches!(&targets.kind, PathSpecKind::Regex { .. })
        ));
        assert!(matches!(
            &program.modifications()[3],
            Modification::Generic(GenericModification::DeleteRanges { target, .. })
                if target.path.as_path() == Path::new("quote\" and tab\t.txt")
        ));
    }
}
