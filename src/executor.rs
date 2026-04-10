use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use globset::Glob;
use regex::Regex;

use crate::edit::{
    EditProgram, GenericModification, Modification, PathDestinationKind, PathSpec, PathSpecKind,
    ProgramMode, RangeSet, TextPattern, resolve_insertion_offset, resolve_matching_line_ranges,
};
use crate::error::{Result, SmartEditError};
use crate::fs::{FileSystem, OsFileSystem};
use crate::plan::{
    EvaluationPlan, ExecutionMode, ExecutionOptions, ModificationPlan, PlannedAction,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Executor<F = OsFileSystem> {
    fs: F,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedTargetKind {
    Directory,
    File,
}

#[derive(Debug, Clone, Copy)]
struct PlannedTarget {
    kind: PlannedTargetKind,
    modification_index: usize,
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

#[derive(Debug, Clone)]
struct PendingTextFileUpdate {
    original_content: String,
    deletions: Vec<crate::edit::TextRange>,
    insertions: Vec<PendingTextInsertion>,
    first_modification_index: usize,
}

#[derive(Debug, Default)]
struct PendingTextUpdates {
    files: BTreeMap<PathBuf, PendingTextFileUpdate>,
}

#[derive(Debug, Clone)]
enum SnapshotEntry {
    File(Vec<u8>),
    Directory,
    Missing,
}

#[derive(Debug, Default, Clone)]
struct SnapshotState {
    entries: BTreeMap<PathBuf, SnapshotEntry>,
}

impl SnapshotState {
    fn apply_action(&mut self, action: &PlannedAction) {
        match action {
            PlannedAction::CreateDirectory { path } => {
                self.ensure_parent_directories(path);
                self.entries.insert(path.clone(), SnapshotEntry::Directory);
            }
            PlannedAction::WriteFile { path, bytes } => {
                self.ensure_parent_directories(path);
                self.entries
                    .insert(path.clone(), SnapshotEntry::File(bytes.clone()));
            }
            PlannedAction::DeleteFile { path, .. } => {
                self.entries.insert(path.clone(), SnapshotEntry::Missing);
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
                .entry(parent.to_path_buf())
                .or_insert(SnapshotEntry::Directory);
            current = parent.parent();
        }
    }

    fn get(&self, path: &Path) -> Option<&SnapshotEntry> {
        self.entries.get(path)
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

        match options.mode {
            ExecutionMode::Atomic => self.apply_plan(&plan)?,
            ExecutionMode::Incremental => self.apply_plan_incrementally(&plan)?,
        }

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
                    self.plan_move_ranges_into(
                        modification_index,
                        source.path.as_path(),
                        &source.ranges,
                        destination.path.as_path(),
                        destination.offset,
                        *create_destination_if_missing,
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
                    self.plan_insert_lines_into(
                        modification_index,
                        target.path.as_path(),
                        target.offset,
                        content,
                        *create_destination_if_missing,
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
                Ok(vec![PlannedAction::CreateDirectory { path: path.clone() }])
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
        if !overwrite && self.snapshot_exists(snapshot, path)? {
            return Err(SmartEditError::FileAlreadyExists {
                path: path.to_path_buf(),
            });
        }

        let mut actions = self.parent_directory_actions(path, true, snapshot)?;
        actions.push(PlannedAction::WriteFile {
            path: path.to_path_buf(),
            bytes: content.as_bytes().to_vec(),
        });
        Ok(actions)
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

        Ok(matches
            .into_iter()
            .map(|matched| PlannedAction::DeleteFile {
                path: matched.path,
                missing_ok: false,
            })
            .collect())
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
            let bytes = self.snapshot_read_bytes(snapshot, matched.path.as_path())?;
            let destination_path = destination_dir.join(&matched.relative_path);
            if destination_path == matched.path {
                continue;
            }

            if !overwrite && self.snapshot_exists(snapshot, destination_path.as_path())? {
                return Err(SmartEditError::FileAlreadyExists {
                    path: destination_path,
                });
            }

            actions.extend(self.parent_directory_actions(
                destination_path.as_path(),
                create_destination_dir,
                snapshot,
            )?);
            actions.push(PlannedAction::WriteFile {
                path: destination_path,
                bytes,
            });
            actions.push(PlannedAction::DeleteFile {
                path: matched.path,
                missing_ok: false,
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
        update.deletions.extend(deletions);
        Ok(())
    }

    fn plan_move_ranges_into(
        &self,
        modification_index: usize,
        source_path: &Path,
        ranges: &RangeSet,
        destination_path: &Path,
        destination_offset: usize,
        create_destination_if_missing: bool,
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
            update.deletions.extend(resolved_ranges);
        }

        let destination_update = self.get_or_load_text_update(
            modification_index,
            destination_path,
            create_destination_if_missing,
            pending,
            snapshot,
        )?;
        let destination_offset = resolve_insertion_offset(
            destination_path,
            &destination_update.original_content,
            destination_offset,
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
        destination_path: &Path,
        destination_offset: usize,
        content: &str,
        create_destination_if_missing: bool,
        pending: &mut PendingTextUpdates,
        snapshot: &SnapshotState,
    ) -> Result<()> {
        let destination_update = self.get_or_load_text_update(
            modification_index,
            destination_path,
            create_destination_if_missing,
            pending,
            snapshot,
        )?;
        let destination_offset = resolve_insertion_offset(
            destination_path,
            &destination_update.original_content,
            destination_offset,
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
        update.deletions.extend(deletions);
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
            matcher.is_match(line.trim_end_matches('\n'))
        });
        update.deletions.extend(deletions);
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
                update.deletions.push(range);
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
        if !pending.files.contains_key(path) {
            let original_content = if self.snapshot_exists(snapshot, path)? {
                self.snapshot_read_text(snapshot, path)?
            } else if create_if_missing {
                String::new()
            } else {
                return Err(SmartEditError::MissingFile {
                    path: path.to_path_buf(),
                });
            };

            pending.files.insert(
                path.to_path_buf(),
                PendingTextFileUpdate {
                    original_content,
                    deletions: Vec::new(),
                    insertions: Vec::new(),
                    first_modification_index: modification_index,
                },
            );
        }

        let update = pending
            .files
            .get_mut(path)
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

        for (path, update) in pending.files {
            let updated = self.render_pending_text_update(path.as_path(), &update)?;
            if updated == update.original_content {
                continue;
            }

            let mut actions = self.parent_directory_actions(path.as_path(), true, snapshot)?;
            actions.push(PlannedAction::WriteFile {
                path: path.clone(),
                bytes: updated.into_bytes(),
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
        let merged_deletions =
            self.merge_deletions(path, &update.original_content, &update.deletions)?;
        let mut insertions = update.insertions.clone();
        insertions.sort_by(|left, right| {
            left.offset
                .cmp(&right.offset)
                .then(left.order.cmp(&right.order))
        });

        for insertion in &insertions {
            if let Some(range) = merged_deletions
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
            if let Some(last) = coalesced.last_mut() {
                if range.start <= last.end {
                    last.end = last.end.max(range.end);
                    continue;
                }
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
        if self.snapshot_is_dir(snapshot, path)? {
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
        if self.snapshot_is_file(snapshot, root)? {
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
        if self.snapshot_is_file(snapshot, root)? {
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
        if self.snapshot_is_file(snapshot, root)? {
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
            let path = action.target_path().clone();
            let kind = match action {
                PlannedAction::CreateDirectory { .. } => PlannedTargetKind::Directory,
                PlannedAction::WriteFile { .. } | PlannedAction::DeleteFile { .. } => {
                    PlannedTargetKind::File
                }
            };

            if let Some(existing) = targets.get(&path) {
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

            targets.insert(
                path,
                PlannedTarget {
                    kind,
                    modification_index,
                },
            );
        }

        Ok(())
    }

    fn apply_plan(&self, plan: &EvaluationPlan) -> Result<()> {
        for action in plan.actions() {
            self.apply_action(action)?;
        }

        Ok(())
    }

    fn apply_plan_incrementally(&self, plan: &EvaluationPlan) -> Result<()> {
        for modification_plan in plan.modification_plans() {
            for action in modification_plan.actions() {
                self.apply_action(action)?;
            }
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
            PlannedAction::WriteFile { path, bytes } => {
                if let Some(parent) = path.parent() {
                    self.fs
                        .create_dir_all(parent)
                        .map_err(|source| SmartEditError::Io {
                            operation: "create directory",
                            path: parent.to_path_buf(),
                            source,
                        })?;
                }
                self.fs
                    .write_bytes(path, bytes)
                    .map_err(|source| SmartEditError::Io {
                        operation: "write file",
                        path: path.clone(),
                        source,
                    })
            }
            PlannedAction::DeleteFile { path, missing_ok } => match self.fs.remove_file(path) {
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
            Some(SnapshotEntry::File(_)) | Some(SnapshotEntry::Directory) => Ok(true),
            Some(SnapshotEntry::Missing) => Ok(false),
            None => self.exists(path),
        }
    }

    fn snapshot_is_file(&self, snapshot: &SnapshotState, path: &Path) -> Result<bool> {
        match snapshot.get(path) {
            Some(SnapshotEntry::File(_)) => Ok(true),
            Some(SnapshotEntry::Directory) | Some(SnapshotEntry::Missing) => Ok(false),
            None => self.is_file(path),
        }
    }

    fn snapshot_is_dir(&self, snapshot: &SnapshotState, path: &Path) -> Result<bool> {
        match snapshot.get(path) {
            Some(SnapshotEntry::Directory) => Ok(true),
            Some(SnapshotEntry::File(_)) | Some(SnapshotEntry::Missing) => Ok(false),
            None => self.is_dir(path),
        }
    }

    fn snapshot_read_bytes(&self, snapshot: &SnapshotState, path: &Path) -> Result<Vec<u8>> {
        match snapshot.get(path) {
            Some(SnapshotEntry::File(bytes)) => Ok(bytes.clone()),
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
        if self.snapshot_is_file(snapshot, root)? {
            return Err(SmartEditError::ExpectedDirectoryButFoundFile {
                path: root.to_path_buf(),
            });
        }

        let mut files = BTreeSet::new();
        if self.exists(root)? && self.is_dir(root)? {
            for path in self.list_files(root, recursive)? {
                files.insert(path);
            }
        }

        for (path, entry) in &snapshot.entries {
            if !path.starts_with(root) {
                continue;
            }
            if !recursive {
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                if relative.components().count() != 1 {
                    continue;
                }
            }

            match entry {
                SnapshotEntry::File(_) => {
                    files.insert(path.clone());
                }
                SnapshotEntry::Missing => {
                    files.remove(path);
                }
                SnapshotEntry::Directory => {}
            }
        }

        Ok(files.into_iter().collect())
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
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::edit::{
        EditProgram, FileInsertion, FilePatternMatch, FileRangeSelection, GenericModification,
        PathDestination, PathSpec, ProgramMode, RangeSet, TextPattern, TextRange,
    };
    use crate::error::SmartEditError;
    use crate::plan::ExecutionOptions;

    use super::Executor;

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
            .run(
                &program,
                ExecutionOptions {
                    dry_run: true,
                    ..ExecutionOptions::default()
                },
            )
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
}
