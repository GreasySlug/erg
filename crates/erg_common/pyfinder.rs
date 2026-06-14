//! Discovery of the CPython interpreter and virtual environments.
//!
//! CPythonインタプリタ・仮想環境を、OS差を吸収しつつ優先順位付きで探索する。
//!
//! 探索は独立した「Resolver（戦略）」のチェーンとして表現され、最初に成功した
//! ものが採用される。OSごとのファイルレイアウト差（`bin/python` vs
//! `Scripts\python.exe` など）はすべて [`platform`] モジュールに閉じ込めてある。
use std::env::var_os;
use std::fs::{canonicalize, File};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fn_name_full;

/// Where a Python interpreter was discovered (kept for diagnostics).
///
/// どの方法でインタプリタを発見したか（診断用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonSource {
    /// `[tool.erg.python] path = ...` in `pyproject.toml`
    Config,
    /// `$VIRTUAL_ENV` — an activated venv (covers uv / hatch / `python -m venv`)
    VirtualEnvVar,
    /// `$CONDA_PREFIX` — an activated conda / mamba environment
    Conda,
    /// a project-local venv dir (`$UV_PROJECT_ENVIRONMENT`, `./.venv`, `./venv`)
    LocalVenv,
    /// `poetry env info -p`
    Poetry,
    /// a pyenv-managed interpreter (`pyenv which`)
    Pyenv,
    /// found on `PATH` (`which` / `where`)
    SystemPath,
}

/// A discovered interpreter and how it was found.
///
/// 発見したインタプリタと、その発見元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonInterpreter {
    pub path: PathBuf,
    pub source: PythonSource,
}

impl PythonInterpreter {
    fn new(path: PathBuf, source: PythonSource) -> Self {
        Self { path, source }
    }

    /// The command/path string handed to the shell when invoking Python.
    pub fn command(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// OS-specific filesystem conventions.
///
/// This is the *only* place that branches on the target OS. Every venv-based
/// resolver funnels through [`venv_interpreter`], so adding support for a new
/// environment manager never needs another `cfg!(windows)`.
pub mod platform {
    use super::*;

    /// Interpreter paths to probe *relative to a venv/conda root*, in order.
    ///
    /// - Unix venv: `bin/python3`, `bin/python`
    /// - Windows venv: `Scripts\python.exe`
    /// - conda / pyenv-win: interpreter sits directly under the root
    #[cfg(windows)]
    const INTERPRETER_RELPATHS: &[&str] = &["Scripts\\python.exe", "python.exe"];
    #[cfg(not(windows))]
    const INTERPRETER_RELPATHS: &[&str] = &["bin/python3", "bin/python"];

    /// Given a venv / conda / pyenv *root*, return the interpreter inside it.
    ///
    /// venv・conda・pyenv の「ルート」から、中のインタプリタ実行ファイルを得る。
    pub fn venv_interpreter(root: &Path) -> Option<PathBuf> {
        for rel in INTERPRETER_RELPATHS {
            let cand = root.join(rel);
            if cand.is_file() {
                return Some(canonicalize(&cand).unwrap_or(cand));
            }
        }
        None
    }

    /// Command names to look up on `PATH`, most-specific first.
    #[cfg(windows)]
    pub fn path_command_names() -> &'static [&'static str] {
        &["python", "python3"]
    }
    #[cfg(not(windows))]
    pub fn path_command_names() -> &'static [&'static str] {
        &["python3", "python"]
    }

    /// `site-packages` directories inside a venv / conda root.
    ///
    /// - Unix: `lib/python3.X/site-packages` (the minor version varies, so scan)
    /// - Windows: `Lib\site-packages`
    #[cfg(windows)]
    pub fn venv_site_packages(root: &Path) -> Vec<PathBuf> {
        let p = root.join("Lib").join("site-packages");
        p.is_dir().then_some(p).into_iter().collect()
    }
    #[cfg(not(windows))]
    pub fn venv_site_packages(root: &Path) -> Vec<PathBuf> {
        let Ok(entries) = root.join("lib").read_dir() else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| e.path().join("site-packages"))
            .filter(|p| p.is_dir())
            .collect()
    }
}

/// Run a command through the platform shell and capture stdout on success.
fn shell_capture(command: &str) -> Option<String> {
    let out = if cfg!(windows) {
        Command::new("cmd").arg("/C").arg(command).output().ok()?
    } else {
        Command::new("sh").arg("-c").arg(command).output().ok()?
    };
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// ```toml
/// [tool.erg.python]  # or [tool.pylyzer.python]
/// path = "path/to/python"
/// ```
fn config_path_from_toml() -> Option<String> {
    use std::io::BufRead;
    let f = File::open("pyproject.toml").ok()?;
    let mut reader = std::io::BufReader::new(f);
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|i| i > 0) {
        if line.starts_with("[tool.erg.python]") || line.starts_with("[tool.pylyzer.python]") {
            line.clear();
            reader.read_line(&mut line).ok()?;
            let attr = line.split('#').next().unwrap();
            if attr.starts_with("path =") {
                return Some(
                    attr.trim_start_matches("path =")
                        .trim_matches('"')
                        .trim()
                        .to_string(),
                );
            }
        }
        line.clear();
    }
    None
}

/// Project-local venv/conda root, if any (without spawning a subprocess).
///
/// 仮想環境のルートディレクトリ（サブプロセスを起動せず分かるもの）。
/// site-packages 探索などインタプリタ以外の用途からも使えるよう公開する。
pub fn local_venv_root() -> Option<PathBuf> {
    [
        var_os("VIRTUAL_ENV").map(PathBuf::from),
        var_os("CONDA_PREFIX").map(PathBuf::from),
        var_os("UV_PROJECT_ENVIRONMENT").map(PathBuf::from),
        Some(PathBuf::from(".venv")),
        Some(PathBuf::from("venv")),
    ]
    .into_iter()
    .flatten()
    .find(|root| root.is_dir())
}

// --- Resolvers (each is a pure-ish strategy returning the first hit) ---------

fn from_config() -> Option<PythonInterpreter> {
    config_path_from_toml().map(|p| PythonInterpreter::new(PathBuf::from(p), PythonSource::Config))
}

fn from_virtual_env() -> Option<PythonInterpreter> {
    let root = var_os("VIRTUAL_ENV")?;
    platform::venv_interpreter(Path::new(&root))
        .map(|p| PythonInterpreter::new(p, PythonSource::VirtualEnvVar))
}

fn from_conda() -> Option<PythonInterpreter> {
    let root = var_os("CONDA_PREFIX")?;
    platform::venv_interpreter(Path::new(&root))
        .map(|p| PythonInterpreter::new(p, PythonSource::Conda))
}

fn from_local_venv() -> Option<PythonInterpreter> {
    [
        var_os("UV_PROJECT_ENVIRONMENT").map(PathBuf::from),
        Some(PathBuf::from(".venv")),
        Some(PathBuf::from("venv")),
    ]
    .into_iter()
    .flatten()
    .find_map(|root| platform::venv_interpreter(&root))
    .map(|p| PythonInterpreter::new(p, PythonSource::LocalVenv))
}

fn from_poetry() -> Option<PythonInterpreter> {
    let root = shell_capture("poetry env info -p")?;
    let root = Path::new(root.trim());
    platform::venv_interpreter(root).map(|p| PythonInterpreter::new(p, PythonSource::Poetry))
}

/// `pyenv which <name>` resolves the shim to the *real* interpreter path.
///
/// これによりシム（特に `-c` 非対応の pyenv-win のシム）を直接踏まずに済む。
fn from_pyenv() -> Option<PythonInterpreter> {
    for name in platform::path_command_names() {
        let Some(out) = shell_capture(&format!("pyenv which {name}")) else {
            continue;
        };
        let path = PathBuf::from(out.trim());
        if path.is_file() {
            return Some(PythonInterpreter::new(path, PythonSource::Pyenv));
        }
    }
    None
}

fn from_system_path() -> Option<PythonInterpreter> {
    let finder = if cfg!(windows) { "where" } else { "which" };
    for name in platform::path_command_names() {
        let Some(out) = shell_capture(&format!("{finder} {name}")) else {
            continue;
        };
        let first = out.lines().next().unwrap_or("").trim().replace('\r', "");
        if first.is_empty() {
            continue;
        }
        // a pyenv-win shim isn't usable via `-c`; let resolution fall through
        if cfg!(windows) && first.contains("pyenv") {
            continue;
        }
        return Some(PythonInterpreter::new(
            PathBuf::from(first),
            PythonSource::SystemPath,
        ));
    }
    None
}

/// The ordered chain of discovery strategies. Earlier entries win.
const RESOLVERS: &[fn() -> Option<PythonInterpreter>] = &[
    from_config,
    from_virtual_env,
    from_conda,
    from_local_venv,
    from_poetry,
    from_pyenv,
    from_system_path,
];

/// Locate the Python interpreter to use, trying each strategy in priority order.
///
/// 採用すべき Python インタプリタを、優先順位順に各戦略を試して探索する。
pub fn find_python() -> Result<PythonInterpreter, String> {
    RESOLVERS
        .iter()
        .find_map(|resolve| resolve())
        .ok_or_else(|| format!("{}: python not found", fn_name_full!()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, File};

    /// the per-OS venv layout is resolved through a single helper
    #[test]
    fn venv_interpreter_finds_layout() {
        let root = std::env::temp_dir().join(format!("erg_pyfinder_{}", crate::random::random()));
        let (subdir, exe) = if cfg!(windows) {
            ("Scripts", "python.exe")
        } else {
            ("bin", "python3")
        };
        create_dir_all(root.join(subdir)).unwrap();
        File::create(root.join(subdir).join(exe)).unwrap();

        let found = platform::venv_interpreter(&root).expect("interpreter not found in venv root");
        assert_eq!(found.file_name().unwrap(), exe);

        std::fs::remove_dir_all(&root).ok();
    }

    /// discovery must never panic regardless of the host environment
    #[test]
    fn discovery_is_total() {
        let _ = find_python();
        let _ = local_venv_root();
    }
}
