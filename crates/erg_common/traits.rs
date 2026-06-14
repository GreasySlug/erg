//! defines common traits used in the compiler.
//!
//! コンパイラ等で汎用的に使われるトレイトを定義する
use std::collections::vec_deque;
use std::collections::VecDeque;
use std::env::consts::{ARCH, OS};
use std::io::{BufWriter, Write};
use std::process;
use std::slice::{Iter, IterMut};

use crate::config::ErgConfig;
use crate::consts::{BUILD_DATE, GIT_HASH_SHORT, SEMVER};
use crate::error::{ErrorDisplay, Location, MultiErrorDisplay};
use crate::io::Input;
use crate::{addr_eq, log};

pub trait DequeStream<T>: Sized {
    fn payload(self) -> VecDeque<T>;
    fn ref_payload(&self) -> &VecDeque<T>;
    fn ref_mut_payload(&mut self) -> &mut VecDeque<T>;

    #[inline]
    fn is_empty(&self) -> bool {
        self.ref_payload().is_empty()
    }

    #[inline]
    fn push(&mut self, elem: T) {
        self.ref_mut_payload().push_back(elem);
    }

    #[inline]
    fn push_front(&mut self, elem: T) {
        self.ref_mut_payload().push_front(elem);
    }

    fn pop_front(&mut self) -> Option<T> {
        self.ref_mut_payload().pop_front()
    }

    #[inline]
    fn get(&self, idx: usize) -> Option<&T> {
        self.ref_payload().get(idx)
    }

    #[inline]
    fn first(&self) -> Option<&T> {
        self.ref_payload().front()
    }

    #[inline]
    fn last(&self) -> Option<&T> {
        self.ref_payload().back()
    }

    #[inline]
    fn iter(&self) -> vec_deque::Iter<'_, T> {
        self.ref_payload().iter()
    }

    #[inline]
    fn len(&self) -> usize {
        self.ref_payload().len()
    }
}

#[macro_export]
macro_rules! impl_displayable_deque_stream_for_wrapper {
    ($Strc: ident, $Inner: ident) => {
        impl $Strc {
            pub const fn new(v: VecDeque<$Inner>) -> $Strc {
                $Strc(v)
            }
            pub fn empty() -> $Strc {
                $Strc(VecDeque::new())
            }
            #[inline]
            pub fn with_capacity(capacity: usize) -> $Strc {
                $Strc(VecDeque::with_capacity(capacity))
            }
        }

        impl Default for $Strc {
            #[inline]
            fn default() -> $Strc {
                $Strc::with_capacity(0)
            }
        }

        impl std::ops::Index<usize> for $Strc {
            type Output = $Inner;
            fn index(&self, idx: usize) -> &Self::Output {
                erg_common::traits::DequeStream::get(self, idx).unwrap()
            }
        }

        impl From<$Strc> for VecDeque<$Inner> {
            fn from(item: $Strc) -> VecDeque<$Inner> {
                item.payload()
            }
        }

        impl std::fmt::Display for $Strc {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "[{}]",
                    erg_common::fmt_iter(self.iter()).replace("\n", "\\n")
                )
            }
        }

        impl IntoIterator for $Strc {
            type Item = $Inner;
            type IntoIter = std::collections::vec_deque::IntoIter<Self::Item>;
            fn into_iter(self) -> Self::IntoIter {
                self.payload().into_iter()
            }
        }

        impl FromIterator<$Inner> for $Strc {
            fn from_iter<I: IntoIterator<Item = $Inner>>(iter: I) -> Self {
                $Strc(iter.into_iter().collect())
            }
        }

        impl $crate::traits::DequeStream<$Inner> for $Strc {
            #[inline]
            fn payload(self) -> VecDeque<$Inner> {
                self.0
            }
            #[inline]
            fn ref_payload(&self) -> &VecDeque<$Inner> {
                &self.0
            }
            #[inline]
            fn ref_mut_payload(&mut self) -> &mut VecDeque<$Inner> {
                &mut self.0
            }
        }
    };
}

pub trait Stream<T>: Sized {
    fn payload(self) -> Vec<T>;
    fn ref_payload(&self) -> &Vec<T>;
    fn ref_mut_payload(&mut self) -> &mut Vec<T>;

    #[inline]
    fn clear(&mut self) {
        self.ref_mut_payload().clear();
    }

    #[inline]
    fn len(&self) -> usize {
        self.ref_payload().len()
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Vec<T>>() + std::mem::size_of::<T>() * self.ref_payload().capacity()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.ref_payload().is_empty()
    }

    #[inline]
    fn insert(&mut self, idx: usize, elem: T) {
        self.ref_mut_payload().insert(idx, elem);
    }

    #[inline]
    fn remove(&mut self, idx: usize) -> T {
        self.ref_mut_payload().remove(idx)
    }

    #[inline]
    fn push(&mut self, elem: T) {
        self.ref_mut_payload().push(elem);
    }

    fn append<S: Stream<T>>(&mut self, s: &mut S) {
        self.ref_mut_payload().append(s.ref_mut_payload());
    }

    #[inline]
    fn pop(&mut self) -> Option<T> {
        self.ref_mut_payload().pop()
    }

    fn lpop(&mut self) -> Option<T> {
        let len = self.len();
        if len == 0 {
            None
        } else {
            Some(self.ref_mut_payload().remove(0))
        }
    }

    #[inline]
    fn get(&self, idx: usize) -> Option<&T> {
        self.ref_payload().get(idx)
    }

    #[inline]
    fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        self.ref_mut_payload().get_mut(idx)
    }

    #[inline]
    fn first(&self) -> Option<&T> {
        self.ref_payload().first()
    }

    #[inline]
    fn first_mut(&mut self) -> Option<&mut T> {
        self.ref_mut_payload().first_mut()
    }

    #[inline]
    fn last(&self) -> Option<&T> {
        self.ref_payload().last()
    }

    #[inline]
    fn last_mut(&mut self) -> Option<&mut T> {
        self.ref_mut_payload().last_mut()
    }

    #[inline]
    fn iter(&self) -> Iter<'_, T> {
        self.ref_payload().iter()
    }

    #[inline]
    fn iter_mut(&mut self) -> IterMut<'_, T> {
        self.ref_mut_payload().iter_mut()
    }

    #[inline]
    fn take_all(&mut self) -> Vec<T> {
        self.ref_mut_payload().drain(..).collect()
    }

    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = T>,
    {
        self.ref_mut_payload().extend(iter);
    }

    fn split_off(&mut self, at: usize) -> Vec<T> {
        self.ref_mut_payload().split_off(at)
    }

    /// Remove all elements that don't satisfy the predicate.
    fn retain(&mut self, f: impl FnMut(&T) -> bool) {
        self.ref_mut_payload().retain(f);
    }

    fn concat(mut self, other: Self) -> Self {
        self.extend(other.payload());
        self
    }
}

#[macro_export]
macro_rules! impl_displayable_stream_for_wrapper {
    ($Strc: ident, $Inner: ident) => {
        impl $Strc {
            pub const fn new(v: Vec<$Inner>) -> $Strc {
                $Strc(v)
            }
            #[inline]
            pub fn empty() -> $Strc {
                $Strc(Vec::with_capacity(20))
            }
        }

        impl From<Vec<$Inner>> for $Strc {
            #[inline]
            fn from(errs: Vec<$Inner>) -> Self {
                Self(errs)
            }
        }

        impl IntoIterator for $Strc {
            type Item = $Inner;
            type IntoIter = std::vec::IntoIter<Self::Item>;
            fn into_iter(self) -> Self::IntoIter {
                self.payload().into_iter()
            }
        }

        impl FromIterator<$Inner> for $Strc {
            fn from_iter<I: IntoIterator<Item = $Inner>>(iter: I) -> Self {
                $Strc(iter.into_iter().collect())
            }
        }

        impl std::fmt::Display for $Strc {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "[{}]",
                    erg_common::fmt_iter(self.iter()).replace("\n", "\\n")
                )
            }
        }

        impl Default for $Strc {
            #[inline]
            fn default() -> Self {
                Self::empty()
            }
        }

        impl std::ops::Index<usize> for $Strc {
            type Output = $Inner;
            fn index(&self, idx: usize) -> &Self::Output {
                erg_common::traits::Stream::get(self, idx).unwrap()
            }
        }

        impl erg_common::traits::Stream<$Inner> for $Strc {
            #[inline]
            fn payload(self) -> Vec<$Inner> {
                self.0
            }
            #[inline]
            fn ref_payload(&self) -> &Vec<$Inner> {
                &self.0
            }
            #[inline]
            fn ref_mut_payload(&mut self) -> &mut Vec<$Inner> {
                &mut self.0
            }
        }
    };
}

#[macro_export]
macro_rules! impl_stream {
    ($Strc: ident, $Inner: ident, $field: ident) => {
        impl $crate::traits::Stream<$Inner> for $Strc {
            #[inline]
            fn payload(self) -> Vec<$Inner> {
                self.$field
            }
            #[inline]
            fn ref_payload(&self) -> &Vec<$Inner> {
                &self.$field
            }
            #[inline]
            fn ref_mut_payload(&mut self) -> &mut Vec<$Inner> {
                &mut self.$field
            }
        }

        impl std::ops::Index<usize> for $Strc {
            type Output = $Inner;
            fn index(&self, idx: usize) -> &Self::Output {
                erg_common::traits::Stream::get(self, idx).unwrap()
            }
        }

        impl std::ops::IndexMut<usize> for $Strc {
            fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
                erg_common::traits::Stream::get_mut(self, idx).unwrap()
            }
        }

        impl From<$Strc> for Vec<$Inner> {
            fn from(item: $Strc) -> Vec<$Inner> {
                item.payload()
            }
        }

        impl IntoIterator for $Strc {
            type Item = $Inner;
            type IntoIter = std::vec::IntoIter<Self::Item>;
            fn into_iter(self) -> Self::IntoIter {
                self.payload().into_iter()
            }
        }

        impl $Strc {
            pub fn iter(&self) -> std::slice::Iter<'_, $Inner> {
                self.ref_payload().iter()
            }
            pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, $Inner> {
                self.ref_mut_payload().iter_mut()
            }
        }
    };
    ($Strc: ident, $Inner: ident) => {
        impl $Strc {
            pub const fn new(v: Vec<$Inner>) -> $Strc {
                $Strc(v)
            }
            pub const fn empty() -> $Strc {
                $Strc(Vec::new())
            }
            #[inline]
            pub fn with_capacity(capacity: usize) -> $Strc {
                $Strc(Vec::with_capacity(capacity))
            }
        }

        impl Default for $Strc {
            #[inline]
            fn default() -> $Strc {
                $Strc::with_capacity(0)
            }
        }

        impl std::ops::Index<usize> for $Strc {
            type Output = $Inner;
            fn index(&self, idx: usize) -> &Self::Output {
                erg_common::traits::Stream::get(self, idx).unwrap()
            }
        }

        impl From<$Strc> for Vec<$Inner> {
            fn from(item: $Strc) -> Vec<$Inner> {
                item.payload()
            }
        }

        impl IntoIterator for $Strc {
            type Item = $Inner;
            type IntoIter = std::vec::IntoIter<Self::Item>;
            fn into_iter(self) -> Self::IntoIter {
                self.payload().into_iter()
            }
        }

        impl FromIterator<$Inner> for $Strc {
            fn from_iter<I: IntoIterator<Item = $Inner>>(iter: I) -> Self {
                $Strc(iter.into_iter().collect())
            }
        }

        impl $crate::traits::Stream<$Inner> for $Strc {
            #[inline]
            fn payload(self) -> Vec<$Inner> {
                self.0
            }
            #[inline]
            fn ref_payload(&self) -> &Vec<$Inner> {
                &self.0
            }
            #[inline]
            fn ref_mut_payload(&mut self) -> &mut Vec<$Inner> {
                &mut self.0
            }
        }

        impl $Strc {
            pub fn iter(&self) -> std::slice::Iter<'_, $Inner> {
                self.ref_payload().iter()
            }
            pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, $Inner> {
                self.ref_mut_payload().iter_mut()
            }
        }
    };
}

pub trait ImmutableStream<T>: Sized {
    fn ref_payload(&self) -> &[T];
    fn capacity(&self) -> usize;

    #[inline]
    fn len(&self) -> usize {
        self.ref_payload().len()
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Vec<T>>() + std::mem::size_of::<T>() * self.capacity()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.ref_payload().is_empty()
    }

    #[inline]
    fn get(&self, idx: usize) -> Option<&T> {
        self.ref_payload().get(idx)
    }

    #[inline]
    fn first(&self) -> Option<&T> {
        self.ref_payload().first()
    }

    #[inline]
    fn last(&self) -> Option<&T> {
        self.ref_payload().last()
    }

    #[inline]
    fn iter(&self) -> Iter<'_, T> {
        self.ref_payload().iter()
    }
}

pub trait LimitedDisplay {
    /// If `limit` was set to a negative value, it will be displayed without abbreviation.
    /// FIXME:
    fn limited_fmt<W: std::fmt::Write>(&self, f: &mut W, limit: isize) -> std::fmt::Result;
    fn to_string_unabbreviated(&self) -> String {
        let mut s = "".to_string();
        self.limited_fmt(&mut s, -1).unwrap();
        s
    }
    const DEFAULT_LIMIT: isize = 10;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ExitStatus {
    pub code: i32,
    pub num_warns: usize,
    pub num_errors: usize,
}

impl ExitStatus {
    pub const OK: ExitStatus = ExitStatus::new(0, 0, 0);
    pub const ERR1: ExitStatus = ExitStatus::new(1, 0, 0);

    pub const fn new(code: i32, num_warns: usize, num_errors: usize) -> Self {
        Self {
            code,
            num_warns,
            num_errors,
        }
    }
    pub const fn compile_passed(num_warns: usize) -> Self {
        Self::new(0, num_warns, 0)
    }

    pub const fn succeed(&self) -> bool {
        self.code == 0 && self.num_errors == 0
    }
}

/// This trait implements REPL (Read-Eval-Print-Loop) automatically
/// The `exec` method is called for file input, etc.
pub trait Runnable: Sized + Default + New {
    type Err: ErrorDisplay;
    type Errs: MultiErrorDisplay<Self::Err>;
    const NAME: &'static str;
    fn cfg(&self) -> &ErgConfig;
    fn cfg_mut(&mut self) -> &mut ErgConfig;
    fn finish(&mut self); // called when the :exit command is received.
    /// Erase all but immutable information.
    fn initialize(&mut self);
    /// Erase information that will no longer be meaningful in the next iteration
    fn clear(&mut self);
    fn eval(&mut self, src: String) -> Result<String, Self::Errs>;
    fn exec(&mut self) -> Result<ExitStatus, Self::Errs>;
    fn input(&self) -> &Input {
        &self.cfg().input
    }
    fn set_input(&mut self, input: Input) {
        self.cfg_mut().input = input;
    }
    fn start_message(&self) -> String {
        #[allow(clippy::const_is_empty)]
        if GIT_HASH_SHORT.is_empty() {
            format!("{} {SEMVER} ({BUILD_DATE}) on {ARCH}/{OS}\n", Self::NAME)
        } else {
            format!(
                "{} {SEMVER} ({GIT_HASH_SHORT}, {BUILD_DATE}) on {ARCH}/{OS}\n",
                Self::NAME
            )
        }
    }
    fn ps1(&self) -> String {
        self.cfg().ps1.to_string()
    }
    fn ps2(&self) -> String {
        self.cfg().ps2.to_string()
    }

    #[inline]
    fn quit(&mut self, code: i32) -> ! {
        self.finish();
        process::exit(code);
    }

    fn quit_successfully(&mut self, mut output: BufWriter<std::io::StdoutLock>) -> ! {
        self.finish();
        if !self.cfg().quiet_repl {
            log!(info_f output, "The REPL has finished successfully.\n");
        }
        process::exit(0);
    }

    /// Returns an optional completeness checker for smart Enter behavior in REPL.
    /// Override this method to provide parser-based completeness detection.
    fn completeness_checker(&self) -> Option<crate::stdin::CompletenessChecker> {
        None
    }

    /// Returns an optional completion provider for Tab completion in the REPL.
    fn completion_provider(&self) -> Option<crate::repl::CompletionProvider> {
        None
    }

    /// Standard execution entry point: executes files/pipes/strings via
    /// `exec`, or drives the REPL (see [`crate::repl`]) for interactive input.
    fn run(cfg: ErgConfig) -> ExitStatus {
        crate::repl::run::<Self>(cfg)
    }
}

pub trait Locational {
    /// NOTE: `loc` cannot be treated as a light method when `self` is a large grammatical element.
    /// If possible, delay the computation by passing `&impl Locational` or other means.
    fn loc(&self) -> Location;

    /// 1-origin
    fn ln_begin(&self) -> Option<u32> {
        match self.loc() {
            Location::Range { ln_begin, .. } | Location::LineRange(ln_begin, _) => Some(ln_begin),
            Location::Line(lineno) => Some(lineno),
            Location::Unknown => None,
        }
    }

    fn ln_end(&self) -> Option<u32> {
        match self.loc() {
            Location::Range { ln_end, .. } | Location::LineRange(_, ln_end) => Some(ln_end),
            Location::Line(lineno) => Some(lineno),
            Location::Unknown => None,
        }
    }

    /// 0-origin
    fn col_begin(&self) -> Option<u32> {
        match self.loc() {
            Location::Range { col_begin, .. } => Some(col_begin),
            _ => None,
        }
    }

    fn col_end(&self) -> Option<u32> {
        match self.loc() {
            Location::Range { col_end, .. } => Some(col_end),
            _ => None,
        }
    }
}

impl<L: Locational> Locational for Option<L> {
    fn loc(&self) -> Location {
        match self {
            Some(l) => l.loc(),
            None => Location::Unknown,
        }
    }
}

impl<L: Locational, R: Locational> Locational for Result<&L, &R> {
    fn loc(&self) -> Location {
        match self {
            Ok(l) => l.loc(),
            Err(r) => r.loc(),
        }
    }
}

impl<L: Locational, R: Locational> Locational for (&L, &R) {
    fn loc(&self) -> Location {
        Location::concat(self.0, self.1)
    }
}

impl Locational for () {
    fn loc(&self) -> Location {
        Location::Unknown
    }
}

impl<T: Locational, const N: usize> Locational for [T; N] {
    fn loc(&self) -> Location {
        Location::stream(self)
    }
}

impl<T: Locational> Locational for Vec<T> {
    fn loc(&self) -> Location {
        Location::stream(self)
    }
}

impl<T: Locational> Locational for &T {
    fn loc(&self) -> Location {
        (*self).loc()
    }
}

#[macro_export]
macro_rules! impl_locational_for_enum {
    ($Enum: ident; $($Variant: ident $(,)?)*) => {
        impl erg_common::traits::Locational for $Enum {
            fn loc(&self) -> erg_common::error::Location {
                match self {
                    $($Enum::$Variant(v) => v.loc(),)*
                }
            }
        }
    }
}

#[macro_export]
macro_rules! impl_locational {
    ($T: ty, $begin: ident, $end: ident) => {
        impl Locational for $T {
            fn loc(&self) -> Location {
                let begin_loc = self.$begin.loc();
                let end_loc = self.$end.loc();
                match (
                    begin_loc.ln_begin(),
                    begin_loc.col_begin(),
                    end_loc.ln_end(),
                    end_loc.col_end(),
                ) {
                    (Some(lb), Some(cb), Some(le), Some(ce)) => Location::range(lb, cb, le, ce),
                    (Some(lb), _, Some(le), _) => Location::LineRange(lb, le),
                    (Some(l), _, _, _) | (_, _, Some(l), _) => Location::Line(l),
                    _ => Location::Unknown,
                }
            }
            fn ln_begin(&self) -> Option<u32> {
                self.$begin.ln_begin()
            }
            fn ln_end(&self) -> Option<u32> {
                self.$end.ln_end()
            }
            fn col_begin(&self) -> Option<u32> {
                self.$begin.col_begin()
            }
            fn col_end(&self) -> Option<u32> {
                self.$end.col_end()
            }
        }
    };
    ($T: ty, lossy $begin: ident, $end: ident) => {
        impl Locational for $T {
            fn loc(&self) -> Location {
                let begin_loc = self.$begin.loc();
                let end_loc = self.$end.loc();
                if begin_loc.is_unknown() {
                    return end_loc;
                }
                match (
                    begin_loc.ln_begin(),
                    begin_loc.col_begin(),
                    end_loc.ln_end(),
                    end_loc.col_end(),
                ) {
                    (Some(lb), Some(cb), Some(le), Some(ce)) => Location::range(lb, cb, le, ce),
                    (Some(lb), _, Some(le), _) => Location::LineRange(lb, le),
                    (Some(l), _, _, _) | (_, _, Some(l), _) => Location::Line(l),
                    _ => Location::Unknown,
                }
            }
        }
    };
    ($T: ty, $begin: ident, lossy $end: ident) => {
        impl Locational for $T {
            fn loc(&self) -> Location {
                let begin_loc = self.$begin.loc();
                let end_loc = self.$end.loc();
                if end_loc.is_unknown() {
                    return begin_loc;
                }
                match (
                    begin_loc.ln_begin(),
                    begin_loc.col_begin(),
                    end_loc.ln_end(),
                    end_loc.col_end(),
                ) {
                    (Some(lb), Some(cb), Some(le), Some(ce)) => Location::range(lb, cb, le, ce),
                    (Some(lb), _, Some(le), _) => Location::LineRange(lb, le),
                    (Some(l), _, _, _) | (_, _, Some(l), _) => Location::Line(l),
                    _ => Location::Unknown,
                }
            }
        }
    };
    ($T: ty, $begin: ident, $middle: ident, $end: ident) => {
        impl Locational for $T {
            fn loc(&self) -> Location {
                let begin_loc = self.$begin.loc();
                let end_loc = self.$end.loc();
                if begin_loc.is_unknown() && end_loc.is_unknown() {
                    return self.$middle.loc();
                }
                match (
                    begin_loc.ln_begin(),
                    begin_loc.col_begin(),
                    end_loc.ln_end(),
                    end_loc.col_end(),
                ) {
                    (Some(lb), Some(cb), Some(le), Some(ce)) => Location::range(lb, cb, le, ce),
                    (Some(lb), _, Some(le), _) => Location::LineRange(lb, le),
                    (Some(l), _, _, _) | (_, _, Some(l), _) => Location::Line(l),
                    _ => Location::Unknown,
                }
            }
            fn ln_begin(&self) -> Option<u32> {
                self.$begin.ln_begin()
            }
            fn ln_end(&self) -> Option<u32> {
                self.$end.ln_end()
            }
            fn col_begin(&self) -> Option<u32> {
                self.$begin.col_begin()
            }
            fn col_end(&self) -> Option<u32> {
                self.$end.col_end()
            }
        }
    };
    ($T: ty, $inner: ident) => {
        impl Locational for $T {
            fn loc(&self) -> Location {
                self.$inner.loc()
            }
        }
    };
}

pub trait NestedDisplay {
    fn fmt_nest(&self, f: &mut std::fmt::Formatter<'_>, level: usize) -> std::fmt::Result;
}

pub trait NoTypeDisplay {
    fn to_string_notype(&self) -> String;
}

/// `impl<T: NestedDisplay> Display for T NestedDisplay`はorphan-ruleに違反するので個別定義する
#[macro_export]
macro_rules! impl_display_from_nested {
    ($T: ty) => {
        impl std::fmt::Display for $T {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.fmt_nest(f, 0)
            }
        }
    };
}

/// For Decl, Def, Call, etc., which can occupy a line by itself
#[macro_export]
macro_rules! impl_nested_display_for_chunk_enum {
    ($Enum: ident; $($Variant: ident $(,)?)*) => {
        impl $crate::traits::NestedDisplay for $Enum {
            fn fmt_nest(&self, f: &mut fmt::Formatter<'_>, level: usize) -> fmt::Result {
                write!(f, "{}", "    ".repeat(level))?;
                match self {
                    $($Enum::$Variant(v) => v.fmt_nest(f, level),)*
                }
            }
        }
    }
}

#[macro_export]
macro_rules! impl_from_trait_for_enum {
    ($Enum: ident; $($Variant: ident $(,)?)*) => {
        $(
            impl From<$Variant> for $Enum {
                fn from(v: $Variant) -> Self {
                    $Enum::$Variant(v)
                }
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_try_from_trait_for_enum {
    ($Enum: ident; $($Variant: ident $(,)?)*) => {
        $(
            impl TryFrom<$Enum> for $Variant {
                type Error = $Enum;
                fn try_from(from: $Enum) -> Result<Self, Self::Error> {
                    match from {
                        Expr::$Variant(to) => Ok(to),
                        _ => Err(from),
                    }
                }
            }
            impl<'x> TryFrom<&'x $Enum> for &'x $Variant {
                type Error = &'x $Enum;
                fn try_from(from: &'x $Enum) -> Result<Self, Self::Error> {
                    match from {
                        Expr::$Variant(to) => Ok(to),
                        _ => Err(from),
                    }
                }
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_nested_display_for_enum {
    ($Enum: ident; $($Variant: ident $(,)?)*) => {
        impl $crate::traits::NestedDisplay for $Enum {
            fn fmt_nest(&self, f: &mut fmt::Formatter<'_>, level: usize) -> fmt::Result {
                match self {
                    $($Enum::$Variant(v) => v.fmt_nest(f, level),)*
                }
            }
        }
    }
}

#[macro_export]
macro_rules! impl_no_type_display_for_enum {
    ($Enum: ident; $($Variant: ident $(,)?)*) => {
        impl $crate::traits::NoTypeDisplay for $Enum {
            fn to_string_notype(&self) -> String {
                match self {
                    $($Enum::$Variant(v) => v.to_string_notype(),)*
                }
            }
        }
    }
}

/// Equivalent to `is` in Python
pub trait AddrEq {
    #[inline]
    fn addr_eq(&self, other: &Self) -> bool {
        addr_eq!(self, other)
    }
}

pub trait StructuralEq {
    fn structural_eq(&self, other: &Self) -> bool;
}

pub trait __Str__ {
    fn __str__(&self) -> String;
}

pub trait OptionalTranspose {
    type Output;
    type Fill;
    /// `self: Option<_>`
    fn transpose(self, fill: Self::Fill) -> Self::Output
    where
        Self: Sized;
}

impl<T: Clone> OptionalTranspose for Option<Vec<T>> {
    type Output = Vec<Option<T>>;
    type Fill = usize;
    fn transpose(self, fill: Self::Fill) -> Self::Output
    where
        Self: Sized,
    {
        match self {
            Some(v) => v.into_iter().map(Some).collect(),
            None => vec![None; fill],
        }
    }
}

pub trait New {
    fn new(cfg: ErgConfig) -> Self;
}

/// Indicates that the type has no interior mutability
// TODO: auto trait
pub trait Immutable {}

impl Immutable for () {}
impl Immutable for bool {}
impl Immutable for char {}
impl Immutable for u8 {}
impl Immutable for u16 {}
impl Immutable for u32 {}
impl Immutable for u64 {}
impl Immutable for u128 {}
impl Immutable for usize {}
impl Immutable for i8 {}
impl Immutable for i16 {}
impl Immutable for i32 {}
impl Immutable for i64 {}
impl Immutable for i128 {}
impl Immutable for isize {}
impl Immutable for f32 {}
impl Immutable for f64 {}
impl Immutable for str {}
impl Immutable for String {}
impl Immutable for crate::Str {}
impl Immutable for std::path::PathBuf {}
impl Immutable for std::path::Path {}
impl Immutable for std::ffi::OsString {}
impl Immutable for std::ffi::OsStr {}
impl Immutable for std::time::Duration {}
impl Immutable for std::time::SystemTime {}
impl Immutable for std::time::Instant {}
impl<T: Immutable + ?Sized> Immutable for &T {}
impl<T: Immutable> Immutable for Option<T> {}
impl<T: Immutable> Immutable for Vec<T> {}
impl<T: Immutable> Immutable for [T] {}
impl<T: Immutable, U: Immutable> Immutable for (T, U) {}
impl<T: Immutable, U: Immutable, V: Immutable> Immutable for (T, U, V) {}
impl<T: Immutable + ?Sized> Immutable for Box<T> {}
impl<T: Immutable + ?Sized> Immutable for std::rc::Rc<T> {}
impl<T: Immutable + ?Sized> Immutable for std::sync::Arc<T> {}

pub trait Traversable {
    type Target;
    fn traverse(&self, rec_f: &mut impl FnMut(&Self::Target));
}

#[macro_export]
macro_rules! impl_traversable_for_enum {
    ($Enum: ident; $Target: ty; $($Variant: ident $(,)?)*) => {
        impl Traversable for $Enum {
            type Target = $Target;
            fn traverse(&self, rec_f: &mut impl FnMut(&Self::Target)) {
                match self {
                    $($Enum::$Variant(v) => v.traverse(rec_f),)*
                }
            }
        }
    }
}
