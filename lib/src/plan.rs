use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationPlan {
    modification_plans: Vec<ModificationPlan>,
}

impl EvaluationPlan {
    pub fn new(modification_plans: Vec<ModificationPlan>) -> Self {
        Self { modification_plans }
    }

    pub fn modification_plans(&self) -> &[ModificationPlan] {
        &self.modification_plans
    }

    pub fn actions(&self) -> impl Iterator<Item = &PlannedAction> {
        self.modification_plans
            .iter()
            .flat_map(|plan| plan.actions())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModificationPlan {
    modification_index: usize,
    actions: Vec<PlannedAction>,
}

impl ModificationPlan {
    pub fn new(modification_index: usize, actions: Vec<PlannedAction>) -> Self {
        Self {
            modification_index,
            actions,
        }
    }

    pub fn modification_index(&self) -> usize {
        self.modification_index
    }

    pub fn actions(&self) -> &[PlannedAction] {
        &self.actions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A concrete filesystem action produced by evaluation.
///
/// `WriteFile::overwrite = false` requires exclusive creation at execution time. `MoveFile`
/// affects both `source` and `destination`, even though [`Self::target_path`] returns only the
/// destination.
pub enum PlannedAction {
    CreateDirectory {
        path: PathBuf,
    },
    WriteFile {
        path: PathBuf,
        bytes: Vec<u8>,
        overwrite: bool,
    },
    DeleteFile {
        path: PathBuf,
        missing_ok: bool,
    },
    MoveFile {
        source: PathBuf,
        destination: PathBuf,
        overwrite: bool,
    },
}

impl PlannedAction {
    /// Returns the primary target path.
    ///
    /// For [`PlannedAction::MoveFile`], this is the destination. Use [`Self::source_path`] or
    /// [`Self::affected_paths`] when the move source is also relevant.
    pub fn target_path(&self) -> &PathBuf {
        match self {
            PlannedAction::CreateDirectory { path } => path,
            PlannedAction::WriteFile { path, .. } => path,
            PlannedAction::DeleteFile { path, .. } => path,
            PlannedAction::MoveFile { destination, .. } => destination,
        }
    }

    /// Returns the source path for a move action.
    pub fn source_path(&self) -> Option<&PathBuf> {
        match self {
            PlannedAction::MoveFile { source, .. } => Some(source),
            _ => None,
        }
    }

    /// Returns every path affected by the action, in source-then-destination order for moves.
    pub fn affected_paths(&self) -> impl Iterator<Item = &PathBuf> {
        let paths = match self {
            PlannedAction::CreateDirectory { path }
            | PlannedAction::WriteFile { path, .. }
            | PlannedAction::DeleteFile { path, .. } => [Some(path), None],
            PlannedAction::MoveFile {
                source,
                destination,
                ..
            } => [Some(source), Some(destination)],
        };
        paths.into_iter().flatten()
    }
}

/// Options for evaluating and applying an edit program.
///
/// Applying a plan is best-effort: if a filesystem action fails, earlier actions are not rolled
/// back. Snapshot versus incremental *evaluation* is selected by [`crate::ProgramMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionOptions {
    pub dry_run: bool,
}

impl ExecutionOptions {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::PlannedAction;

    #[test]
    fn move_path_accessors_expose_source_and_destination() {
        let source = PathBuf::from("source.txt");
        let destination = PathBuf::from("destination.txt");
        let action = PlannedAction::MoveFile {
            source: source.clone(),
            destination: destination.clone(),
            overwrite: false,
        };

        assert_eq!(action.target_path(), &destination);
        assert_eq!(action.source_path(), Some(&source));
        assert_eq!(
            action.affected_paths().cloned().collect::<Vec<_>>(),
            vec![source, destination]
        );
    }
}
