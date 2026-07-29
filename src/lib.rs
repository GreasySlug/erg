extern crate erg_common;
extern crate erg_compiler;
mod dummy;
#[cfg(feature = "pydecl")]
pub mod pydecl;
pub use dummy::{DummyVM, PackageManagerRunner};
