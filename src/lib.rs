extern crate erg_common;
extern crate erg_compiler;

mod dummy;
mod repl;

pub use dummy::{DummyVM, PackageManagerRunner};
#[cfg(feature = "els")]
pub use repl::ReplElsIntegration;
