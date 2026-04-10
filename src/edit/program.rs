use super::Modification;
use crate::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgramMode {
    #[default]
    Snapshot,
    Incremental,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditStage {
    modifications: Vec<Modification>,
    pub span: Option<Span>,
}

impl EditStage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_modifications(modifications: Vec<Modification>) -> Self {
        Self {
            modifications,
            span: None,
        }
    }

    pub fn modifications(&self) -> &[Modification] {
        &self.modifications
    }

    pub fn is_empty(&self) -> bool {
        self.modifications.is_empty()
    }

    pub fn push(&mut self, modification: impl Into<Modification>) {
        self.modifications.push(modification.into());
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditProgram {
    stages: Vec<EditStage>,
    pub mode: ProgramMode,
    pub span: Option<Span>,
}

impl EditProgram {
    pub fn new() -> Self {
        Self {
            stages: vec![EditStage::new()],
            mode: ProgramMode::Snapshot,
            span: None,
        }
    }

    pub fn from_modifications(modifications: Vec<Modification>) -> Self {
        Self {
            stages: vec![EditStage::from_modifications(modifications)],
            mode: ProgramMode::Snapshot,
            span: None,
        }
    }

    pub fn push(&mut self, modification: impl Into<Modification>) {
        if self.stages.is_empty() {
            self.stages.push(EditStage::new());
        }
        self.stages
            .last_mut()
            .expect("program should have a current stage")
            .push(modification);
    }

    pub fn apply(&mut self) {
        if self.stages.is_empty() {
            self.stages.push(EditStage::new());
            return;
        }

        if self.stages.last().is_some_and(EditStage::is_empty) {
            return;
        }

        self.stages.push(EditStage::new());
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_mode(mut self, mode: ProgramMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn stages(&self) -> &[EditStage] {
        &self.stages
    }

    pub fn modifications(&self) -> Vec<&Modification> {
        self.stages
            .iter()
            .flat_map(|stage| stage.modifications())
            .collect()
    }

    pub fn modification_count(&self) -> usize {
        self.stages
            .iter()
            .map(|stage| stage.modifications().len())
            .sum()
    }

    pub fn into_modifications(self) -> Vec<Modification> {
        self.stages
            .into_iter()
            .flat_map(|stage| stage.modifications)
            .collect()
    }
}
