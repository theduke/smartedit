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
pub enum PlannedAction {
    CreateDirectory { path: PathBuf },
    WriteFile { path: PathBuf, bytes: Vec<u8> },
    DeleteFile { path: PathBuf, missing_ok: bool },
}

impl PlannedAction {
    pub fn target_path(&self) -> &PathBuf {
        match self {
            PlannedAction::CreateDirectory { path } => path,
            PlannedAction::WriteFile { path, .. } => path,
            PlannedAction::DeleteFile { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    #[default]
    Atomic,
    Incremental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub mode: ExecutionMode,
    pub dry_run: bool,
}

impl ExecutionOptions {
    pub fn new(mode: ExecutionMode, dry_run: bool) -> Self {
        Self { mode, dry_run }
    }
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Atomic,
            dry_run: false,
        }
    }
}
