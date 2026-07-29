//! `erg pydecl` subcommand: generate Erg declarations (`.d.er` syntax)
//! from Python sources (`.py`) or type stubs (`.pyi`).
//!
//! The declarations are printed to stdout by default; with
//! `--output-dir <dir>`, they are written to `<dir>/<stem>.d.er` instead.

use std::path::Path;

use erg_common::config::ErgConfig;
use erg_common::traits::ExitStatus;

use erg_pydecl::{convert_py_to_decl, convert_pyi_to_decl};

const USAGE: &str = "\
USAGE:
    erg pydecl [OPTIONS] <file.py|file.pyi>

OPTIONS:
    --output-dir <dir>    write <dir>/<stem>.d.er instead of printing to stdout";

pub fn run(mut cfg: ErgConfig) -> ExitStatus {
    if cfg.input.is_repl() {
        eprintln!("{USAGE}");
        return ExitStatus::ERR1;
    }
    let src = match cfg.input.try_read() {
        Ok(src) => src,
        Err(err) => {
            eprintln!("cannot read {}: {err}", cfg.input.path().display());
            return ExitStatus::ERR1;
        }
    };
    let is_pyi = cfg.input.path().extension().is_some_and(|ext| ext == "pyi");
    let result = if is_pyi {
        convert_pyi_to_decl(&src)
    } else {
        convert_py_to_decl(&src)
    };
    match result {
        Ok(decl) => {
            if let Some(dir) = cfg.dist_dir {
                let stem = cfg
                    .input
                    .path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "out".to_string());
                let dest = Path::new(dir).join(format!("{stem}.d.er"));
                if let Err(err) = std::fs::write(&dest, decl) {
                    eprintln!("cannot write {}: {err}", dest.display());
                    return ExitStatus::ERR1;
                }
                println!("generated: {}", dest.display());
            } else {
                print!("{decl}");
            }
            ExitStatus::OK
        }
        Err(err) => {
            eprintln!("{err}");
            ExitStatus::ERR1
        }
    }
}
