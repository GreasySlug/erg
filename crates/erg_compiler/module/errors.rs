use erg_common::pathutil::NormalizedPathBuf;
use erg_common::set::Set;
use erg_common::shared::Shared;

use crate::error::{CompileError, CompileErrors};

#[derive(Debug, Clone, Default)]
pub struct SharedCompileErrors(Shared<Set<CompileError>>);

impl SharedCompileErrors {
    pub fn new() -> Self {
        Self(Shared::new(Set::new()))
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn push(&self, error: CompileError) {
        self.0.borrow_mut().insert(error);
    }

    pub fn extend(&self, errors: CompileErrors) {
        self.0.borrow_mut().extend(errors);
    }

    pub fn take(&self) -> CompileErrors {
        self.0.borrow_mut().take_all().to_vec().into()
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn remove(&self, path: &NormalizedPathBuf) {
        self.0
            .borrow_mut()
            .retain(|e| &NormalizedPathBuf::from(e.input.path()) != path);
    }

    pub fn get(&self, path: &NormalizedPathBuf) -> CompileErrors {
        self.0
            .borrow()
            .iter()
            .filter(|e| &NormalizedPathBuf::from(e.input.path()) == path)
            .cloned()
            .collect()
    }

    /// Clone while the lock is held. Prefer this over iterating unlocked
    /// pointers into the set (see the old `raw_iter`).
    pub fn snapshot(&self) -> Vec<CompileError> {
        self.0.borrow().iter().cloned().collect()
    }
}

pub type SharedCompileWarnings = SharedCompileErrors;
