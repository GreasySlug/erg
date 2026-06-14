//! Tests for the completion engine (`erg_compiler::complete`).
use erg_common::config::ErgConfig;
use erg_common::io::Input;
use erg_common::spawn::exec_new_thread;

use erg_compiler::build_hir::HIRBuilder;
use erg_compiler::complete::{complete, Completion};
use erg_compiler::context::ModuleContext;

fn build_ctx(src: &'static str) -> ModuleContext {
    let cfg = ErgConfig {
        input: Input::str(src.into()),
        ..Default::default()
    };
    let mut builder = HIRBuilder::new(cfg);
    let _ = builder.build(src.to_string(), "exec");
    builder.pop_mod_ctx().unwrap()
}

fn names(comp: &Completion) -> Vec<&str> {
    comp.candidates.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn complete_local_names() -> Result<(), ()> {
    exec_new_thread(
        || {
            let mc = build_ctx("foo = 1\nfoobar = 2\n");
            let comp = complete(&mc.context, "foo", 3);
            assert_eq!(comp.start, 0);
            let cands = names(&comp);
            assert!(cands.contains(&"foo"), "{cands:?}");
            assert!(cands.contains(&"foobar"), "{cands:?}");
            // user-defined names are sorted before builtins
            assert_eq!(cands[0], "foo");
            // prefix filtering works
            let comp = complete(&mc.context, "x = foob", 8);
            assert_eq!(comp.start, 4);
            assert_eq!(names(&comp), vec!["foobar"]);
            Ok(())
        },
        "complete_local_names",
    )
}

#[test]
fn complete_builtin_names() -> Result<(), ()> {
    exec_new_thread(
        || {
            let mc = build_ctx("x = 1\n");
            let comp = complete(&mc.context, "pri", 3);
            let cands = names(&comp);
            assert!(cands.contains(&"print!"), "{cands:?}");
            Ok(())
        },
        "complete_builtin_names",
    )
}

#[test]
fn complete_attributes() -> Result<(), ()> {
    exec_new_thread(
        || {
            let mc = build_ctx("s = \"hello\"\n");
            let comp = complete(&mc.context, "s.", 2);
            assert_eq!(comp.start, 2);
            let cands = names(&comp);
            assert!(!cands.is_empty(), "no Str attribute candidates");
            assert!(cands.contains(&"replace"), "{cands:?}");
            // prefix filtering after the dot
            let comp = complete(&mc.context, "s.rep", 5);
            assert!(names(&comp).contains(&"replace"));
            assert!(!names(&comp).contains(&"contains"));
            Ok(())
        },
        "complete_attributes",
    )
}

#[test]
fn complete_range_op_is_not_attr_access() -> Result<(), ()> {
    exec_new_thread(
        || {
            let mc = build_ctx("start = 0\n");
            // `0..st` is a range expression, not an attribute access
            let comp = complete(&mc.context, "for! 0..st", 10);
            assert!(names(&comp).contains(&"start"));
            Ok(())
        },
        "complete_range_op",
    )
}
