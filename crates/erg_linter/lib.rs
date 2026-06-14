mod rules;
mod traverse;
mod warn;

use erg_common::config::ErgConfig;
use erg_common::error::MultiErrorDisplay;
use erg_common::io::Input;
use erg_common::log;
use erg_common::traits::{ExitStatus, New, Runnable, Stream};

use erg_compiler::artifact::{Buildable, ErrorArtifact};
use erg_compiler::build_package::PackageBuilder;
use erg_compiler::error::{CompileError, CompileErrors, CompileWarnings};
use erg_compiler::hir::HIR;
use erg_compiler::module::SharedCompilerResource;

#[derive(Debug)]
pub struct Linter {
    pub cfg: ErgConfig,
    builder: PackageBuilder,
    warns: CompileWarnings,
}

impl Default for Linter {
    fn default() -> Self {
        Self::new(ErgConfig::default())
    }
}

impl New for Linter {
    fn new(cfg: ErgConfig) -> Self {
        let shared = SharedCompilerResource::new(cfg.copy());
        Self {
            builder: PackageBuilder::new(cfg.copy(), shared),
            cfg,
            warns: CompileWarnings::empty(),
        }
    }
}

impl Runnable for Linter {
    type Err = CompileError;
    type Errs = CompileErrors;
    const NAME: &'static str = "Erg linter";

    #[inline]
    fn cfg(&self) -> &ErgConfig {
        &self.cfg
    }
    #[inline]
    fn cfg_mut(&mut self) -> &mut ErgConfig {
        &mut self.cfg
    }

    #[inline]
    fn finish(&mut self) {}

    fn initialize(&mut self) {
        self.builder.initialize();
        self.warns.clear();
    }

    fn clear(&mut self) {
        self.builder.clear();
        self.warns.clear();
    }

    fn set_input(&mut self, input: erg_common::io::Input) {
        self.cfg.input = input;
        self.builder.set_input(self.cfg.input.clone());
    }

    fn exec(&mut self) -> Result<ExitStatus, Self::Errs> {
        let warns = self.lint_module().map_err(|eart| {
            eart.warns.write_all_stderr();
            eart.errors
        })?;
        warns.write_all_stderr();
        Ok(ExitStatus::compile_passed(warns.len()))
    }

    fn eval(&mut self, src: String) -> Result<String, CompileErrors> {
        let warns = self.lint_from_string(src).map_err(|eart| {
            eart.warns.write_all_stderr();
            eart.errors
        })?;
        warns.write_all_stderr();
        Ok("OK".to_string())
    }

    fn completeness_checker(&self) -> Option<erg_common::stdin::CompletenessChecker> {
        Some(Box::new(erg_parser::parse::check_code_completeness))
    }
}

impl Linter {
    pub fn new(cfg: ErgConfig) -> Self {
        New::new(cfg)
    }

    pub(crate) fn caused_by(&self) -> String {
        self.builder.get_context().unwrap().context.caused_by()
    }

    pub(crate) fn input(&self) -> Input {
        self.builder.input().clone()
    }

    /// Whether `name` resolves to a built-in in the module scope.
    ///
    /// `get_var_info` falls back to the builtins context, so a name that is not
    /// locally shadowed but exists as a builtin returns its builtin `VarInfo`.
    pub(crate) fn is_builtin_name(&self, name: &str) -> bool {
        self.builder
            .get_context()
            .and_then(|mc| mc.context.get_var_info(name))
            .is_some_and(|(_, vi)| vi.kind.is_builtin())
    }

    pub fn lint_module(&mut self) -> Result<CompileWarnings, ErrorArtifact> {
        let src = self.input().read();
        self.lint_from_string(src)
    }

    pub fn lint_from_string(&mut self, src: String) -> Result<CompileWarnings, ErrorArtifact> {
        let art = self.builder.build(src, "exec")?;
        self.warns.extend(art.warns);
        let warns = self.lint(&art.object);
        Ok(warns)
    }

    /// Runs every lint rule over each top-level chunk of the HIR.
    ///
    /// This is the single place that enumerates the active rules; the rules
    /// themselves live under [`rules`], and the shared HIR traversal is in
    /// [`traverse`].
    pub fn lint(&mut self, hir: &HIR) -> CompileWarnings {
        log!(info "Start linting");
        for chunk in hir.module.iter() {
            self.lint_too_many_params(chunk);
            self.lint_bool_comparison(chunk);
            self.lint_too_many_instance_attributes(chunk);
            self.lint_tautology(chunk);
            self.lint_magic_number(chunk);
            self.lint_double_negation(chunk);
            self.lint_arithmetic(chunk);
            self.lint_redundant_if(chunk);
            self.lint_absurd_comparison(chunk);
            self.lint_builtin_shadowing(chunk);
            self.lint_unreachable(chunk);
            self.lint_effect_free_proc(chunk);
        }
        // top-level chunks form a block too, so check it for unreachable code
        self.check_chunks(hir.module.iter());
        log!(info "Finished linting");
        self.warns.take()
    }
}
