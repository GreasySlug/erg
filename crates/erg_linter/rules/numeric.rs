//! Rules about numeric literals.
//!
//! - `magic_number`: floats that approximate a well-known mathematical constant
//!   (analogous to `clippy::approx_constant`).

use erg_common::traits::{Locational, Stream};

use erg_compiler::hir::Expr;
use erg_compiler::ty::ValueObj;

use crate::warn::magic_number;
use crate::Linter;

impl Linter {
    /// Floats that are (approximately) well-known mathematical constants
    /// (e.g. `3.14`) are better written as named constants like `math.pi`.
    pub(crate) fn lint_magic_number(&mut self, expr: &Expr) {
        if let Expr::Literal(lit) = expr {
            if let ValueObj::Float(fl) = &lit.value {
                if let Some(name) = well_known_constant(**fl) {
                    self.warns.push(magic_number(
                        self.input(),
                        line!() as usize,
                        self.caused_by(),
                        lit.loc(),
                        name,
                    ));
                }
            }
        }
        self.check_recursively(&Self::lint_magic_number, expr);
    }
}

/// Returns the suggested named constant when `value` (approximately) equals a
/// well-known mathematical constant, otherwise `None`.
///
/// The tolerance is loose enough to catch common truncations such as `3.14`
/// (π), `2.72` (e) and `6.28` (τ), but tight enough not to flag unrelated values.
fn well_known_constant(value: f64) -> Option<&'static str> {
    const TABLE: [(f64, &str); 3] = [
        (std::f64::consts::PI, "math.pi"),
        (std::f64::consts::TAU, "math.tau"),
        (std::f64::consts::E, "math.e"),
    ];
    TABLE
        .iter()
        .find(|(constant, _)| (value - constant).abs() < 5e-3)
        .map(|(_, name)| *name)
}
