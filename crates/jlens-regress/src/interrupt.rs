//! Interruptible matches (jlens-regress; not in regress 0.10.5).
//!
//! The backtracker counts steps: every failure that sends it back to its backtrack stack, and
//! every iteration of a loop. Every [`INTERRUPT_INTERVAL`] steps it asks an [`Interrupt`]
//! whether to stop. Between two steps it does work bounded by the size of the pattern and the
//! length of the text, so a match that would run for minutes stops shortly after its interrupt
//! says so.

use core::fmt;

/// How many steps the backtracker takes between two calls to [`Interrupt::should_stop`].
///
/// A step costs tens of nanoseconds on the patterns that backtrack catastrophically, so the
/// calls are well under a millisecond apart while their own cost stays negligible.
pub const INTERRUPT_INTERVAL: u32 = 1024;

/// Decides whether a match stops before it completes.
///
/// Any `FnMut() -> bool` closure is an `Interrupt`: it returns `true` to stop the match, for
/// example when a deadline has passed or a cancellation flag is set.
pub trait Interrupt {
    /// Whether the backtracker counts steps at all. Only [`NoInterrupt`] sets it to `false`, which
    /// compiles the counting away.
    #[doc(hidden)]
    const ENABLED: bool = true;

    /// Returns `true` to stop the match.
    fn should_stop(&mut self) -> bool;
}

impl<F: FnMut() -> bool> Interrupt for F {
    #[inline]
    fn should_stop(&mut self) -> bool {
        self()
    }
}

/// The interrupt of the matches that cannot be interrupted: the code of regress 0.10.5.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoInterrupt;

impl Interrupt for NoInterrupt {
    const ENABLED: bool = false;

    #[inline(always)]
    fn should_stop(&mut self) -> bool {
        false
    }
}

/// Gives any interrupt the `Debug` implementation that the executors derive.
pub(crate) struct Callback<I>(pub(crate) I);

impl<I: Interrupt> Interrupt for Callback<I> {
    const ENABLED: bool = I::ENABLED;

    #[inline(always)]
    fn should_stop(&mut self) -> bool {
        self.0.should_stop()
    }
}

impl<I> fmt::Debug for Callback<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Callback")
    }
}

/// The result of a match that its [`Interrupt`] stopped before it completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupted;

impl fmt::Display for Interrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the match was interrupted")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Interrupted {}

/// The step counter of one match.
pub(crate) struct Poll<I> {
    interrupt: I,
    countdown: u32,
    pub(crate) interrupted: bool,
}

impl<I: Interrupt> Poll<I> {
    pub(crate) fn new(interrupt: I) -> Self {
        Self {
            interrupt,
            countdown: INTERRUPT_INTERVAL,
            interrupted: false,
        }
    }

    /// Counts a step and returns whether the match stops. Callers test `I::ENABLED` first.
    #[inline(always)]
    pub(crate) fn tick(&mut self) -> bool {
        self.countdown -= 1;
        self.countdown == 0 && self.check()
    }

    #[cold]
    #[inline(never)]
    fn check(&mut self) -> bool {
        self.countdown = INTERRUPT_INTERVAL;
        self.interrupted = self.interrupted || self.interrupt.should_stop();
        self.interrupted
    }
}

impl<I> fmt::Debug for Poll<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Poll")
            .field("countdown", &self.countdown)
            .field("interrupted", &self.interrupted)
            .finish_non_exhaustive()
    }
}
