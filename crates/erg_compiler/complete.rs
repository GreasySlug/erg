//! Frontend-agnostic code completion engine.
//!
//! Given a scope [`Context`] and a (line, cursor) pair, [`complete`] produces
//! candidates for identifiers in scope and for attributes/methods after
//! `.` / `::`. It performs no I/O and knows nothing about the frontend, so it
//! can back the terminal REPL, ELS, or a Jupyter kernel (whose
//! `complete_request` maps 1:1 onto this API).
use crate::context::Context;
use crate::varinfo::VarKind;

/// A single completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    /// human-readable type of the candidate (may be truncated)
    pub typ: String,
}

/// Result of a completion query.
#[derive(Debug, Clone)]
pub struct Completion {
    /// byte offset in the line where the prefix being completed starts;
    /// frontends replace `line[start..cursor]` with a candidate name
    pub start: usize,
    pub candidates: Vec<Candidate>,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '!'
}

/// `"x = arr"` -> `"arr"`
fn trailing_ident(s: &str) -> &str {
    let mut start = s.len();
    for (i, c) in s.char_indices().rev() {
        if is_ident_char(c) {
            start = i;
        } else {
            break;
        }
    }
    &s[start..]
}

/// Generated/special names (e.g. `%v`, `<module>`) are not completion targets
fn is_presentable_name(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
}

fn display_type(typ: impl ToString) -> String {
    let s = typ.to_string();
    if s.chars().count() > 60 {
        let mut truncated = s.chars().take(60).collect::<String>();
        truncated.push('…');
        truncated
    } else {
        s
    }
}

enum Target<'l> {
    /// identifiers visible in the current scope
    Local,
    /// attributes of the receiver; `.1` is whether private members are included
    Attr(&'l str, bool),
    None,
}

/// `cursor` is a byte offset into `line` (clamped to a char boundary).
pub fn complete(ctx: &Context, line: &str, cursor: usize) -> Completion {
    let mut cursor = cursor.min(line.len());
    while !line.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let head = &line[..cursor];
    let prefix = trailing_ident(head);
    let start = cursor - prefix.len();
    let before = &head[..start];
    let target = if let Some(b) = before.strip_suffix("::") {
        match trailing_ident(b) {
            "" => Target::None,
            recv => Target::Attr(recv, true),
        }
    } else if before.ends_with("..") {
        // range operator, not an attribute access
        Target::Local
    } else if let Some(b) = before.strip_suffix('.') {
        match trailing_ident(b) {
            "" => Target::None,
            recv => Target::Attr(recv, false),
        }
    } else {
        Target::Local
    };
    let candidates = match target {
        Target::Local => local_candidates(ctx, prefix),
        Target::Attr(recv, include_private) => attr_candidates(ctx, recv, prefix, include_private),
        Target::None => vec![],
    };
    Completion { start, candidates }
}

/// (sorts user-defined names before builtins, then alphabetically)
fn finish(mut entries: Vec<(bool, Candidate)>) -> Vec<Candidate> {
    entries.sort_by(|(l_builtin, l), (r_builtin, r)| {
        l_builtin.cmp(r_builtin).then_with(|| l.name.cmp(&r.name))
    });
    let mut candidates: Vec<Candidate> = Vec::with_capacity(entries.len());
    for (_, cand) in entries {
        if candidates.iter().all(|c| c.name != cand.name) {
            candidates.push(cand);
        }
    }
    candidates
}

fn local_candidates(ctx: &Context, prefix: &str) -> Vec<Candidate> {
    let mut entries = vec![];
    for (name, vi) in ctx.dir() {
        let name = &name.inspect()[..];
        if !is_presentable_name(name) || !name.starts_with(prefix) {
            continue;
        }
        entries.push((
            matches!(vi.kind, VarKind::Builtin),
            Candidate {
                name: name.to_string(),
                typ: display_type(&vi.t),
            },
        ));
    }
    finish(entries)
}

fn attr_candidates(
    ctx: &Context,
    receiver: &str,
    prefix: &str,
    include_private: bool,
) -> Vec<Candidate> {
    let mut entries = vec![];
    for recv_ctx in ctx.get_receiver_ctxs(receiver) {
        for (name, vi) in recv_ctx.local_dir() {
            let name = &name.inspect()[..];
            if !is_presentable_name(name) || !name.starts_with(prefix) {
                continue;
            }
            if !include_private && !vi.vis.is_public() {
                continue;
            }
            entries.push((
                matches!(vi.kind, VarKind::Builtin),
                Candidate {
                    name: name.to_string(),
                    typ: display_type(&vi.t),
                },
            ));
        }
    }
    // record/instance fields are not necessarily registered in the method
    // contexts, so collect them from the receiver's type as well
    if let Some((_, vi)) = ctx.get_var_info(receiver) {
        for (field, t) in ctx.fields(&vi.t) {
            let name = &field.symbol[..];
            if !is_presentable_name(name) || !name.starts_with(prefix) {
                continue;
            }
            if !include_private && !field.vis.is_public() {
                continue;
            }
            entries.push((
                false,
                Candidate {
                    name: name.to_string(),
                    typ: display_type(&t),
                },
            ));
        }
    }
    finish(entries)
}
