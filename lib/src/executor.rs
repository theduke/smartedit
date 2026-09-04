use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use globset::Glob;
use regex::Regex;

use crate::edit::{
    EditProgram, GenericModification, Modification, PathDestinationKind, PathSpec, PathSpecKind,
    ProgramMode, RangeSet, TextPattern, resolve_insertion_offset, resolve_matching_line_ranges,
};
use crate::error::{Result, SmartEditError};
use crate::fs::{FileIdentity, FileSystem, OsFileSystem};
use crate::plan::{EvaluationPlan, ExecutionOptions, ModificationPlan, PlannedAction};

#[derive(Debug, Default, Clone, Copy)]
pub struct Executor<F = OsFileSystem> {
    fs: F,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedTargetKind {
    Directory,
    File,
}

#[derive(Debug, Clone)]
struct PlannedTarget {
    kind: PlannedTargetKind,
    modification_index: usize,
    path: PathBuf,
    follows_final_symlink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFileMatch {
    path: PathBuf,
    relative_path: PathBuf,
}

#[derive(Debug, Clone)]
struct PendingTextInsertion {
    offset: usize,
    text: String,
    order: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingDeletionKind {
    Delete,
    Move,
    Replace,
}

#[derive(Debug, Clone)]
struct PendingTextDeletion {
    range: crate::edit::TextRange,
    kind: PendingDeletionKind,
    modification_index: usize,
}

#[derive(Debug, Clone)]
struct PendingTextFileUpdate {
    path: PathBuf,
    original_existed: bool,
    expected_identity: Option<FileIdentity>,
    original_content: String,
    deletions: Vec<PendingTextDeletion>,
    insertions: Vec<PendingTextInsertion>,
    first_modification_index: usize,
}

#[derive(Debug, Default)]
struct PendingTextUpdates {
    files: BTreeMap<PathBuf, PendingTextFileUpdate>,
}

#[derive(Debug, Clone, Copy)]
struct TextInsertionTarget<'a> {
    path: &'a Path,
    offset: usize,
    create_if_missing: bool,
}

#[derive(Debug, Clone)]
enum SnapshotEntry {
    File(Vec<u8>),
    MovedFile(PathBuf),
    Directory,
    Missing,
}

#[derive(Debug, Default, Clone)]
struct SnapshotState {
    entries: BTreeMap<PathBuf, SnapshotEntry>,
    content_objects: Vec<(FileIdentity, SnapshotEntry)>,
}

/// Resolves a filesystem-entry identity using the host cwd and existing parent metadata.
///
/// This intentionally lives outside `FileSystem`: custom implementations still use host path
/// identity for snapshot coalescing and conflict checks.
fn entry_path_identity(path: &Path) -> PathBuf {
    let rooted = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(current_dir) = std::env::current_dir() {
        current_dir.join(path)
    } else {
        path.to_path_buf()
    };

    let Some(file_name) = rooted.file_name() else {
        return std::fs::canonicalize(&rooted).unwrap_or_else(|_| lexical_path(&rooted));
    };
    let mut existing_ancestor = rooted
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let mut remainder = vec![file_name.to_os_string()];

    loop {
        if let Ok(canonical) = std::fs::canonicalize(&existing_ancestor) {
            let resolved = remainder
                .iter()
                .rev()
                .fold(canonical, |path, component| path.join(component));
            return lexical_path(&resolved);
        }

        let Some(component) = existing_ancestor.components().next_back() else {
            return lexical_path(&rooted);
        };
        remainder.push(component.as_os_str().to_os_string());
        if !existing_ancestor.pop() {
            return lexical_path(&rooted);
        }
    }
}

fn content_path_identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| entry_path_identity(path))
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(std::path::Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

impl SnapshotState {
    fn apply_action(&mut self, action: &PlannedAction) {
        match action {
            PlannedAction::CreateDirectory { path } => {
                self.ensure_parent_directories(path);
                self.entries
                    .insert(entry_path_identity(path), SnapshotEntry::Directory);
            }
            PlannedAction::WriteFile {
                path,
                bytes,
                expected_identity,
                ..
            } => {
                self.ensure_parent_directories(path);
                let path_identity = content_path_identity(path);
                self.entries
                    .insert(path_identity.clone(), SnapshotEntry::File(bytes.clone()));
                if let Some(expected_identity) = expected_identity {
                    let content = SnapshotEntry::File(bytes.clone());
                    if let Some((_, existing_content)) = self
                        .content_objects
                        .iter_mut()
                        .find(|(identity, _)| identity == expected_identity)
                    {
                        *existing_content = content;
                    } else {
                        self.content_objects
                            .push((expected_identity.clone(), content));
                    }
                }
            }
            PlannedAction::DeleteFile { path, .. } => {
                self.entries
                    .insert(entry_path_identity(path), SnapshotEntry::Missing);
            }
            PlannedAction::MoveFile {
                source,
                destination,
                ..
            } => {
                self.ensure_parent_directories(destination);
                let source_identity = entry_path_identity(source);
                let entry = self
                    .entries
                    .get(&source_identity)
                    .cloned()
                    .unwrap_or_else(|| SnapshotEntry::MovedFile(source.clone()));
                self.entries.insert(entry_path_identity(destination), entry);
                self.entries.insert(source_identity, SnapshotEntry::Missing);
            }
        }
    }

    fn ensure_parent_directories(&mut self, path: &Path) {
        let mut current = path.parent();
        while let Some(parent) = current {
            if parent.as_os_str().is_empty() {
                break;
            }
            self.entries
                .entry(entry_path_identity(parent))
                .or_insert(SnapshotEntry::Directory);
            current = parent.parent();
        }
    }

    fn get(&self, path: &Path) -> Option<&SnapshotEntry> {
        let entry_identity = entry_path_identity(path);
        self.entries.get(&entry_identity).or_else(|| {
            let content_identity = content_path_identity(path);
            (content_identity != entry_identity)
                .then(|| self.entries.get(&content_identity))
                .flatten()
        })
    }

    fn get_content_object(&self, identity: &FileIdentity) -> Option<&SnapshotEntry> {
        self.content_objects
            .iter()
            .find(|(candidate, _)| candidate == identity)
            .map(|(_, entry)| entry)
    }
}

impl Executor<OsFileSystem> {
    pub fn new() -> Self {
        Self { fs: OsFileSystem }
    }
}

impl<F> Executor<F>
where
    F: FileSystem,
{
    pub fn with_file_system(fs: F) -> Self {
        Self { fs }
    }

    pub fn evaluate(&self, program: &EditProgram) -> Result<EvaluationPlan> {
        let mut modification_actions = vec![Vec::new(); program.modification_count()];
        let mut snapshot = SnapshotState::default();
        let mut modification_index = 0usize;

        match program.mode {
            ProgramMode::Snapshot => {
                for stage in program.stages() {
                    modification_index = self.evaluate_stage(
                        stage.modifications(),
                        modification_index,
                        &mut snapshot,
                        &mut modification_actions,
                    )?;
                }
            }
            ProgramMode::Incremental => {
                for stage in program.stages() {
                    for modification in stage.modifications() {
                        modification_index = self.evaluate_stage(
                            std::slice::from_ref(modification),
                            modification_index,
                            &mut snapshot,
                            &mut modification_actions,
                        )?;
                    }
                }
            }
        }

        Ok(EvaluationPlan::new(
            modification_actions
                .into_iter()
                .enumerate()
                .map(|(index, actions)| ModificationPlan::new(index, actions))
                .collect(),
        ))
    }

    pub fn execute(&self, program: &EditProgram) -> Result<()> {
        self.run(program, ExecutionOptions::default()).map(|_| ())
    }

    pub fn run(&self, program: &EditProgram, options: ExecutionOptions) -> Result<EvaluationPlan> {
        let plan = self.evaluate(program)?;
        if options.dry_run {
            return Ok(plan);
        }

        self.apply_plan(&plan)?;

        Ok(plan)
    }

    fn evaluate_stage(
        &self,
        modifications: &[Modification],
        base_modification_index: usize,
        snapshot: &mut SnapshotState,
        modification_actions: &mut [Vec<PlannedAction>],
    ) -> Result<usize> {
        let mut pending_text_updates = PendingTextUpdates::default();

        for (offset, modification) in modifications.iter().enumerate() {
            let modification_index = base_modification_index + offset;
            match modification {
                Modification::Generic(GenericModification::DeleteRanges { target, .. }) => {
                    self.plan_delete_ranges_into(
                        modification_index,
                        target,
                        &mut pending_text_updates,
                        snapshot,
                    )?;
                }
                Modification::Generic(GenericModification::DeleteLinesMatching {
                    target, ..
                }) => {
                    self.plan_delete_lines_matching_into(
                        modification_index,
                        target,
                        &mut pending_text_updates,
                        snapshot,
                    )?;
                }
                Modification::Generic(GenericModification::MoveRanges {
                    source,
                    destination,
                    create_destination_if_missing,
                    ..
                }) => {
                    let destination = TextInsertionTarget {
                        path: destination.path.as_path(),
                        offset: destination.offset,
                        create_if_missing: *create_destination_if_missing,
                    };
                    self.plan_move_ranges_into(
                        modification_index,
                        source.path.as_path(),
                        &source.ranges,
                        destination,
                        &mut pending_text_updates,
                        snapshot,
                    )?;
                }
                Modification::Generic(GenericModification::InsertLines {
                    target,
                    content,
                    create_destination_if_missing,
                    ..
                }) => {
                    let destination = TextInsertionTarget {
                        path: target.path.as_path(),
                        offset: target.offset,
                        create_if_missing: *create_destination_if_missing,
                    };
                    self.plan_insert_lines_into(
                        modification_index,
                        destination,
                        content,
                        &mut pending_text_updates,
                        snapshot,
                    )?;
                }
                Modification::Generic(GenericModification::ReplaceRanges {
                    target,
                    content,
                    create_destination_if_missing,
                    ..
                }) => {
                    self.plan_replace_ranges_into(
                        modification_index,
                        target,
                        content,
                        *create_destination_if_missing,
                        &mut pending_text_updates,
                        snapshot,
                    )?;
                }
                Modification::Generic(GenericModification::TextReplace {
                    targets,
                    pattern,
                    replacement,
                    ..
                }) => {
                    self.plan_text_replace_into(
                        modification_index,
                        targets,
                        pattern,
                        replacement,
                        &mut pending_text_updates,
                        snapshot,
                    )?;
                }
                _ => {
                    modification_actions[modification_index] =
                        self.evaluate_modification(modification_index, modification, snapshot)?;
                }
            }
        }

        for (modification_index, mut actions) in
            self.finalize_text_updates(pending_text_updates, snapshot)?
        {
            modification_actions[modification_index].append(&mut actions);
        }

        let mut targets = BTreeMap::new();
        for (offset, actions) in modifications.iter().enumerate().map(|(offset, _)| {
            (
                offset,
                &modification_actions[base_modification_index + offset],
            )
        }) {
            self.register_targets(base_modification_index + offset, actions, &mut targets)?;
        }

        for offset in 0..modifications.len() {
            for action in &modification_actions[base_modification_index + offset] {
                snapshot.apply_action(action);
            }
        }

        Ok(base_modification_index + modifications.len())
    }

    fn evaluate_modification(
        &self,
        modification_index: usize,
        modification: &Modification,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        match modification {
            Modification::Generic(modification) => {
                self.evaluate_generic(modification_index, modification, snapshot)
            }
            Modification::Language(_) => Err(SmartEditError::UnsupportedLanguageModification),
        }
    }

    fn evaluate_generic(
        &self,
        _modification_index: usize,
        modification: &GenericModification,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        match modification {
            GenericModification::CreateDirectory { path, .. } => {
                self.plan_create_directory(path, snapshot)
            }
            GenericModification::CreateFile {
                path,
                content,
                overwrite,
                ..
            } => self.plan_create_file(path, content, *overwrite, snapshot),
            GenericModification::DeleteFiles {
                targets,
                missing_matches_ok,
                ..
            } => self.plan_delete_files(targets, *missing_matches_ok, snapshot),
            GenericModification::DeleteRanges { .. } => Ok(Vec::new()),
            GenericModification::DeleteLinesMatching { .. } => Ok(Vec::new()),
            GenericModification::MoveFiles {
                sources,
                destination_dir,
                create_destination_dir,
                overwrite,
                ..
            } => self.plan_move_files(
                sources,
                destination_dir,
                *create_destination_dir,
                *overwrite,
                snapshot,
            ),
            GenericModification::MoveRanges { .. } => Ok(Vec::new()),
            GenericModification::InsertLines { .. } => Ok(Vec::new()),
            GenericModification::ReplaceRanges { .. } => Ok(Vec::new()),
            GenericModification::TextReplace { .. } => Ok(Vec::new()),
        }
    }

    fn plan_create_file(
        &self,
        path: &Path,
        content: &str,
        overwrite: bool,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        let path_existed = self.snapshot_exists(snapshot, path)?;
        if path_existed {
            if !self.snapshot_is_symlink(snapshot, path)? && self.snapshot_is_dir(snapshot, path)? {
                return Err(SmartEditError::ExpectedFileButFoundDirectory {
                    path: path.to_path_buf(),
                });
            }
            if !overwrite {
                return Err(SmartEditError::FileAlreadyExists {
                    path: path.to_path_buf(),
                });
            }
        }

        let mut actions = self.parent_directory_actions(path, true, snapshot)?;
        let expected_identity = overwrite
            .then(|| self.content_identity(path))
            .transpose()?
            .flatten();
        if overwrite
            && path_existed
            && snapshot.get(path).is_none()
            && self.fs.identity_checks_supported()
            && expected_identity.is_none()
        {
            return Err(SmartEditError::MissingFile {
                path: path.to_path_buf(),
            });
        }
        actions.push(PlannedAction::WriteFile {
            path: path.to_path_buf(),
            bytes: content.as_bytes().to_vec(),
            overwrite,
            expected_identity,
        });
        Ok(actions)
    }

    fn plan_create_directory(
        &self,
        path: &Path,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        if self.snapshot_exists(snapshot, path)? && self.snapshot_is_file(snapshot, path)? {
            return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                path: path.to_path_buf(),
            });
        }

        Ok(vec![PlannedAction::CreateDirectory {
            path: path.to_path_buf(),
        }])
    }

    fn plan_delete_files(
        &self,
        targets: &PathSpec,
        missing_matches_ok: bool,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        let matches = self.resolve_file_matches(targets, snapshot)?;
        if matches.is_empty() && !missing_matches_ok {
            return Err(SmartEditError::NoFilesMatched {
                description: self.describe_file_source_spec(targets),
            });
        }

        matches
            .into_iter()
            .map(|matched| {
                let expected_identity = self.entry_identity(&matched.path)?;
                if snapshot.get(&matched.path).is_none()
                    && self.fs.identity_checks_supported()
                    && expected_identity.is_none()
                {
                    return Err(SmartEditError::MissingFile { path: matched.path });
                }
                Ok(PlannedAction::DeleteFile {
                    expected_identity,
                    path: matched.path,
                    missing_ok: false,
                })
            })
            .collect()
    }

    fn plan_move_files(
        &self,
        sources: &PathSpec,
        destination_dir: &crate::edit::PathDestination,
        create_destination_dir: bool,
        overwrite: bool,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        let PathDestinationKind::Directory {
            path: destination_dir,
        } = &destination_dir.kind;
        let matches = self.resolve_file_matches(sources, snapshot)?;
        if matches.is_empty() {
            return Err(SmartEditError::NoFilesMatched {
                description: self.describe_file_source_spec(sources),
            });
        }

        let mut actions = Vec::new();

        for matched in matches {
            let destination_path = destination_dir.join(&matched.relative_path);
            if entry_path_identity(&destination_path) == entry_path_identity(&matched.path) {
                continue;
            }

            if self.snapshot_exists(snapshot, destination_path.as_path())? {
                if !self.snapshot_is_symlink(snapshot, destination_path.as_path())?
                    && self.snapshot_is_dir(snapshot, destination_path.as_path())?
                {
                    return Err(SmartEditError::ExpectedFileButFoundDirectory {
                        path: destination_path,
                    });
                }
                if !overwrite {
                    return Err(SmartEditError::FileAlreadyExists {
                        path: destination_path,
                    });
                }
            }

            actions.extend(self.parent_directory_actions(
                destination_path.as_path(),
                create_destination_dir,
                snapshot,
            )?);
            actions.push(PlannedAction::MoveFile {
                source: matched.path,
                destination: destination_path,
                overwrite,
            });
        }

        Ok(actions)
    }

    fn plan_delete_ranges_into(
        &self,
        modification_index: usize,
        target: &crate::edit::FileRangeSelection,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let update = self.get_or_load_text_update(
            modification_index,
            target.path.as_path(),
            false,
            pending,
            snapshot,
        )?;
        let deletions = target
            .ranges
            .resolve_against(target.path.as_path(), &update.original_content)?;
        update
            .deletions
            .extend(deletions.into_iter().map(|range| PendingTextDeletion {
                range,
                kind: PendingDeletionKind::Delete,
                modification_index,
            }));
        Ok(())
    }

    fn plan_move_ranges_into(
        &self,
        modification_index: usize,
        source_path: &Path,
        ranges: &RangeSet,
        destination: TextInsertionTarget<'_>,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let source_content = {
            let update = self.get_or_load_text_update(
                modification_index,
                source_path,
                false,
                pending,
                snapshot,
            )?;
            update.original_content.clone()
        };
        let resolved_ranges = ranges.resolve_against(source_path, &source_content)?;
        let moved_text = ranges.extract_from(source_path, &source_content)?;

        {
            let update = self.get_or_load_text_update(
                modification_index,
                source_path,
                false,
                pending,
                snapshot,
            )?;
            update.deletions.extend(
                resolved_ranges
                    .into_iter()
                    .map(|range| PendingTextDeletion {
                        range,
                        kind: PendingDeletionKind::Move,
                        modification_index,
                    }),
            );
        }

        let destination_update = self.get_or_load_text_update(
            modification_index,
            destination.path,
            destination.create_if_missing,
            pending,
            snapshot,
        )?;
        let destination_offset = resolve_insertion_offset(
            destination.path,
            &destination_update.original_content,
            destination.offset,
        )?;
        destination_update.insertions.push(PendingTextInsertion {
            offset: destination_offset,
            text: moved_text,
            order: modification_index,
        });
        Ok(())
    }

    fn plan_insert_lines_into(
        &self,
        modification_index: usize,
        destination: TextInsertionTarget<'_>,
        content: &str,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let destination_update = self.get_or_load_text_update(
            modification_index,
            destination.path,
            destination.create_if_missing,
            pending,
            snapshot,
        )?;
        let destination_offset = resolve_insertion_offset(
            destination.path,
            &destination_update.original_content,
            destination.offset,
        )?;
        destination_update.insertions.push(PendingTextInsertion {
            offset: destination_offset,
            text: content.to_owned(),
            order: modification_index,
        });
        Ok(())
    }

    fn plan_replace_ranges_into(
        &self,
        modification_index: usize,
        target: &crate::edit::FileRangeSelection,
        content: &str,
        create_destination_if_missing: bool,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let update = self.get_or_load_text_update(
            modification_index,
            target.path.as_path(),
            create_destination_if_missing,
            pending,
            snapshot,
        )?;
        let deletions = target
            .ranges
            .resolve_against(target.path.as_path(), &update.original_content)?;
        let insertion_offset = deletions
            .first()
            .map(|range| range.start)
            .unwrap_or_else(|| update.original_content.len());
        update
            .deletions
            .extend(deletions.into_iter().map(|range| PendingTextDeletion {
                range,
                kind: PendingDeletionKind::Replace,
                modification_index,
            }));
        update.insertions.push(PendingTextInsertion {
            offset: insertion_offset,
            text: content.to_owned(),
            order: modification_index,
        });
        Ok(())
    }

    fn plan_delete_lines_matching_into(
        &self,
        modification_index: usize,
        target: &crate::edit::FilePatternMatch,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let update = self.get_or_load_text_update(
            modification_index,
            target.path.as_path(),
            false,
            pending,
            snapshot,
        )?;
        let matcher =
            Regex::new(&target.pattern).map_err(|error| SmartEditError::InvalidRegexPattern {
                pattern: target.pattern.clone(),
                message: error.to_string(),
            })?;
        let deletions = resolve_matching_line_ranges(&update.original_content, |line| {
            let line = line
                .strip_suffix("\r\n")
                .or_else(|| line.strip_suffix('\n'))
                .unwrap_or(line);
            matcher.is_match(line)
        });
        update
            .deletions
            .extend(deletions.into_iter().map(|range| PendingTextDeletion {
                range,
                kind: PendingDeletionKind::Delete,
                modification_index,
            }));
        Ok(())
    }

    fn plan_text_replace_into(
        &self,
        modification_index: usize,
        targets: &PathSpec,
        pattern: &TextPattern,
        replacement: &str,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let matches = self.resolve_file_matches(targets, snapshot)?;
        if matches.is_empty() {
            return Err(SmartEditError::NoFilesMatched {
                description: self.describe_file_source_spec(targets),
            });
        }

        for matched in matches {
            let update = self.get_or_load_text_update(
                modification_index,
                matched.path.as_path(),
                false,
                pending,
                snapshot,
            )?;
            let replacements = self.resolve_text_replacements(
                matched.path.as_path(),
                &update.original_content,
                pattern,
                replacement,
            )?;

            for (range, replacement_text) in replacements {
                update.deletions.push(PendingTextDeletion {
                    range,
                    kind: PendingDeletionKind::Replace,
                    modification_index,
                });
                update.insertions.push(PendingTextInsertion {
                    offset: range.start,
                    text: replacement_text,
                    order: modification_index,
                });
            }
        }

        Ok(())
    }

    fn resolve_text_replacements(
        &self,
        _path: &Path,
        content: &str,
        pattern: &TextPattern,
        replacement: &str,
    ) -> Result<Vec<(crate::edit::TextRange, String)>> {
        match pattern {
            TextPattern::Literal { text, .. } => {
                if text.is_empty() {
                    return Err(SmartEditError::EmptyTextPattern);
                }

                let mut replacements = Vec::new();
                let mut search_start = 0usize;
                while let Some(relative_start) = content[search_start..].find(text) {
                    let start = search_start + relative_start;
                    let end = start + text.len();
                    replacements.push((
                        crate::edit::TextRange {
                            start,
                            end,
                            span: None,
                        },
                        replacement.to_owned(),
                    ));
                    search_start = end;
                }

                Ok(replacements)
            }
            TextPattern::Regex { pattern, .. } => {
                let regex =
                    Regex::new(pattern).map_err(|error| SmartEditError::InvalidRegexPattern {
                        pattern: pattern.clone(),
                        message: error.to_string(),
                    })?;
                let mut replacements = Vec::new();

                for captures in regex.captures_iter(content) {
                    let Some(matched) = captures.get(0) else {
                        continue;
                    };
                    let mut expanded = String::new();
                    captures.expand(replacement, &mut expanded);
                    replacements.push((
                        crate::edit::TextRange {
                            start: matched.start(),
                            end: matched.end(),
                            span: None,
                        },
                        expanded,
                    ));
                }

                Ok(replacements)
            }
        }
    }

    fn get_or_load_text_update<'a>(
        &self,
        modification_index: usize,
        path: &Path,
        create_if_missing: bool,
        pending: &'a mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<&'a mut PendingTextFileUpdate> {
        let mut identity = content_path_identity(path);
        for (candidate, update) in &pending.files {
            if self.same_content_file(&update.path, path)? {
                identity = candidate.clone();
                break;
            }
        }
        if !pending.files.contains_key(&identity) {
            let original_existed = self.snapshot_exists(snapshot, path)?;
            let original_content = if original_existed {
                self.snapshot_read_text(snapshot, path)?
            } else if create_if_missing {
                String::new()
            } else {
                return Err(SmartEditError::MissingFile {
                    path: path.to_path_buf(),
                });
            };

            let resolved_path = if original_existed {
                content_path_identity(path)
            } else {
                path.to_path_buf()
            };
            let expected_identity = if original_existed {
                self.content_identity(path)?
            } else {
                None
            };
            if original_existed
                && snapshot.get(path).is_none()
                && self.fs.identity_checks_supported()
                && expected_identity.is_none()
            {
                return Err(SmartEditError::MissingFile {
                    path: path.to_path_buf(),
                });
            }
            pending.files.insert(
                identity.clone(),
                PendingTextFileUpdate {
                    path: resolved_path,
                    original_existed,
                    expected_identity,
                    original_content,
                    deletions: Vec::new(),
                    insertions: Vec::new(),
                    first_modification_index: modification_index,
                },
            );
        }

        let update = pending
            .files
            .get_mut(&identity)
            .expect("text update must exist after insertion");
        update.first_modification_index = update.first_modification_index.min(modification_index);
        Ok(update)
    }

    fn finalize_text_updates(
        &self,
        pending: PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<Vec<(usize, Vec<PlannedAction>)>> {
        let mut finalized = Vec::new();

        for (_, update) in pending.files {
            let path = &update.path;
            let updated = self.render_pending_text_update(path.as_path(), &update)?;
            if update.original_existed && updated == update.original_content {
                continue;
            }

            let mut actions = self.parent_directory_actions(path.as_path(), true, snapshot)?;
            actions.push(PlannedAction::WriteFile {
                path: path.clone(),
                bytes: updated.into_bytes(),
                overwrite: update.original_existed,
                expected_identity: update.expected_identity,
            });
            finalized.push((update.first_modification_index, actions));
        }

        Ok(finalized)
    }

    fn render_pending_text_update(
        &self,
        path: &Path,
        update: &PendingTextFileUpdate,
    ) -> Result<String> {
        self.validate_destructive_overlaps(path, &update.deletions)?;
        let deletion_ranges = update
            .deletions
            .iter()
            .map(|deletion| deletion.range)
            .collect::<Vec<_>>();
        let merged_deletions =
            self.merge_deletions(path, &update.original_content, &deletion_ranges)?;
        let mut insertions = update.insertions.clone();
        insertions.sort_by(|left, right| {
            left.offset
                .cmp(&right.offset)
                .then(left.order.cmp(&right.order))
        });

        for insertion in &insertions {
            if let Some(range) = deletion_ranges
                .iter()
                .find(|range| range.start < insertion.offset && insertion.offset < range.end)
            {
                return Err(SmartEditError::InsertionPointInsideDeletedRange {
                    path: path.to_path_buf(),
                    offset: insertion.offset,
                    range_start: range.start,
                    range_end: range.end,
                });
            }
        }

        let removed_len: usize = merged_deletions
            .iter()
            .map(crate::edit::TextRange::len)
            .sum();
        let inserted_len: usize = insertions
            .iter()
            .map(|insertion| insertion.text.len())
            .sum();
        let mut updated =
            String::with_capacity(update.original_content.len() - removed_len + inserted_len);

        let mut cursor = 0usize;
        let mut deletion_index = 0usize;
        let mut insertion_index = 0usize;

        loop {
            let next_insertion_offset = insertions
                .get(insertion_index)
                .map(|insertion| insertion.offset)
                .unwrap_or(usize::MAX);
            let next_deletion_start = merged_deletions
                .get(deletion_index)
                .map(|range| range.start)
                .unwrap_or(usize::MAX);

            if next_insertion_offset == usize::MAX && next_deletion_start == usize::MAX {
                break;
            }

            if next_insertion_offset <= next_deletion_start {
                updated.push_str(&update.original_content[cursor..next_insertion_offset]);
                cursor = next_insertion_offset;

                while let Some(insertion) = insertions.get(insertion_index) {
                    if insertion.offset != next_insertion_offset {
                        break;
                    }
                    updated.push_str(&insertion.text);
                    insertion_index += 1;
                }
            } else {
                let deletion = merged_deletions[deletion_index];
                updated.push_str(&update.original_content[cursor..deletion.start]);
                cursor = deletion.end;
                deletion_index += 1;
            }
        }

        updated.push_str(&update.original_content[cursor..]);
        Ok(updated)
    }

    fn validate_destructive_overlaps(
        &self,
        path: &Path,
        deletions: &[PendingTextDeletion],
    ) -> Result<()> {
        for (index, left) in deletions.iter().enumerate() {
            for right in &deletions[index + 1..] {
                if left.modification_index == right.modification_index {
                    continue;
                }
                let overlaps =
                    left.range.start < right.range.end && right.range.start < left.range.end;
                let destructive = left.kind != PendingDeletionKind::Delete
                    || right.kind != PendingDeletionKind::Delete;
                if overlaps && destructive {
                    return Err(SmartEditError::OverlappingDestructiveEdits {
                        path: path.to_path_buf(),
                        first_modification: left.modification_index,
                        second_modification: right.modification_index,
                    });
                }
            }
        }

        Ok(())
    }

    fn merge_deletions(
        &self,
        path: &Path,
        content: &str,
        deletions: &[crate::edit::TextRange],
    ) -> Result<Vec<crate::edit::TextRange>> {
        if deletions.is_empty() {
            return Ok(Vec::new());
        }

        for deletion in deletions {
            if deletion.start > deletion.end {
                return Err(SmartEditError::InvalidRange {
                    start: deletion.start,
                    end: deletion.end,
                });
            }
            if deletion.end > content.len() {
                return Err(SmartEditError::RangeOutOfBounds {
                    path: path.to_path_buf(),
                    start: deletion.start,
                    end: deletion.end,
                    len: content.len(),
                });
            }
            if !content.is_char_boundary(deletion.start) {
                return Err(SmartEditError::RangeNotOnCharBoundary {
                    path: path.to_path_buf(),
                    offset: deletion.start,
                });
            }
            if !content.is_char_boundary(deletion.end) {
                return Err(SmartEditError::RangeNotOnCharBoundary {
                    path: path.to_path_buf(),
                    offset: deletion.end,
                });
            }
        }

        let mut merged = deletions.to_vec();
        merged.sort_by_key(|range| (range.start, range.end));

        let mut coalesced: Vec<crate::edit::TextRange> = Vec::with_capacity(merged.len());
        for range in merged {
            if let Some(last) = coalesced.last_mut()
                && range.start < last.end
            {
                last.end = last.end.max(range.end);
                continue;
            }
            coalesced.push(range);
        }

        Ok(coalesced)
    }

    fn resolve_file_matches(
        &self,
        spec: &PathSpec,
        snapshot: &SnapshotState,
    ) -> Result<Vec<ResolvedFileMatch>> {
        match &spec.kind {
            PathSpecKind::ExactFile { path } => self.resolve_exact_file(path, snapshot),
            PathSpecKind::FilesInDirectory { root, recursive } => {
                self.resolve_directory_files(root.as_path(), *recursive, snapshot)
            }
            PathSpecKind::Glob { root, pattern } => {
                self.resolve_glob_matches(root.as_path(), pattern, snapshot)
            }
            PathSpecKind::Regex { root, pattern } => {
                self.resolve_regex_matches(root.as_path(), pattern, snapshot)
            }
        }
    }

    fn resolve_exact_file(
        &self,
        path: &Path,
        snapshot: &SnapshotState,
    ) -> Result<Vec<ResolvedFileMatch>> {
        if !self.snapshot_exists(snapshot, path)? {
            return Ok(Vec::new());
        }
        if !self.snapshot_is_symlink(snapshot, path)? && self.snapshot_is_dir(snapshot, path)? {
            return Err(SmartEditError::ExpectedFileButFoundDirectory {
                path: path.to_path_buf(),
            });
        }

        let relative_path = path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());

        Ok(vec![ResolvedFileMatch {
            path: path.to_path_buf(),
            relative_path,
        }])
    }

    fn resolve_directory_files(
        &self,
        root: &Path,
        recursive: bool,
        snapshot: &SnapshotState,
    ) -> Result<Vec<ResolvedFileMatch>> {
        if !self.snapshot_exists(snapshot, root)? {
            return Ok(Vec::new());
        }
        if !self.snapshot_is_dir(snapshot, root)? {
            return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                path: root.to_path_buf(),
            });
        }

        let files = self.snapshot_list_files(snapshot, root, recursive)?;
        Ok(files
            .into_iter()
            .map(|path| ResolvedFileMatch {
                relative_path: path
                    .strip_prefix(root)
                    .expect("listed path should live below root")
                    .to_path_buf(),
                path,
            })
            .collect())
    }

    fn resolve_glob_matches(
        &self,
        root: &Path,
        pattern: &str,
        snapshot: &SnapshotState,
    ) -> Result<Vec<ResolvedFileMatch>> {
        if !self.snapshot_exists(snapshot, root)? {
            return Ok(Vec::new());
        }
        if !self.snapshot_is_dir(snapshot, root)? {
            return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                path: root.to_path_buf(),
            });
        }

        let matcher = Glob::new(pattern)
            .map_err(|error| SmartEditError::InvalidGlobPattern {
                pattern: pattern.to_owned(),
                message: error.to_string(),
            })?
            .compile_matcher();

        let mut matches = Vec::new();
        for path in self.snapshot_list_files(snapshot, root, true)? {
            let relative_path = path
                .strip_prefix(root)
                .expect("listed path should live below root")
                .to_path_buf();
            if matcher.is_match(Self::normalize_path_for_glob(relative_path.as_path())) {
                matches.push(ResolvedFileMatch {
                    path,
                    relative_path,
                });
            }
        }

        Ok(matches)
    }

    fn resolve_regex_matches(
        &self,
        root: &Path,
        pattern: &str,
        snapshot: &SnapshotState,
    ) -> Result<Vec<ResolvedFileMatch>> {
        if !self.snapshot_exists(snapshot, root)? {
            return Ok(Vec::new());
        }
        if !self.snapshot_is_dir(snapshot, root)? {
            return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                path: root.to_path_buf(),
            });
        }

        let matcher = Regex::new(pattern).map_err(|error| SmartEditError::InvalidRegexPattern {
            pattern: pattern.to_owned(),
            message: error.to_string(),
        })?;

        let mut matches = Vec::new();
        for path in self.snapshot_list_files(snapshot, root, true)? {
            let relative_path = path
                .strip_prefix(root)
                .expect("listed path should live below root")
                .to_path_buf();
            if matcher.is_match(&Self::normalize_path_for_glob(relative_path.as_path())) {
                matches.push(ResolvedFileMatch {
                    path,
                    relative_path,
                });
            }
        }

        Ok(matches)
    }

    fn normalize_path_for_glob(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn describe_file_source_spec(&self, spec: &PathSpec) -> String {
        match &spec.kind {
            PathSpecKind::ExactFile { path } => format!("exact file {}", path.display()),
            PathSpecKind::FilesInDirectory { root, recursive } => {
                if *recursive {
                    format!("files under directory {}", root.display())
                } else {
                    format!("top-level files in directory {}", root.display())
                }
            }
            PathSpecKind::Glob { root, pattern } => {
                format!("glob `{pattern}` under {}", root.display())
            }
            PathSpecKind::Regex { root, pattern } => {
                format!("regex `{pattern}` under {}", root.display())
            }
        }
    }

    fn register_targets(
        &self,
        modification_index: usize,
        actions: &[PlannedAction],
        targets: &mut BTreeMap<PathBuf, PlannedTarget>,
    ) -> Result<()> {
        for action in actions {
            let action_targets = match action {
                PlannedAction::CreateDirectory { path } => {
                    vec![(
                        path.clone(),
                        PlannedTargetKind::Directory,
                        entry_path_identity(path),
                        false,
                    )]
                }
                PlannedAction::WriteFile { path, .. } => {
                    vec![(
                        path.clone(),
                        PlannedTargetKind::File,
                        content_path_identity(path),
                        true,
                    )]
                }
                PlannedAction::DeleteFile { path, .. } => {
                    vec![(
                        path.clone(),
                        PlannedTargetKind::File,
                        entry_path_identity(path),
                        false,
                    )]
                }
                PlannedAction::MoveFile {
                    source,
                    destination,
                    ..
                } => vec![
                    (
                        source.clone(),
                        PlannedTargetKind::File,
                        entry_path_identity(source),
                        false,
                    ),
                    (
                        destination.clone(),
                        PlannedTargetKind::File,
                        entry_path_identity(destination),
                        false,
                    ),
                ],
            };

            for (path, kind, identity, follows_final_symlink) in action_targets {
                if let Some(existing) = targets.get(&identity) {
                    if existing.kind == PlannedTargetKind::Directory
                        && kind == PlannedTargetKind::Directory
                    {
                        continue;
                    }
                    return Err(SmartEditError::ConflictingActionTargets {
                        path,
                        first_modification: existing.modification_index,
                        second_modification: modification_index,
                    });
                }

                for (existing_path, existing) in targets.iter() {
                    if follows_final_symlink
                        && existing.follows_final_symlink
                        && self.same_content_file(&path, &existing.path)?
                    {
                        return Err(SmartEditError::ConflictingActionTargets {
                            path,
                            first_modification: existing.modification_index,
                            second_modification: modification_index,
                        });
                    }
                    let existing_file_is_ancestor = existing.kind == PlannedTargetKind::File
                        && identity.starts_with(existing_path);
                    let new_file_is_ancestor =
                        kind == PlannedTargetKind::File && existing_path.starts_with(&identity);
                    if existing_file_is_ancestor || new_file_is_ancestor {
                        return Err(SmartEditError::ConflictingActionTargets {
                            path,
                            first_modification: existing.modification_index,
                            second_modification: modification_index,
                        });
                    }
                }

                targets.insert(
                    identity,
                    PlannedTarget {
                        kind,
                        modification_index,
                        path,
                        follows_final_symlink,
                    },
                );
            }
        }

        Ok(())
    }

    fn apply_plan(&self, plan: &EvaluationPlan) -> Result<()> {
        for action in plan.actions() {
            self.apply_action(action)?;
        }

        Ok(())
    }

    fn apply_action(&self, action: &PlannedAction) -> Result<()> {
        match action {
            PlannedAction::CreateDirectory { path } => {
                self.fs
                    .create_dir_all(path)
                    .map_err(|source| SmartEditError::Io {
                        operation: "create directory",
                        path: path.clone(),
                        source,
                    })
            }
            PlannedAction::WriteFile {
                path,
                bytes,
                overwrite,
                expected_identity,
            } => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    self.fs
                        .create_dir_all(parent)
                        .map_err(|source| SmartEditError::Io {
                            operation: "create directory",
                            path: parent.to_path_buf(),
                            source,
                        })?;
                }
                if *overwrite {
                    let result = if let Some(expected) = expected_identity {
                        self.fs.write_bytes_checked(path, bytes, expected)
                    } else {
                        self.fs.write_bytes(path, bytes)
                    };
                    result.map_err(|source| SmartEditError::Io {
                        operation: "write file",
                        path: path.clone(),
                        source,
                    })
                } else {
                    self.fs.create_new_bytes(path, bytes).map_err(|source| {
                        if source.kind() == std::io::ErrorKind::AlreadyExists {
                            SmartEditError::FileAlreadyExists { path: path.clone() }
                        } else {
                            SmartEditError::Io {
                                operation: "create file",
                                path: path.clone(),
                                source,
                            }
                        }
                    })
                }
            }
            PlannedAction::DeleteFile {
                path,
                missing_ok,
                expected_identity,
            } => match if let Some(expected) = expected_identity {
                self.fs.remove_file_checked(path, expected)
            } else {
                self.fs.remove_file(path)
            } {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound && *missing_ok => {
                    Ok(())
                }
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    Err(SmartEditError::MissingFile { path: path.clone() })
                }
                Err(source) => Err(SmartEditError::Io {
                    operation: "delete file",
                    path: path.clone(),
                    source,
                }),
            },
            PlannedAction::MoveFile {
                source,
                destination,
                overwrite,
            } => self
                .fs
                .move_file(source, destination, *overwrite)
                .map_err(|source_error| SmartEditError::Io {
                    operation: "move file",
                    path: source.clone(),
                    source: source_error,
                }),
        }
    }

    fn parent_directory_actions(
        &self,
        path: &Path,
        create_if_missing: bool,
        snapshot: &SnapshotState,
    ) -> Result<Vec<PlannedAction>> {
        let Some(parent) = path.parent() else {
            return Ok(Vec::new());
        };
        if parent.as_os_str().is_empty() {
            return Ok(Vec::new());
        }

        if self.snapshot_exists(snapshot, parent)? {
            if self.snapshot_is_file(snapshot, parent)? {
                return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                    path: parent.to_path_buf(),
                });
            }
            return Ok(Vec::new());
        }

        if !create_if_missing {
            return Err(SmartEditError::MissingFile {
                path: parent.to_path_buf(),
            });
        }

        Ok(vec![PlannedAction::CreateDirectory {
            path: parent.to_path_buf(),
        }])
    }

    fn snapshot_exists(&self, snapshot: &SnapshotState, path: &Path) -> Result<bool> {
        match snapshot.get(path) {
            Some(SnapshotEntry::File(_) | SnapshotEntry::MovedFile(_))
            | Some(SnapshotEntry::Directory) => Ok(true),
            Some(SnapshotEntry::Missing) => Ok(false),
            None => self.exists(path),
        }
    }

    fn snapshot_is_file(&self, snapshot: &SnapshotState, path: &Path) -> Result<bool> {
        match snapshot.get(path) {
            Some(SnapshotEntry::File(_) | SnapshotEntry::MovedFile(_)) => Ok(true),
            Some(SnapshotEntry::Directory) | Some(SnapshotEntry::Missing) => Ok(false),
            None => self.is_file(path),
        }
    }

    fn snapshot_is_dir(&self, snapshot: &SnapshotState, path: &Path) -> Result<bool> {
        match snapshot.get(path) {
            Some(SnapshotEntry::Directory) => Ok(true),
            Some(SnapshotEntry::File(_) | SnapshotEntry::MovedFile(_))
            | Some(SnapshotEntry::Missing) => Ok(false),
            None => self.is_dir(path),
        }
    }

    fn snapshot_is_symlink(&self, snapshot: &SnapshotState, path: &Path) -> Result<bool> {
        match snapshot.get(path) {
            Some(_) => Ok(false),
            None => self.is_symlink(path),
        }
    }

    fn snapshot_read_bytes(&self, snapshot: &SnapshotState, path: &Path) -> Result<Vec<u8>> {
        let direct_entry = snapshot.get(path);
        let entry = if direct_entry.is_some() {
            direct_entry
        } else if let Some(identity) = self.content_identity(path)? {
            snapshot.get_content_object(&identity)
        } else {
            None
        };
        match entry {
            Some(SnapshotEntry::File(bytes)) => Ok(bytes.clone()),
            Some(SnapshotEntry::MovedFile(source)) => self.read_bytes(source),
            Some(SnapshotEntry::Directory) => Err(SmartEditError::ExpectedFileButFoundDirectory {
                path: path.to_path_buf(),
            }),
            Some(SnapshotEntry::Missing) => Err(SmartEditError::MissingFile {
                path: path.to_path_buf(),
            }),
            None => self.read_bytes(path),
        }
    }

    fn snapshot_read_text(&self, snapshot: &SnapshotState, path: &Path) -> Result<String> {
        let bytes = self.snapshot_read_bytes(snapshot, path)?;
        String::from_utf8(bytes).map_err(|source| SmartEditError::InvalidUtf8 {
            path: path.to_path_buf(),
            source,
        })
    }

    fn snapshot_list_files(
        &self,
        snapshot: &SnapshotState,
        root: &Path,
        recursive: bool,
    ) -> Result<Vec<PathBuf>> {
        if !self.snapshot_exists(snapshot, root)? {
            return Ok(Vec::new());
        }
        if !self.snapshot_is_dir(snapshot, root)? {
            return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                path: root.to_path_buf(),
            });
        }

        let root_identity = entry_path_identity(root);
        let mut files = BTreeMap::new();
        if self.exists(root)? && self.is_dir(root)? {
            for path in self.list_files(root, recursive)? {
                files.insert(entry_path_identity(&path), path);
            }
        }

        for (identity, entry) in &snapshot.entries {
            if !identity.starts_with(&root_identity) {
                continue;
            }
            let Ok(relative) = identity.strip_prefix(&root_identity) else {
                continue;
            };
            if !recursive && relative.components().count() != 1 {
                continue;
            }

            match entry {
                SnapshotEntry::File(_) | SnapshotEntry::MovedFile(_) => {
                    files.insert(identity.clone(), root.join(relative));
                }
                SnapshotEntry::Missing => {
                    files.remove(identity);
                }
                SnapshotEntry::Directory => {}
            }
        }

        Ok(files.into_values().collect())
    }

    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        self.fs
            .read_bytes(path)
            .map_err(|source| match source.kind() {
                std::io::ErrorKind::NotFound => SmartEditError::MissingFile {
                    path: path.to_path_buf(),
                },
                _ => SmartEditError::Io {
                    operation: "read file",
                    path: path.to_path_buf(),
                    source,
                },
            })
    }

    fn exists(&self, path: &Path) -> Result<bool> {
        self.fs.exists(path).map_err(|source| SmartEditError::Io {
            operation: "check path",
            path: path.to_path_buf(),
            source,
        })
    }

    fn is_file(&self, path: &Path) -> Result<bool> {
        self.fs.is_file(path).map_err(|source| SmartEditError::Io {
            operation: "check file type",
            path: path.to_path_buf(),
            source,
        })
    }

    fn is_dir(&self, path: &Path) -> Result<bool> {
        self.fs.is_dir(path).map_err(|source| SmartEditError::Io {
            operation: "check file type",
            path: path.to_path_buf(),
            source,
        })
    }

    fn is_symlink(&self, path: &Path) -> Result<bool> {
        self.fs
            .is_symlink(path)
            .map_err(|source| SmartEditError::Io {
                operation: "check file type",
                path: path.to_path_buf(),
                source,
            })
    }

    fn content_identity(&self, path: &Path) -> Result<Option<FileIdentity>> {
        match self.fs.content_identity(path) {
            Ok(identity) => Ok(identity),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SmartEditError::Io {
                operation: "identify file contents",
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    fn entry_identity(&self, path: &Path) -> Result<Option<FileIdentity>> {
        match self.fs.entry_identity(path) {
            Ok(identity) => Ok(identity),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SmartEditError::Io {
                operation: "identify file entry",
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    fn same_content_file(&self, left: &Path, right: &Path) -> Result<bool> {
        match self.fs.same_content_file(left, right) {
            Ok(same) => Ok(same),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(SmartEditError::Io {
                operation: "compare file identity",
                path: right.to_path_buf(),
                source,
            }),
        }
    }

    fn list_files(&self, root: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
        self.fs
            .list_files(root, recursive)
            .map_err(|source| SmartEditError::Io {
                operation: "list files",
                path: root.to_path_buf(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::edit::{
        EditProgram, FileInsertion, FilePatternMatch, FileRangeSelection, GenericModification,
        PathDestination, PathSpec, ProgramMode, RangeSet, TextPattern, TextRange,
    };
    use crate::error::SmartEditError;
    use crate::fs::FileSystem;
    use crate::plan::ExecutionOptions;

    use super::{Executor, content_path_identity, entry_path_identity};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("smartedit-{name}-{}-{unique}", process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[derive(Clone)]
    struct FailingWriteFileSystem {
        written: Rc<RefCell<Vec<PathBuf>>>,
        fail_path: PathBuf,
        create_new_collision: bool,
    }

    impl FileSystem for FailingWriteFileSystem {
        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn write_bytes(&self, path: &Path, _contents: &[u8]) -> io::Result<()> {
            if path == self.fail_path {
                return Err(io::Error::other("injected write failure"));
            }
            self.written.borrow_mut().push(path.to_path_buf());
            Ok(())
        }

        fn create_new_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            if self.create_new_collision {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "file appeared after planning",
                ));
            }
            self.write_bytes(path, contents)
        }

        fn read_bytes(&self, _path: &Path) -> io::Result<Vec<u8>> {
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
        }

        fn remove_file(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn move_file(
            &self,
            _source: &Path,
            _destination: &Path,
            _overwrite: bool,
        ) -> io::Result<()> {
            Ok(())
        }

        fn exists(&self, _path: &Path) -> io::Result<bool> {
            Ok(false)
        }

        fn is_file(&self, _path: &Path) -> io::Result<bool> {
            Ok(false)
        }

        fn is_dir(&self, _path: &Path) -> io::Result<bool> {
            Ok(false)
        }

        fn list_files(&self, _root: &Path, _recursive: bool) -> io::Result<Vec<PathBuf>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn creates_directories_and_files_recursively() {
        let dir = TestDir::new("create");
        let nested_dir = dir.path().join("a/b/c");
        let nested_file = dir.path().join("nested/tree/file.txt");

        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateDirectory {
                path: nested_dir.clone(),
                span: None,
            }
            .into(),
            GenericModification::CreateFile {
                path: nested_file.clone(),
                content: "hello".into(),
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert!(nested_dir.is_dir());
        assert_eq!(fs::read_to_string(nested_file).unwrap(), "hello");
    }

    #[test]
    fn applying_a_plan_is_explicitly_best_effort() {
        let written = Rc::new(RefCell::new(Vec::new()));
        let fs = FailingWriteFileSystem {
            written: Rc::clone(&written),
            fail_path: PathBuf::from("second.txt"),
            create_new_collision: false,
        };
        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateFile {
                path: PathBuf::from("first.txt"),
                content: "first".into(),
                overwrite: true,
                span: None,
            }
            .into(),
            GenericModification::CreateFile {
                path: PathBuf::from("second.txt"),
                content: "second".into(),
                overwrite: true,
                span: None,
            }
            .into(),
        ]);

        let error = Executor::with_file_system(fs)
            .execute(&program)
            .unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::Io {
                operation: "write file",
                ..
            }
        ));
        assert_eq!(&*written.borrow(), &[PathBuf::from("first.txt")]);
    }

    #[test]
    fn non_overwrite_create_fails_if_target_appears_after_planning() {
        let written = Rc::new(RefCell::new(Vec::new()));
        let fs = FailingWriteFileSystem {
            written: Rc::clone(&written),
            fail_path: PathBuf::from("never-fails-via-write.txt"),
            create_new_collision: true,
        };
        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateFile {
                path: PathBuf::from("race.txt"),
                content: "planned".into(),
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        let error = Executor::with_file_system(fs)
            .execute(&program)
            .unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::FileAlreadyExists { path } if path == Path::new("race.txt")
        ));
        assert!(written.borrow().is_empty());
    }

    #[test]
    fn deletes_multiple_ranges_from_a_file() {
        let dir = TestDir::new("delete-ranges");
        let file = dir.path().join("data.txt");
        fs::write(&file, "zero\none\ntwo\nthree\nfour\n").unwrap();

        let ranges = RangeSet::new(vec![
            TextRange::new(1, 2).unwrap(),
            TextRange::new(3, 4).unwrap(),
        ]);
        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteRanges {
                target: FileRangeSelection::new(&file, ranges),
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "zero\ntwo\nfour\n");
    }

    #[test]
    fn inserts_lines_into_a_file() {
        let dir = TestDir::new("insert-lines");
        let file = dir.path().join("data.txt");
        fs::write(&file, "a\nb\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&file, 1),
                content: "x\ny\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "a\nx\ny\nb\n");
    }

    #[test]
    fn snapshot_text_edits_merge_lexical_aliases_into_one_write() {
        let dir = TestDir::new("snapshot-path-aliases");
        let file = dir.path().join("data.txt");
        let alias_parent = dir.path().join("alias");
        fs::create_dir_all(&alias_parent).unwrap();
        fs::write(&file, "base\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&file, 0),
                content: "first\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(dir.path().join("./data.txt"), 0),
                content: "second\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(alias_parent.join("../data.txt"), 0),
                content: "third\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new().evaluate(&program).unwrap();
        assert_eq!(
            plan.actions()
                .filter(|action| matches!(action, crate::plan::PlannedAction::WriteFile { .. }))
                .count(),
            1
        );

        Executor::new().execute(&program).unwrap();
        assert_eq!(
            fs::read_to_string(file).unwrap(),
            "first\nsecond\nthird\nbase\n"
        );
    }

    #[test]
    fn relative_and_absolute_spellings_have_the_same_path_identity() {
        let relative = Path::new("identity.txt");
        let absolute = std::env::current_dir().unwrap().join(relative);

        assert_eq!(
            entry_path_identity(relative),
            entry_path_identity(&absolute)
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_identity_does_not_collapse_parent_components_across_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("symlink-parent-identity");
        let root_file = dir.path().join("a.txt");
        let other = dir.path().join("other");
        let other_file = other.join("a.txt");
        fs::create_dir_all(other.join("child")).unwrap();
        fs::write(&root_file, "root\n").unwrap();
        fs::write(&other_file, "other\n").unwrap();
        symlink(other.join("child"), dir.path().join("link")).unwrap();
        let through_symlink_parent = dir.path().join("link/../a.txt");
        assert_ne!(
            entry_path_identity(&root_file),
            entry_path_identity(&through_symlink_parent)
        );
        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&root_file, 0),
                content: "first\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(&through_symlink_parent, 0),
                content: "second\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new().evaluate(&program).unwrap();
        assert_eq!(
            plan.actions()
                .filter(|action| matches!(action, crate::plan::PlannedAction::WriteFile { .. }))
                .count(),
            2
        );
        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(root_file).unwrap(), "first\nroot\n");
        assert_eq!(fs::read_to_string(other_file).unwrap(), "second\nother\n");
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_text_edits_merge_final_symlink_and_target_content() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("final-symlink-snapshot");
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "base\n").unwrap();
        symlink(&target, &link).unwrap();
        assert_eq!(content_path_identity(&target), content_path_identity(&link));
        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&target, 0),
                content: "first\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(&link, 0),
                content: "second\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new().evaluate(&program).unwrap();
        assert_eq!(
            plan.actions()
                .filter(|action| matches!(action, crate::plan::PlannedAction::WriteFile { .. }))
                .count(),
            1
        );
        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(target).unwrap(), "first\nsecond\nbase\n");
    }

    #[cfg(unix)]
    #[test]
    fn incremental_snapshot_reads_updated_content_through_final_symlink() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("final-symlink-incremental");
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "base\n").unwrap();
        symlink(&target, &link).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&target, 0),
                content: "first\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(&link, 0),
                content: "second\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ])
        .with_mode(ProgramMode::Incremental);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(target).unwrap(), "second\nfirst\nbase\n");
    }

    #[test]
    fn incremental_snapshot_lookups_use_normalized_path_identity() {
        let dir = TestDir::new("incremental-path-alias");
        let file = dir.path().join("new.txt");
        let alias = dir.path().join("./new.txt");
        let mut program = EditProgram::new().with_mode(ProgramMode::Incremental);
        program.push(GenericModification::CreateFile {
            path: file.clone(),
            content: "base\n".into(),
            overwrite: false,
            span: None,
        });
        program.push(GenericModification::InsertLines {
            target: FileInsertion::new(alias, 0),
            content: "first\n".into(),
            create_destination_if_missing: false,
            span: None,
        });

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "first\nbase\n");
    }

    #[test]
    fn replaces_lines_in_a_file() {
        let dir = TestDir::new("replace-lines");
        let file = dir.path().join("data.txt");
        fs::write(&file, "a\nb\nc\nd\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::ReplaceRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(1, 3).unwrap()),
                ),
                content: "x\ny\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "a\nx\ny\nd\n");
    }

    #[test]
    fn deletes_lines_matching_a_regex() {
        let dir = TestDir::new("delete-match");
        let file = dir.path().join("data.txt");
        fs::write(&file, "use a;\nkeep\nuse b;\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteLinesMatching {
                target: FilePatternMatch::new(&file, r"^use "),
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "keep\n");
    }

    #[test]
    fn deletes_lines_matching_an_anchored_regex_with_crlf() {
        let dir = TestDir::new("delete-match-crlf");
        let file = dir.path().join("data.txt");
        fs::write(&file, "foo\r\nkeep\r\nfoo suffix\r\nfoo").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteLinesMatching {
                target: FilePatternMatch::new(&file, r"^foo$"),
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "keep\r\nfoo suffix\r\n");
    }

    #[test]
    fn root_level_relative_write_has_no_empty_directory_action() {
        let filename = format!(
            "smartedit-relative-plan-{}-{}.txt",
            process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateFile {
                path: PathBuf::from(&filename),
                content: "content".into(),
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new().evaluate(&program).unwrap();

        assert_eq!(plan.actions().count(), 1);
        assert!(matches!(
            plan.actions().next(),
            Some(crate::plan::PlannedAction::WriteFile { path, .. }) if path == Path::new(&filename)
        ));
    }

    #[test]
    fn text_replace_rewrites_literal_matches_across_globbed_files() {
        let dir = TestDir::new("text-replace-literal");
        let root = dir.path().join("src");
        fs::create_dir_all(&root).unwrap();
        let file_a = root.join("a.txt");
        let file_b = root.join("b.txt");
        fs::write(&file_a, "foo 1\nfoo 2\n").unwrap();
        fs::write(&file_b, "keep\nfoo 3\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::TextReplace {
                targets: PathSpec::glob(&root, "*.txt"),
                pattern: TextPattern::literal("foo"),
                replacement: "bar".into(),
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file_a).unwrap(), "bar 1\nbar 2\n");
        assert_eq!(fs::read_to_string(file_b).unwrap(), "keep\nbar 3\n");
    }

    #[test]
    fn text_replace_supports_regex_capture_groups() {
        let dir = TestDir::new("text-replace-regex");
        let file = dir.path().join("Cargo.toml");
        fs::write(&file, "name = \"old\"\nversion = \"0.1.0\"\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::TextReplace {
                targets: PathSpec::exact_file(&file),
                pattern: TextPattern::regex(r#"^(name = )"([^"]+)""#),
                replacement: "$1\"smartedit\"".into(),
                span: None,
            }
            .into(),
        ])
        .with_mode(ProgramMode::Incremental);

        Executor::new().execute(&program).unwrap();

        assert_eq!(
            fs::read_to_string(file).unwrap(),
            "name = \"smartedit\"\nversion = \"0.1.0\"\n"
        );
    }

    #[test]
    fn moves_files_selected_from_a_directory() {
        let dir = TestDir::new("move-files");
        let source_root = dir.path().join("a/b");
        let nested = source_root.join("nested");
        let destination_root = dir.path().join("c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(source_root.join("one.txt"), "one").unwrap();
        fs::write(nested.join("two.txt"), "two").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveFiles {
                sources: PathSpec::files_in_directory(&source_root),
                destination_dir: PathDestination::directory(destination_root.clone()),
                create_destination_dir: true,
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert!(!source_root.join("one.txt").exists());
        assert!(!nested.join("two.txt").exists());
        assert_eq!(
            fs::read_to_string(destination_root.join("one.txt")).unwrap(),
            "one"
        );
        assert_eq!(
            fs::read_to_string(destination_root.join("nested/two.txt")).unwrap(),
            "two"
        );
    }

    #[cfg(unix)]
    #[test]
    fn moving_a_file_preserves_executable_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDir::new("move-permissions");
        let source = dir.path().join("tool");
        let destination_root = dir.path().join("bin");
        fs::write(&source, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveFiles {
                sources: PathSpec::exact_file(&source),
                destination_dir: PathDestination::directory(&destination_root),
                create_destination_dir: true,
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new().evaluate(&program).unwrap();
        assert!(
            plan.actions()
                .any(|action| matches!(action, crate::plan::PlannedAction::MoveFile { .. }))
        );
        Executor::new().execute(&program).unwrap();

        let mode = fs::metadata(destination_root.join("tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn moving_with_overwrite_replaces_an_existing_destination() {
        let dir = TestDir::new("move-overwrite");
        let source = dir.path().join("source/item.txt");
        let destination_root = dir.path().join("destination");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        fs::write(&source, "new").unwrap();
        fs::write(destination_root.join("item.txt"), "old").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveFiles {
                sources: PathSpec::exact_file(&source),
                destination_dir: PathDestination::directory(&destination_root),
                create_destination_dir: true,
                overwrite: true,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(destination_root.join("item.txt")).unwrap(),
            "new"
        );
        assert!(fs::read_dir(destination_root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("smartedit-backup")
        }));
    }

    #[test]
    fn moving_with_overwrite_rejects_a_destination_directory() {
        let dir = TestDir::new("move-overwrite-directory");
        let source = dir.path().join("source/item.txt");
        let destination_root = dir.path().join("destination");
        let destination = destination_root.join("item.txt");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(&source, "new").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveFiles {
                sources: PathSpec::exact_file(&source),
                destination_dir: PathDestination::directory(&destination_root),
                create_destination_dir: true,
                overwrite: true,
                span: None,
            }
            .into(),
        ]);

        let error = Executor::new().evaluate(&program).unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::ExpectedFileButFoundDirectory { path } if path == destination
        ));
        assert!(source.exists());
        assert!(destination.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn moving_a_symlink_preserves_symlink_identity() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("move-symlink");
        let target = dir.path().join("target.txt");
        let source = dir.path().join("link.txt");
        let destination_root = dir.path().join("destination");
        fs::write(&target, "target").unwrap();
        symlink(&target, &source).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveFiles {
                sources: PathSpec::exact_file(&source),
                destination_dir: PathDestination::directory(&destination_root),
                create_destination_dir: true,
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        let destination = destination_root.join("link.txt");
        assert!(
            fs::symlink_metadata(destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!source.exists());
    }

    #[test]
    fn deletes_files_selected_by_glob() {
        let dir = TestDir::new("delete-glob");
        let root = dir.path().join("src");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("keep.txt"), "keep").unwrap();
        fs::write(root.join("remove.rs"), "remove").unwrap();
        fs::write(root.join("nested/also_remove.rs"), "remove").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteFiles {
                targets: PathSpec::glob(&root, "**/*.rs"),
                missing_matches_ok: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert!(root.join("keep.txt").exists());
        assert!(!root.join("remove.rs").exists());
        assert!(!root.join("nested/also_remove.rs").exists());
    }

    #[test]
    fn documented_root_relative_regex_matches_files() {
        let dir = TestDir::new("regex-root-relative");
        let root = dir.path().join("src");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.rs"), "a").unwrap();
        fs::write(root.join("module_name.rs"), "module").unwrap();
        fs::write(root.join("keep-1.rs"), "keep").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteFiles {
                targets: PathSpec::regex(&root, r"[a-z_]+\.rs"),
                missing_matches_ok: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert!(!root.join("a.rs").exists());
        assert!(!root.join("module_name.rs").exists());
        assert!(root.join("keep-1.rs").exists());
    }

    #[test]
    fn dry_run_returns_plan_without_changing_files() {
        let dir = TestDir::new("dry-run");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("dest.txt");
        fs::write(&source, "a0\na1\na2\na3\n").unwrap();
        fs::write(&destination, "d0\nd1\n").unwrap();

        let ranges = RangeSet::single(TextRange::new(1, 3).unwrap());
        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(&source, ranges),
                destination: FileInsertion::new(&destination, 1),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new()
            .run(&program, ExecutionOptions { dry_run: true })
            .unwrap();

        assert_eq!(plan.actions().count(), 2);
        assert_eq!(fs::read_to_string(source).unwrap(), "a0\na1\na2\na3\n");
        assert_eq!(fs::read_to_string(destination).unwrap(), "d0\nd1\n");
    }

    #[test]
    fn snapshot_line_moves_from_same_source_are_merged_into_one_final_write() {
        let dir = TestDir::new("snapshot-source");
        let source = dir.path().join("source.txt");
        let destination_a = dir.path().join("a.txt");
        let destination_b = dir.path().join("b.txt");
        fs::write(&source, "l0\nl1\nl2\nl3\nl4\nl5\n").unwrap();
        fs::write(&destination_a, "A\n").unwrap();
        fs::write(&destination_b, "B\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(
                    &source,
                    RangeSet::single(TextRange::new(0, 2).unwrap()),
                ),
                destination: FileInsertion::new(&destination_a, 1),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(
                    &source,
                    RangeSet::single(TextRange::new(3, 5).unwrap()),
                ),
                destination: FileInsertion::new(&destination_b, 1),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(source).unwrap(), "l2\nl5\n");
        assert_eq!(fs::read_to_string(destination_a).unwrap(), "A\nl0\nl1\n");
        assert_eq!(fs::read_to_string(destination_b).unwrap(), "B\nl3\nl4\n");
    }

    #[test]
    fn snapshot_line_moves_into_same_destination_preserve_modification_order() {
        let dir = TestDir::new("snapshot-destination");
        let source_a = dir.path().join("a.txt");
        let source_b = dir.path().join("b.txt");
        let destination = dir.path().join("dest.txt");
        fs::write(&source_a, "a0\na1\n").unwrap();
        fs::write(&source_b, "b0\nb1\n").unwrap();
        fs::write(&destination, "d0\nd1\n").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(
                    &source_a,
                    RangeSet::single(TextRange::new(0, 2).unwrap()),
                ),
                destination: FileInsertion::new(&destination, 1),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(
                    &source_b,
                    RangeSet::single(TextRange::new(0, 2).unwrap()),
                ),
                destination: FileInsertion::new(&destination, 1),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(source_a).unwrap(), "");
        assert_eq!(fs::read_to_string(source_b).unwrap(), "");
        assert_eq!(
            fs::read_to_string(destination).unwrap(),
            "d0\na0\na1\nb0\nb1\nd1\n"
        );
    }

    #[test]
    fn apply_starts_a_new_snapshot_scope() {
        let dir = TestDir::new("apply-scope");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("dest.txt");
        fs::write(&source, "a\nb\nc\nd\n").unwrap();
        fs::write(&destination, "").unwrap();

        let mut program = EditProgram::new();
        program.push(GenericModification::MoveRanges {
            source: FileRangeSelection::new(
                &source,
                RangeSet::single(TextRange::new(0, 1).unwrap()),
            ),
            destination: FileInsertion::new(&destination, 0),
            create_destination_if_missing: false,
            span: None,
        });
        program.apply();
        program.push(GenericModification::MoveRanges {
            source: FileRangeSelection::new(
                &source,
                RangeSet::single(TextRange::new(1, 2).unwrap()),
            ),
            destination: FileInsertion::new(&destination, 1),
            create_destination_if_missing: false,
            span: None,
        });

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(source).unwrap(), "b\nd\n");
        assert_eq!(fs::read_to_string(destination).unwrap(), "a\nc\n");
    }

    #[test]
    fn incremental_mode_applies_each_modification_sequentially() {
        let dir = TestDir::new("incremental-mode");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("dest.txt");
        fs::write(&source, "a\nb\nc\nd\n").unwrap();
        fs::write(&destination, "").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(
                    &source,
                    RangeSet::single(TextRange::new(0, 1).unwrap()),
                ),
                destination: FileInsertion::new(&destination, 0),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(
                    &source,
                    RangeSet::single(TextRange::new(1, 2).unwrap()),
                ),
                destination: FileInsertion::new(&destination, 1),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ])
        .with_mode(ProgramMode::Incremental);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(source).unwrap(), "b\nd\n");
        assert_eq!(fs::read_to_string(destination).unwrap(), "a\nc\n");
    }

    #[test]
    fn conflicting_modifications_to_the_same_destination_are_rejected() {
        let dir = TestDir::new("conflict");
        let source_root = dir.path().join("src");
        let destination_root = dir.path().join("dst");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("one.txt"), "one").unwrap();
        fs::write(source_root.join("two.txt"), "two").unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        fs::write(destination_root.join("one.txt"), "existing").unwrap();

        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveFiles {
                sources: PathSpec::exact_file(source_root.join("one.txt")),
                destination_dir: PathDestination::directory(&destination_root),
                create_destination_dir: true,
                overwrite: false,
                span: None,
            }
            .into(),
            GenericModification::CreateFile {
                path: destination_root.join("one.txt"),
                content: "other".into(),
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        let error = Executor::new().evaluate(&program).unwrap_err();
        assert!(matches!(
            error,
            SmartEditError::FileAlreadyExists { path } if path == destination_root.join("one.txt")
        ));
    }

    #[test]
    fn conflicting_action_targets_are_detected_through_lexical_aliases() {
        let dir = TestDir::new("alias-conflict");
        let path = dir.path().join("new.txt");
        let alias = dir.path().join("./new.txt");
        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateFile {
                path: path.clone(),
                content: "first".into(),
                overwrite: false,
                span: None,
            }
            .into(),
            GenericModification::CreateFile {
                path: alias,
                content: "second".into(),
                overwrite: false,
                span: None,
            }
            .into(),
        ]);

        let error = Executor::new().evaluate(&program).unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::ConflictingActionTargets {
                first_modification: 0,
                second_modification: 1,
                ..
            }
        ));
    }

    #[test]
    fn file_and_descendant_directory_targets_conflict_in_either_order() {
        let dir = TestDir::new("ancestor-conflict");
        for file_first in [false, true] {
            let file = GenericModification::CreateFile {
                path: dir.path().join("a"),
                content: "file".into(),
                overwrite: true,
                span: None,
            }
            .into();
            let descendant = GenericModification::CreateDirectory {
                path: dir.path().join("a/b"),
                span: None,
            }
            .into();
            let modifications = if file_first {
                vec![file, descendant]
            } else {
                vec![descendant, file]
            };

            let error = Executor::new()
                .evaluate(&EditProgram::from_modifications(modifications))
                .unwrap_err();
            assert!(matches!(
                error,
                SmartEditError::ConflictingActionTargets { .. }
            ));
        }
    }

    #[test]
    fn create_directory_and_overwrite_file_validate_existing_target_kind() {
        let dir = TestDir::new("existing-target-kind");
        let existing_file = dir.path().join("file");
        let existing_directory = dir.path().join("directory");
        fs::write(&existing_file, "file").unwrap();
        fs::create_dir(&existing_directory).unwrap();

        let directory_error = Executor::new()
            .evaluate(&EditProgram::from_modifications(vec![
                GenericModification::CreateDirectory {
                    path: existing_file.clone(),
                    span: None,
                }
                .into(),
            ]))
            .unwrap_err();
        assert!(matches!(
            directory_error,
            SmartEditError::ExpectedDirectoryButFoundFile { path } if path == existing_file
        ));

        let file_error = Executor::new()
            .evaluate(&EditProgram::from_modifications(vec![
                GenericModification::CreateFile {
                    path: existing_directory.clone(),
                    content: "file".into(),
                    overwrite: true,
                    span: None,
                }
                .into(),
            ]))
            .unwrap_err();
        assert!(matches!(
            file_error,
            SmartEditError::ExpectedFileButFoundDirectory { path } if path == existing_directory
        ));
    }

    #[test]
    fn snapshot_rejects_overlapping_replacements() {
        let dir = TestDir::new("overlapping-replacements");
        let file = dir.path().join("data.txt");
        fs::write(&file, "zero\none\ntwo\n").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::ReplaceRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(0, 2).unwrap()),
                ),
                content: "first\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::ReplaceRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(1, 3).unwrap()),
                ),
                content: "second\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        let error = Executor::new().evaluate(&program).unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::OverlappingDestructiveEdits {
                first_modification: 0,
                second_modification: 1,
                ..
            }
        ));
    }

    #[test]
    fn snapshot_rejects_moving_the_same_source_to_two_destinations() {
        let dir = TestDir::new("duplicate-move-source");
        let source = dir.path().join("source.txt");
        let destination_a = dir.path().join("a.txt");
        let destination_b = dir.path().join("b.txt");
        fs::write(&source, "line\nkeep\n").unwrap();
        fs::write(&destination_a, "").unwrap();
        fs::write(&destination_b, "").unwrap();
        let moved_range = || RangeSet::single(TextRange::new(0, 1).unwrap());
        let program = EditProgram::from_modifications(vec![
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(&source, moved_range()),
                destination: FileInsertion::new(&destination_a, 0),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::MoveRanges {
                source: FileRangeSelection::new(&source, moved_range()),
                destination: FileInsertion::new(&destination_b, 0),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        assert!(matches!(
            Executor::new().evaluate(&program).unwrap_err(),
            SmartEditError::OverlappingDestructiveEdits { .. }
        ));
    }

    #[test]
    fn snapshot_rejects_a_delete_overlapping_a_replacement() {
        let dir = TestDir::new("delete-replace-overlap");
        let file = dir.path().join("data.txt");
        fs::write(&file, "zero\none\ntwo\n").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(0, 2).unwrap()),
                ),
                span: None,
            }
            .into(),
            GenericModification::ReplaceRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(1, 2).unwrap()),
                ),
                content: "replacement\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        assert!(matches!(
            Executor::new().evaluate(&program).unwrap_err(),
            SmartEditError::OverlappingDestructiveEdits { .. }
        ));
    }

    #[test]
    fn snapshot_allows_overlapping_deletes_and_adjacent_replacements() {
        let dir = TestDir::new("allowed-overlaps");
        let file = dir.path().join("data.txt");
        fs::write(&file, "zero\none\ntwo\nthree\n").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(0, 2).unwrap()),
                ),
                span: None,
            }
            .into(),
            GenericModification::DeleteRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(1, 3).unwrap()),
                ),
                span: None,
            }
            .into(),
            GenericModification::ReplaceRanges {
                target: FileRangeSelection::new(
                    &file,
                    RangeSet::single(TextRange::new(3, 4).unwrap()),
                ),
                content: "last\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "last\n");
    }

    #[test]
    fn creating_an_empty_missing_text_destination_still_creates_the_file() {
        let dir = TestDir::new("empty-created-destination");
        let file = dir.path().join("empty.txt");
        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&file, 0),
                content: String::new(),
                create_destination_if_missing: true,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read(&file).unwrap(), b"");
    }

    #[test]
    fn snapshot_text_edits_merge_hardlink_content_aliases() {
        let dir = TestDir::new("hardlink-content-aliases");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "base\n").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&first, 0),
                content: "first\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(&second, 0),
                content: "second\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        let plan = Executor::new().evaluate(&program).unwrap();
        assert_eq!(
            plan.actions()
                .filter(|action| matches!(action, crate::plan::PlannedAction::WriteFile { .. }))
                .count(),
            1
        );
        Executor::new().execute(&program).unwrap();
        assert_eq!(fs::read_to_string(first).unwrap(), "first\nsecond\nbase\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "first\nsecond\nbase\n");
    }

    #[test]
    fn staged_text_edits_through_hardlinks_see_the_previous_stage() {
        let dir = TestDir::new("staged-hardlink-content-aliases");
        let first = dir.path().join("a.txt");
        let second = dir.path().join("b.txt");
        fs::write(&first, "base\n").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let mut program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&first, 0),
                content: "FOO\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);
        program.apply();
        program.push(GenericModification::InsertLines {
            target: FileInsertion::new(&second, 1),
            content: "BAR\n".into(),
            create_destination_if_missing: false,
            span: None,
        });

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(first).unwrap(), "FOO\nBAR\nbase\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "FOO\nBAR\nbase\n");
    }

    #[test]
    fn incremental_text_edits_through_hardlinks_see_the_previous_edit() {
        let dir = TestDir::new("incremental-hardlink-content-aliases");
        let first = dir.path().join("a.txt");
        let second = dir.path().join("b.txt");
        fs::write(&first, "base\n").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::InsertLines {
                target: FileInsertion::new(&first, 0),
                content: "FOO\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(&second, 1),
                content: "BAR\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ])
        .with_mode(ProgramMode::Incremental);

        Executor::new().execute(&program).unwrap();

        assert_eq!(fs::read_to_string(first).unwrap(), "FOO\nBAR\nbase\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "FOO\nBAR\nbase\n");
    }

    #[test]
    fn independent_writes_to_hardlinks_are_rejected_as_conflicts() {
        let dir = TestDir::new("hardlink-write-conflict");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "base\n").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateFile {
                path: first,
                content: "first\n".into(),
                overwrite: true,
                span: None,
            }
            .into(),
            GenericModification::CreateFile {
                path: second,
                content: "second\n".into(),
                overwrite: true,
                span: None,
            }
            .into(),
        ]);

        assert!(matches!(
            Executor::new().evaluate(&program).unwrap_err(),
            SmartEditError::ConflictingActionTargets { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn deleting_a_symlink_and_writing_its_content_target_are_independent() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("symlink-entry-content-independence");
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "base\n").unwrap();
        symlink(&target, &link).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteFiles {
                targets: PathSpec::exact_file(&link),
                missing_matches_ok: false,
                span: None,
            }
            .into(),
            GenericModification::InsertLines {
                target: FileInsertion::new(&link, 0),
                content: "inserted\n".into(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();

        assert!(fs::symlink_metadata(link).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "inserted\nbase\n");
    }

    #[test]
    fn overwrite_write_rejects_an_entry_replaced_after_planning() {
        let dir = TestDir::new("write-precondition");
        let file = dir.path().join("data.txt");
        let original = dir.path().join("original.txt");
        fs::write(&file, "original\n").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::CreateFile {
                path: file.clone(),
                content: "planned\n".into(),
                overwrite: true,
                span: None,
            }
            .into(),
        ]);
        let executor = Executor::new();
        let plan = executor.evaluate(&program).unwrap();
        fs::rename(&file, &original).unwrap();
        fs::write(&file, "raced\n").unwrap();

        let error = executor.apply_plan(&plan).unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::Io {
                operation: "write file",
                ..
            }
        ));
        assert_eq!(fs::read_to_string(file).unwrap(), "raced\n");
        assert_eq!(fs::read_to_string(original).unwrap(), "original\n");
    }

    #[test]
    fn delete_rejects_an_entry_replaced_after_planning() {
        let dir = TestDir::new("delete-precondition");
        let file = dir.path().join("data.txt");
        let original = dir.path().join("original.txt");
        fs::write(&file, "original\n").unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::DeleteFiles {
                targets: PathSpec::exact_file(&file),
                missing_matches_ok: false,
                span: None,
            }
            .into(),
        ]);
        let executor = Executor::new();
        let plan = executor.evaluate(&program).unwrap();
        fs::rename(&file, &original).unwrap();
        fs::write(&file, "raced\n").unwrap();

        let error = executor.apply_plan(&plan).unwrap_err();

        assert!(matches!(
            error,
            SmartEditError::Io {
                operation: "delete file",
                ..
            }
        ));
        assert_eq!(fs::read_to_string(file).unwrap(), "raced\n");
        assert_eq!(fs::read_to_string(original).unwrap(), "original\n");
    }

    #[test]
    fn planned_file_ancestors_are_rejected_across_stages_and_incremental_steps() {
        let dir = TestDir::new("planned-file-ancestor");
        let parent = dir.path().join("parent");
        let child = parent.join("child.txt");
        for program in [
            {
                let mut program = EditProgram::from_modifications(vec![
                    GenericModification::CreateFile {
                        path: parent.clone(),
                        content: String::new(),
                        overwrite: false,
                        span: None,
                    }
                    .into(),
                ]);
                program.apply();
                program.push(GenericModification::CreateFile {
                    path: child.clone(),
                    content: String::new(),
                    overwrite: false,
                    span: None,
                });
                program
            },
            EditProgram::from_modifications(vec![
                GenericModification::CreateFile {
                    path: parent.clone(),
                    content: String::new(),
                    overwrite: false,
                    span: None,
                }
                .into(),
                GenericModification::CreateFile {
                    path: child,
                    content: String::new(),
                    overwrite: false,
                    span: None,
                }
                .into(),
            ])
            .with_mode(ProgramMode::Incremental),
        ] {
            assert!(matches!(
                Executor::new().evaluate(&program).unwrap_err(),
                SmartEditError::ExpectedDirectoryButFoundFile { .. }
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn exact_move_and_delete_support_dangling_and_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("exact-symlink-sources");
        let missing_link = dir.path().join("missing-link");
        let directory = dir.path().join("directory");
        let directory_link = dir.path().join("directory-link");
        let destination = dir.path().join("destination");
        fs::create_dir_all(&directory).unwrap();
        fs::create_dir_all(&destination).unwrap();
        symlink("missing-target", &missing_link).unwrap();
        symlink(&directory, &directory_link).unwrap();

        Executor::new()
            .execute(&EditProgram::from_modifications(vec![
                GenericModification::MoveFiles {
                    sources: PathSpec::exact_file(&missing_link),
                    destination_dir: PathDestination::directory(&destination),
                    create_destination_dir: false,
                    overwrite: false,
                    span: None,
                }
                .into(),
                GenericModification::DeleteFiles {
                    targets: PathSpec::exact_file(&directory_link),
                    missing_matches_ok: false,
                    span: None,
                }
                .into(),
            ]))
            .unwrap();

        assert_eq!(
            fs::read_link(destination.join("missing-link")).unwrap(),
            Path::new("missing-target")
        );
        assert!(directory.is_dir());
        assert!(fs::symlink_metadata(directory_link).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn glob_selects_symlink_entries_and_directory_source_follows_root_link() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("glob-symlink-sources");
        let tree = dir.path().join("tree");
        let tree_link = dir.path().join("tree-link");
        let nested = tree.join("nested.txt");
        let child_link = tree.join("child-link");
        fs::create_dir_all(&tree).unwrap();
        fs::write(&nested, "nested").unwrap();
        symlink("missing", &child_link).unwrap();
        symlink(&tree, &tree_link).unwrap();

        let glob = EditProgram::from_modifications(vec![
            GenericModification::DeleteFiles {
                targets: PathSpec::glob(&tree, "*-link"),
                missing_matches_ok: false,
                span: None,
            }
            .into(),
        ]);
        Executor::new().execute(&glob).unwrap();
        assert!(fs::symlink_metadata(&child_link).is_err());

        let plan = Executor::new()
            .evaluate(&EditProgram::from_modifications(vec![
                GenericModification::MoveFiles {
                    sources: PathSpec::files_in_directory(&tree_link),
                    destination_dir: PathDestination::directory(dir.path().join("out")),
                    create_destination_dir: true,
                    overwrite: false,
                    span: None,
                }
                .into(),
            ]))
            .unwrap();
        assert!(plan.actions().any(|action| matches!(
            action,
            crate::plan::PlannedAction::MoveFile { source, .. } if source.ends_with("nested.txt")
        )));
    }
}
