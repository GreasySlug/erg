//! Lint rules, grouped by concern.
//!
//! Each submodule adds `lint_*` methods to [`crate::Linter`] via an `impl`
//! block. A rule inspects the node it cares about, pushes a warning when it
//! matches, and then recurses through [`crate::Linter::check_recursively`].
//!
//! To add a rule: implement the method here, then register it in
//! [`crate::Linter::lint`].

mod arithmetic;
mod comparison;
mod control;
mod effect;
mod numeric;
mod redundancy;
mod shadowing;
mod structure;
