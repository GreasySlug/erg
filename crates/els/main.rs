mod call_hierarchy;
mod channels;
mod code_action;
mod code_lens;
mod command;
mod completion;
mod definition;
mod diagnostics;
mod diff;
mod doc_highlight;
mod doc_link;
mod file_cache;
mod folding_range;
mod formatting;
mod hir_visitor;
mod hover;
mod implementation;
mod inlay_hint;
mod message;
mod references;
mod rename;
mod scheduler;
mod selection_range;
mod semantic;
mod server;
mod sig_help;
mod symbol;
mod type_definition;
mod type_hierarchy;
mod util;

use erg_common::config::ErgConfig;

#[cfg(unix)]
fn install_signal_handler() {
    extern "C" fn sigusr1_handler(_sig: libc::c_int) {
        // Signal handlers must be async-signal-safe.
        // libc::_exit() is safe; std::process::exit() is NOT (runs atexit handlers, may deadlock).
        unsafe {
            let msg = b"ELS: SIGUSR1 received, force exiting.\n";
            libc::write(
                libc::STDERR_FILENO,
                msg.as_ptr() as *const libc::c_void,
                msg.len(),
            );
            libc::_exit(1);
        }
    }
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            sigusr1_handler as *const () as libc::sighandler_t,
        );
    }
}

#[cfg(not(unix))]
fn install_signal_handler() {
    // No-op on non-Unix platforms
}

fn main() {
    install_signal_handler();
    let cfg = ErgConfig::default();
    let server = server::ErgLanguageServer::new(cfg, None);
    server.run();
}
