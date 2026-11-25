//! Classical backtracking execution engine

use crate::api::Match;
use crate::bytesearch;
use crate::cursor;
use crate::cursor::{Backward, Direction, Forward};
use crate::exec;
use crate::indexing::{AsciiInput, ElementType, InputIndexer, Utf8Input};
#[cfg(not(feature = "utf16"))]
use crate::insn::StartPredicate;
use crate::insn::{CompiledRegex, Insn, LoopFields, unpack_char_set};
use crate::interrupt::{Interrupt, NoInterrupt, Poll};
use crate::matchers;
use crate::matchers::CharProperties;
use crate::position::PositionType;
use crate::scm;
use crate::scm::SingleCharMatcher;
use crate::types::{
    CaptureGroupID, GroupData, IP, Loop1CharID, LoopData, LoopID, MAX_CAPTURE_GROUPS,
};
use crate::util::DebugCheckIndex;
#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};
use core::ops::Range;

/// BacktrackInsn::SetLoopData is used to restore a position and iteration count.
/// For size reasons, the iteration count is stored as a u32; should the iterations
/// exceed that then we fall back to a more expensive path.
///
/// Lower the limit in debug builds so that we can exercise the more expensive path
/// without needing to overflow a u32.
const MAX_ITERS_STORED_IN_SET_LOOP_DATA: usize = if cfg!(debug_assertions) {
    300
} else {
    u32::MAX as usize
};

/// The most iterations of a greedy 1-char loop that one GreedyLoop1Char entry
/// covers; a longer run pushes an entry per chunk. A character is at most four
/// code units, so an entry's count of code units always fits in its u32.
///
/// As with MAX_ITERS_STORED_IN_SET_LOOP_DATA, debug builds lower the limit so
/// that tests reach that path.
const LOOP1CHAR_CHUNK_ITERS: usize = if cfg!(debug_assertions) {
    75
} else {
    u32::MAX as usize / 4
};

/// The code units from `from` to `to`, travelling in direction `Dir`.
#[inline(always)]
fn distance<Dir: Direction, P: PositionType>(from: P, to: P) -> usize {
    if Dir::FORWARD { to - from } else { from - to }
}

// Note we work hard to ensure this stays 16 bytes with pointer positions.
#[derive(Clone, Debug)]
enum BacktrackInsn<Input: InputIndexer> {
    /// Nothing more to backtrack.
    /// This "backstops" our stack.
    Exhausted,

    /// Restore the IP and position.
    /// IPs here are u32, matching JumpTarget, to keep this enum small.
    SetPosition { ip: u32, pos: Input::Position },

    /// Undo one LoopAgain: decrement the loop's iteration count and restore
    /// the position its previous iteration started at.
    PrevLoopIteration { id: LoopID, entry: Input::Position },

    /// Undo an EnterLoop: restore the loop's state from before this run of it.
    /// For a loop nested in another loop, that is the state an earlier run
    /// left behind, which backtracking into that run still needs.
    SetLoopData {
        id: LoopID,
        iters: u32,
        entry: Input::Position,
    },

    /// Restore only a loop's iteration count. Undoes an EnterLoop whose old
    /// count is too large for SetLoopData, together with a PrevLoopIteration
    /// pushed above it.
    SetLoopIters { id: LoopID, iters: usize },

    /// Set the start (or end, if is_start is false) of a capture group to a position.
    /// This is the value the bound held before it was overwritten.
    SetCaptureGroup {
        id: CaptureGroupID,
        pos: Option<Input::Position>,
        is_start: bool,
    },

    /// Resume by entering a non-greedy loop whose body was skipped.
    EnterNonGreedyLoop {
        // The IP of the loop.
        // This is guaranteed to point to an EnterLoopInsn.
        ip: u32,
        // True if EnterLoop skipped the body, so resuming starts a fresh run of
        // the loop. False if LoopAgain did, so resuming continues the current
        // run, whose count unwinding has already restored.
        entering: bool,
        // The position to enter the body at.
        entry: Input::Position,
    },

    /// Retry a greedy 1-char loop stopping one character earlier than `pos`,
    /// where it last stopped. `remaining` is how far back, in code units, the
    /// earlier stopping points go; at least one character's worth. `id`
    /// locates the loop's instruction through `CompiledRegex::loop1char_ips`.
    GreedyLoop1Char {
        id: Loop1CharID,
        remaining: u32,
        pos: Input::Position,
    },

    /// Retry a non-greedy 1-char loop stopping one character later than `pos`,
    /// where it last stopped, if its body matches there. `remaining` is how
    /// many more iterations the quantifier allows, or 0 if it is unbounded.
    /// `id` as for GreedyLoop1Char.
    NonGreedyLoop1Char {
        id: Loop1CharID,
        remaining: u32,
        pos: Input::Position,
    },
}

#[derive(Debug)]
struct State<Position: PositionType> {
    loops: Box<[LoopData<Position>]>,
    groups: Box<[GroupData<Position>]>,
}

/// jlens-regress: storage the lookarounds reuse instead of allocating it each time: the groups
/// they save, on a stack, and backtrack stacks that hold only `Exhausted`.
#[derive(Debug)]
struct Lookarounds<Input: InputIndexer> {
    saved_groups: Vec<GroupData<Input::Position>>,
    spare_bts: Vec<Vec<BacktrackInsn<Input>>>,
}

impl<Input: InputIndexer> Default for Lookarounds<Input> {
    fn default() -> Self {
        Self {
            saved_groups: Vec::new(),
            spare_bts: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct MatchAttempter<'a, Input: InputIndexer, I: Interrupt = NoInterrupt> {
    re: &'a CompiledRegex,
    /// `re.loop1char_ips`, held directly to save a pointer hop on every 1-char
    /// loop retry.
    loop1char_ips: &'a [u32],
    bts: Vec<BacktrackInsn<Input>>,
    s: State<Input::Position>,
    poll: Poll<I>,
    // jlens-regress: behind a pointer: as fields of their own, they changed the code of the
    // main loop, which searches without a lookaround run.
    lookarounds: Option<Box<Lookarounds<Input>>>,
}

impl<'a, Input: InputIndexer> MatchAttempter<'a, Input> {
    pub(crate) fn new(re: &'a CompiledRegex, entry: Input::Position) -> Self {
        Self::with_interrupt(re, entry, NoInterrupt)
    }
}

impl<'a, Input: InputIndexer, I: Interrupt> MatchAttempter<'a, Input, I> {
    pub(crate) fn with_interrupt(
        re: &'a CompiledRegex,
        entry: Input::Position,
        interrupt: I,
    ) -> Self {
        Self {
            re,
            loop1char_ips: &re.loop1char_ips,
            bts: vec![BacktrackInsn::Exhausted],
            s: State {
                loops: vec![LoopData::new(entry); re.loops as usize].into_boxed_slice(),
                groups: vec![GroupData::new(); re.groups as usize].into_boxed_slice(),
            },
            poll: Poll::new(interrupt),
            lookarounds: None,
        }
    }

    #[inline(always)]
    fn push_backtrack(&mut self, bt: BacktrackInsn<Input>) {
        self.bts.push(bt)
    }

    /// Set the end (or start, if `is_start`) of a capture group to `pos`, first
    /// saving the old value so that backtracking restores it.
    ///
    /// A write that leaves the value unchanged needs nothing restored, so it
    /// pushes nothing.
    #[inline(always)]
    fn set_capture_bound_with_bt(
        &mut self,
        id: CaptureGroupID,
        is_start: bool,
        pos: Option<Input::Position>,
    ) {
        let cg = self.s.groups.mat(id as usize);
        let bound = if is_start { &mut cg.start } else { &mut cg.end };
        if *bound != pos {
            self.bts.push(BacktrackInsn::SetCaptureGroup {
                id,
                pos: *bound,
                is_start,
            });
            *bound = pos;
        }
    }

    #[inline(always)]
    fn pop_backtrack(&mut self) {
        // Note we never pop the last instruction so this will never be empty.
        debug_assert!(!self.bts.is_empty());
        if cfg!(feature = "prohibit-unsafe") {
            self.bts.pop();
        } else {
            unsafe { self.bts.set_len(self.bts.len() - 1) }
        }
    }

    /// Start an iteration of a loop at `pos`, first pushing what undoes it.
    ///
    /// `entering` is true for the first iteration of a fresh run of the loop
    /// (EnterLoop). The loop's slot may then hold an earlier run's state, so
    /// that whole state is saved. Otherwise (LoopAgain) the count simply goes
    /// up by one, so undoing it is a decrement and only the entry position is
    /// saved.
    fn prepare_to_enter_loop(
        bts: &mut Vec<BacktrackInsn<Input>>,
        pos: Input::Position,
        id: LoopID,
        loop_data: &mut LoopData<Input::Position>,
        entering: bool,
    ) {
        if !entering {
            bts.push(BacktrackInsn::PrevLoopIteration {
                id,
                entry: loop_data.entry,
            });
            loop_data.iters += 1;
        } else {
            if loop_data.iters <= MAX_ITERS_STORED_IN_SET_LOOP_DATA {
                bts.push(BacktrackInsn::SetLoopData {
                    id,
                    iters: loop_data.iters as u32,
                    entry: loop_data.entry,
                });
            } else {
                crate::util::cold_path();
                // Unwinding pops the PrevLoopIteration first (count 1 -> 0,
                // entry restored), then restores the old count.
                bts.push(BacktrackInsn::SetLoopIters {
                    id,
                    iters: loop_data.iters,
                });
                bts.push(BacktrackInsn::PrevLoopIteration {
                    id,
                    entry: loop_data.entry,
                });
            }
            loop_data.iters = 1;
        }
        loop_data.entry = pos;
    }

    /// Executes a loop starting at the given instruction pointer.
    ///
    /// # Arguments
    ///
    /// * `loop_fields` - Metadata describing the loop.
    /// * `pos` - The current position in the input.
    /// * `ip` - The instruction pointer of the loop itself.
    /// * `entering` - True for the `EnterLoop` instruction, which starts a fresh
    ///   run of the loop at iteration 0. False for `LoopAgain`, which continues
    ///   the current run from the loop's recorded iteration count.
    ///
    /// # Returns
    ///
    /// The next instruction pointer, or `None` if execution cannot continue.
    fn run_loop(
        &mut self,
        loop_fields: &'a LoopFields,
        pos: Input::Position,
        ip: IP,
        entering: bool,
    ) -> Option<IP> {
        let loop_data = &mut self.s.loops[loop_fields.loop_id as usize];
        let iteration = if entering { 0 } else { loop_data.iters };

        let do_taken = iteration < loop_fields.max_iters;
        let do_not_taken = iteration >= loop_fields.min_iters;

        let loop_taken_ip = ip + 1;
        let loop_not_taken_ip = loop_fields.exit as IP;

        // If we have looped more than the minimum number of iterations, reject empty
        // matches. ES6 21.2.2.5.1 Note 4: "once the minimum number of
        // repetitions has been satisfied, any more expansions of Atom that match the
        // empty character sequence are not considered for further repetitions."
        if loop_data.entry == pos && iteration > loop_fields.min_iters {
            return None;
        }

        match (do_taken, do_not_taken) {
            (false, false) => {
                // No arms viable.
                None
            }
            (false, true) => {
                // Only skipping is viable.
                Some(loop_not_taken_ip)
            }
            (true, false) => {
                // Only entering is viable.
                Self::prepare_to_enter_loop(
                    &mut self.bts,
                    pos,
                    loop_fields.loop_id,
                    loop_data,
                    entering,
                );
                Some(loop_taken_ip)
            }
            (true, true) if !loop_fields.greedy => {
                // Both arms are viable; backtrack into the loop.
                self.bts.push(BacktrackInsn::EnterNonGreedyLoop {
                    ip: ip as u32,
                    entering,
                    entry: pos,
                });
                Some(loop_not_taken_ip)
            }
            (true, true) => {
                debug_assert!(loop_fields.greedy, "Should be greedy");
                // Both arms are viable; backtrack out of the loop.
                self.bts.push(BacktrackInsn::SetPosition {
                    ip: loop_not_taken_ip as u32,
                    pos,
                });
                Self::prepare_to_enter_loop(
                    &mut self.bts,
                    pos,
                    loop_fields.loop_id,
                    loop_data,
                    entering,
                );
                Some(loop_taken_ip)
            }
        }
    }

    // Drive the loop up to \p max times.
    // \return the position (min, max), or None on failure.
    #[inline(always)]
    fn run_scm_loop_impl<Dir: Direction, Scm: SingleCharMatcher<Input, Dir>>(
        input: &Input,
        mut pos: Input::Position,
        min: usize,
        max: usize,
        dir: Dir,
        matcher: Scm,
    ) -> Option<(Input::Position, Input::Position)> {
        debug_assert!(min <= max, "min should be <= max");
        // Drive the iteration min times.
        // That tells us the min position.
        for _ in 0..min {
            if !matcher.matches(input, dir, &mut pos) {
                return None;
            }
        }
        let min_pos = pos;

        // Drive it up to the max.
        for _ in 0..(max - min) {
            let saved = pos;
            if !matcher.matches(input, dir, &mut pos) {
                pos = saved;
                break;
            }
        }
        Some((min_pos, pos))
    }

    // Run the loop at \p ip, whose body matches one character, between \p min
    // and \p max times from \p pos. This is always inlined, so that a caller
    // passing constant counts gets a loop specialized to them; callers that
    // walk use with_scm_loop_impl instead, sharing one copy.
    // \return the positions after min and after the last iteration, or None if
    // fewer than min iterations match.
    #[inline(always)]
    fn with_scm_loop_inline<Dir: Direction>(
        re: &CompiledRegex,
        input: &Input,
        pos: Input::Position,
        min: usize,
        max: usize,
        dir: Dir,
        ip: IP,
    ) -> Option<(Input::Position, Input::Position)> {
        match re.insns.iat(ip + 1) {
            &Insn::Char(c) => {
                // jlens-regress: a character the input cannot hold (a lone surrogate in a
                // `&str`) matches nowhere, so the loop matches it zero times, if it may.
                let Some(c) = <<Input as InputIndexer>::Element as ElementType>::try_from(c) else {
                    return (min == 0).then_some((pos, pos));
                };
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::Char { c })
            }
            &Insn::Bracket(idx) => {
                let bc = re.brackets.iat(idx as usize);
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::Bracket { bc })
            }
            Insn::AsciiBracket(bitmap) => Self::run_scm_loop_impl(
                input,
                pos,
                min,
                max,
                dir,
                scm::MatchByteSet { bytes: bitmap },
            ),
            Insn::MatchAny => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchAny::new())
            }
            Insn::MatchAnyExceptLineTerminator => Self::run_scm_loop_impl(
                input,
                pos,
                min,
                max,
                dir,
                scm::MatchAnyExceptLineTerminator::new(),
            ),
            &Insn::CharSet { lo, hi } => {
                let chars = unpack_char_set(lo, hi);
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::CharSet { chars })
            }
            &Insn::ByteSet2(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteArraySet(bytes))
            }
            &Insn::ByteSet3(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteArraySet(bytes))
            }
            &Insn::ByteSet4(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteArraySet(bytes))
            }
            Insn::ByteSeq1(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteSeq(bytes))
            }
            Insn::ByteSeq2(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteSeq(bytes))
            }
            Insn::ByteSeq3(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteSeq(bytes))
            }
            Insn::ByteSeq4(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteSeq(bytes))
            }
            Insn::ByteSeq5(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteSeq(bytes))
            }
            Insn::ByteSeq6(bytes) => {
                Self::run_scm_loop_impl(input, pos, min, max, dir, scm::MatchByteSeq(bytes))
            }
            _ => {
                unreachable!("Missing SCM: {:?}", re.insns.iat(ip + 1));
            }
        }
    }

    // with_scm_loop_inline, not inlined, for the loop walks: they share this one
    // copy of the dispatch instead of each getting their own. Left to itself,
    // LLVM inlines this small wrapper, and with it the dispatch, into
    // try_at_pos, doubling its size and slowing every match that backtracks.
    #[inline(never)]
    fn with_scm_loop_impl<Dir: Direction>(
        re: &CompiledRegex,
        input: &Input,
        pos: Input::Position,
        min: usize,
        max: usize,
        dir: Dir,
        ip: IP,
    ) -> Option<(Input::Position, Input::Position)> {
        Self::with_scm_loop_inline(re, input, pos, min, max, dir, ip)
    }

    // Run the greedy 1-char loop at \p ip from \p pos. The big idea here is that
    // we don't need to save our position every iteration: we know that our loop
    // body matches a single character so we can backtrack by matching a
    // character backwards.
    // \return where the loop stops, or None if it failed.
    #[allow(clippy::too_many_arguments)]
    fn run_greedy_loop1char<Dir: Direction>(
        &mut self,
        input: &Input,
        dir: Dir,
        pos: Input::Position,
        id: Loop1CharID,
        min: usize,
        max: usize,
        ip: IP,
    ) -> Option<Input::Position> {
        // Take as many iterations as possible. Each other stopping point, back
        // to the end of the minimum iterations, is a retry. An entry counts how
        // far back its stopping points go in code units, which costs one
        // subtraction here; counting iterations would slow the walk. The walk
        // goes in chunks so that each entry's count fits.
        let mut left = max - min;
        let mut n = left.min(LOOP1CHAR_CHUNK_ITERS);
        let (mut start, mut end) =
            Self::with_scm_loop_impl(self.re, input, pos, min, min + n, dir, ip)?;
        loop {
            let span = distance::<Dir, _>(start, end);
            debug_assert!(span <= 4 * LOOP1CHAR_CHUNK_ITERS, "Chunk too long");
            if span > 0 {
                self.bts.push(BacktrackInsn::GreedyLoop1Char {
                    id,
                    remaining: span as u32,
                    pos: end,
                });
            }
            left -= n;
            // Fewer than n code units means fewer than n iterations: the body
            // stopped matching.
            if left == 0 || span < n {
                return Some(end);
            }
            crate::util::cold_path();
            n = left.min(LOOP1CHAR_CHUNK_ITERS);
            start = end;
            (_, end) = Self::with_scm_loop_impl(self.re, input, start, 0, n, dir, ip)?;
        }
    }

    // Run the non-greedy 1-char loop at \p ip from \p pos. It takes only the
    // minimum; each retry then tries one more iteration, so the loop never
    // looks further ahead than it has to.
    // \return where the loop stops, or None if it failed.
    #[allow(clippy::too_many_arguments)]
    fn run_nongreedy_loop1char<Dir: Direction>(
        &mut self,
        input: &Input,
        dir: Dir,
        pos: Input::Position,
        id: Loop1CharID,
        min: usize,
        max: usize,
        ip: IP,
    ) -> Option<Input::Position> {
        let (min_pos, _) = Self::with_scm_loop_impl(self.re, input, pos, min, min, dir, ip)?;
        if min < max {
            // Count the iterations the quantifier still allows, or 0 if it is
            // unbounded. The optimizer keeps bounded counts within a u32.
            let remaining = if max == usize::MAX { 0 } else { max - min };
            debug_assert!(remaining <= u32::MAX as usize, "Count should fit in u32");
            self.bts.push(BacktrackInsn::NonGreedyLoop1Char {
                id,
                remaining: remaining as u32,
                pos: min_pos,
            });
        }
        Some(min_pos)
    }

    // Run a lookaround instruction, which is either forwards or backwards
    // (according to Direction). The half-open range
    // start_group..end_group is the range of contained capture groups.
    // \return whether we matched and negate was false, or did not match but negate
    // is true.
    fn run_lookaround<Dir: Direction>(
        &mut self,
        input: &Input,
        ip: IP,
        pos: Input::Position,
        start_group: CaptureGroupID,
        end_group: CaptureGroupID,
        negate: bool,
    ) -> bool {
        // Copy capture groups, because if the match fails (or if we are inverted)
        // we need to restore these.
        let range = (start_group as usize)..(end_group as usize);
        // jlens-regress: the groups go on `saved_groups` and the lookaround runs on a spare
        // backtrack stack, both reused by the next lookaround; nested ones take theirs above.
        let store = self.lookarounds.get_or_insert_with(Default::default);
        let base = store.saved_groups.len();
        store
            .saved_groups
            .extend_from_slice(self.s.groups.iat(range.clone()));

        // Start with an "empty" backtrack stack.
        let mut saved_bts = store
            .spare_bts
            .pop()
            .unwrap_or_else(|| vec![BacktrackInsn::Exhausted]);
        core::mem::swap(&mut self.bts, &mut saved_bts);

        // Enter into the lookaround's instruction stream.
        let matched = self.try_at_pos(*input, ip, pos, Dir::new()).is_some();

        // Put back our bts. The lookaround's holds only `Exhausted` again, as every attempt
        // leaves it.
        core::mem::swap(&mut self.bts, &mut saved_bts);
        let store = match self.lookarounds.as_deref_mut() {
            Some(store) => store,
            None => rs_unreachable!("The lookaround's storage was allocated above"),
        };
        store.spare_bts.push(saved_bts);

        // If we are a positive lookahead that successfully matched, retain the
        // capture groups (but we need to set up backtracking). Otherwise restore
        // them.
        if matched && !negate {
            // Save each bound the lookaround changed. A bound it left alone
            // needs nothing restored.
            for (idx, old) in store.saved_groups[base..].iter().enumerate() {
                debug_assert!(idx + (start_group as usize) < MAX_CAPTURE_GROUPS);
                let id = (idx as CaptureGroupID) + start_group;
                let cur = *self.s.groups.iat(id as usize);
                if cur.start != old.start {
                    self.bts.push(BacktrackInsn::SetCaptureGroup {
                        id,
                        pos: old.start,
                        is_start: true,
                    });
                }
                if cur.end != old.end {
                    self.bts.push(BacktrackInsn::SetCaptureGroup {
                        id,
                        pos: old.end,
                        is_start: false,
                    });
                }
            }
        } else {
            self.s
                .groups
                .mat(range)
                .copy_from_slice(&store.saved_groups[base..]);
        }
        store.saved_groups.truncate(base);
        matched != negate
    }

    /// Attempt to backtrack.
    /// \return true if we backtracked, false if we exhaust the backtrack stack.
    fn try_backtrack<Dir: Direction>(
        &mut self,
        input: &Input,
        ip: &mut IP,
        pos: &mut Input::Position,
        dir: Dir,
    ) -> bool {
        loop {
            // We always have a single Exhausted instruction backstopping our stack,
            // so we do not need to check for empty bts.
            debug_assert!(!self.bts.is_empty(), "Backtrack stack should not be empty");
            let bt = match self.bts.last_mut() {
                Some(bt) => bt,
                None => rs_unreachable!("BT stack should never be empty"),
            };
            match bt {
                BacktrackInsn::Exhausted => return false,

                BacktrackInsn::SetPosition {
                    ip: saved_ip,
                    pos: saved_pos,
                } => {
                    *ip = *saved_ip as IP;
                    *pos = *saved_pos;
                    self.pop_backtrack();
                    return true;
                }
                &mut BacktrackInsn::PrevLoopIteration { id, entry } => {
                    let loop_data = self.s.loops.mat(id as usize);
                    debug_assert!(loop_data.iters >= 1, "No iteration to undo");
                    loop_data.iters -= 1;
                    loop_data.entry = entry;
                    self.pop_backtrack();
                }
                &mut BacktrackInsn::SetLoopData { id, iters, entry } => {
                    let loop_data = self.s.loops.mat(id as usize);
                    loop_data.iters = iters as usize;
                    loop_data.entry = entry;
                    self.pop_backtrack();
                }
                &mut BacktrackInsn::SetLoopIters { id, iters } => {
                    self.s.loops.mat(id as usize).iters = iters;
                    self.pop_backtrack();
                }
                &mut BacktrackInsn::SetCaptureGroup {
                    id,
                    pos: saved_pos,
                    is_start,
                } => {
                    let cg = self.s.groups.mat(id as usize);
                    if is_start {
                        cg.start = saved_pos;
                    } else {
                        cg.end = saved_pos;
                    }
                    self.pop_backtrack();
                }

                &mut BacktrackInsn::EnterNonGreedyLoop {
                    ip: loop_ip,
                    entering,
                    entry,
                } => {
                    self.pop_backtrack();
                    let loop_ip = loop_ip as IP;
                    *ip = loop_ip + 1;
                    *pos = entry;
                    let loop_fields = match &self.re.insns.iat(loop_ip) {
                        Insn::EnterLoop(loop_fields) => loop_fields,
                        _ => rs_unreachable!("EnterNonGreedyLoop must point at a loop instruction"),
                    };
                    let loop_data = self.s.loops.mat(loop_fields.loop_id as usize);
                    debug_assert!(
                        entering || loop_data.iters >= 1,
                        "Continuing a run of a loop that has not been entered"
                    );
                    Self::prepare_to_enter_loop(
                        &mut self.bts,
                        *pos,
                        loop_fields.loop_id,
                        loop_data,
                        entering,
                    );
                    return true;
                }

                BacktrackInsn::GreedyLoop1Char {
                    id,
                    remaining,
                    pos: stop,
                } => {
                    // The match failed with the loop stopped at `stop`; stop one
                    // character earlier, against the direction of travel.
                    let loop_ip = *self.loop1char_ips.iat(*id as usize) as IP;
                    let earlier = if Dir::FORWARD {
                        input.next_left_pos(*stop)
                    } else {
                        input.next_right_pos(*stop)
                    };
                    let Some(earlier) = earlier else {
                        rs_unreachable!("Should be able to step back since retries remain")
                    };
                    // In debug builds, this panics if the step went past the
                    // loop's minimum.
                    *remaining -= distance::<Dir, _>(earlier, *stop) as u32;
                    *pos = earlier;
                    *ip = loop_ip + 2;
                    if *remaining == 0 {
                        // That was the last retry.
                        self.pop_backtrack();
                    } else {
                        *stop = earlier;
                    }
                    return true;
                }

                BacktrackInsn::NonGreedyLoop1Char {
                    id,
                    remaining,
                    pos: stop,
                } => {
                    // The match failed with the loop stopped at `stop`; try one
                    // more iteration there. If the body does not match, no later
                    // stopping point exists either.
                    let loop_ip = *self.loop1char_ips.iat(*id as usize) as IP;
                    // With constant counts, this inlines to a single match.
                    let Some((later, _)) =
                        Self::with_scm_loop_inline(self.re, input, *stop, 1, 1, dir, loop_ip)
                    else {
                        self.pop_backtrack();
                        continue;
                    };
                    *pos = later;
                    *ip = loop_ip + 2;
                    if *remaining == 1 {
                        // That was the last iteration the quantifier allows.
                        self.pop_backtrack();
                    } else {
                        // An unbounded loop's 0 stays 0.
                        *remaining = remaining.saturating_sub(1);
                        *stop = later;
                    }
                    return true;
                }
            }
        }
    }

    /// Attempt to match at a given IP and position.
    fn try_at_pos<Dir: Direction>(
        &mut self,
        inp: Input,
        ip: IP,
        pos: Input::Position,
        dir: Dir,
    ) -> Option<Input::Position> {
        // jlens-regress: the attempt counts its steps on a local copy of the counter.
        if !I::ENABLED {
            return self.attempt(inp, ip, pos, dir, &mut 0);
        }
        let mut countdown = self.poll.countdown;
        let result = self.attempt(inp, ip, pos, dir, &mut countdown);
        self.poll.countdown = countdown;
        result
    }

    /// The body of `try_at_pos`, which counts its steps on `countdown` (jlens-regress).
    #[inline(always)]
    fn attempt<Dir: Direction>(
        &mut self,
        inp: Input,
        mut ip: IP,
        mut pos: Input::Position,
        dir: Dir,
        countdown: &mut u32,
    ) -> Option<Input::Position> {
        debug_assert!(
            self.bts.len() == 1,
            "Should be only initial exhausted backtrack insn"
        );
        // TODO: we are inconsistent about passing Input by reference or value.
        let input = &inp;
        let re = self.re;
        // These are not really loops, they are just labels that we effectively 'goto'
        // to.
        #[allow(clippy::never_loop)]
        'nextinsn: loop {
            'backtrack: loop {
                // Helper macro to either increment ip and go to the next insn, or backtrack.
                macro_rules! next_or_bt {
                    ($e:expr) => {
                        if $e {
                            ip += 1;
                            continue 'nextinsn;
                        } else {
                            break 'backtrack;
                        }
                    };
                }

                match re.insns.iat(ip) {
                    &Insn::Char(c) => {
                        let m = match <<Input as InputIndexer>::Element as ElementType>::try_from(c)
                        {
                            Some(c) => scm::Char { c }.matches(input, dir, &mut pos),
                            None => false,
                        };
                        next_or_bt!(m);
                    }

                    &Insn::CharSet { lo, hi } => {
                        let chars = unpack_char_set(lo, hi);
                        let m = scm::CharSet { chars }.matches(input, dir, &mut pos);
                        next_or_bt!(m);
                    }

                    &Insn::ByteSet2(bytes) => {
                        next_or_bt!(scm::MatchByteArraySet(bytes).matches(input, dir, &mut pos))
                    }
                    &Insn::ByteSet3(bytes) => {
                        next_or_bt!(scm::MatchByteArraySet(bytes).matches(input, dir, &mut pos))
                    }
                    &Insn::ByteSet4(bytes) => {
                        next_or_bt!(scm::MatchByteArraySet(bytes).matches(input, dir, &mut pos))
                    }

                    Insn::ByteSeq1(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq2(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq3(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq4(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq5(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq6(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq7(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq8(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq9(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq10(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq11(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq12(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq13(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq14(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    Insn::ByteSeq15(v) => {
                        next_or_bt!(cursor::try_match_lit(input, dir, &mut pos, v))
                    }
                    &Insn::ByteSeqN((start, end)) => {
                        let v = re.byte_literal_buffer.iat(start as usize..end as usize);
                        next_or_bt!(input.match_byte_slice(dir, &mut pos, v))
                    }

                    Insn::AsciiBracket(bitmap) => next_or_bt!(
                        scm::MatchByteSet { bytes: bitmap }.matches(input, dir, &mut pos)
                    ),

                    &Insn::Bracket(idx) => {
                        next_or_bt!(
                            scm::Bracket {
                                bc: re.brackets.iat(idx as usize)
                            }
                            .matches(input, dir, &mut pos)
                        )
                    }

                    Insn::MatchAny => {
                        next_or_bt!(scm::MatchAny::new().matches(input, dir, &mut pos))
                    }

                    Insn::MatchAnyExceptLineTerminator => {
                        next_or_bt!(
                            scm::MatchAnyExceptLineTerminator::new().matches(input, dir, &mut pos)
                        )
                    }

                    &Insn::WordBoundary { invert } => {
                        // Copy the positions since these destructively move them.
                        let prev_wordchar = input
                            .peek_left(pos)
                            .is_some_and(Input::CharProps::is_word_char);
                        let curr_wordchar = input
                            .peek_right(pos)
                            .is_some_and(Input::CharProps::is_word_char);
                        let is_boundary = prev_wordchar != curr_wordchar;
                        next_or_bt!(is_boundary != invert)
                    }

                    &Insn::WordBoundaryUnicodeICase { invert } => {
                        let prev_wordchar = input
                            .peek_left(pos)
                            .is_some_and(Input::CharProps::is_word_char_unicode_icase);
                        let curr_wordchar = input
                            .peek_right(pos)
                            .is_some_and(Input::CharProps::is_word_char_unicode_icase);
                        let is_boundary = prev_wordchar != curr_wordchar;
                        next_or_bt!(is_boundary != invert)
                    }

                    Insn::StartOfLine { multiline } => {
                        let multiline = *multiline;
                        let matches = match input.peek_left(pos) {
                            None => true,
                            Some(c) if multiline && Input::CharProps::is_line_terminator(c) => true,
                            _ => false,
                        };
                        next_or_bt!(matches)
                    }
                    Insn::EndOfLine { multiline } => {
                        let multiline = *multiline;
                        let matches = match input.peek_right(pos) {
                            None => true, // we're at the right of the string
                            Some(c) if multiline && Input::CharProps::is_line_terminator(c) => true,
                            _ => false,
                        };
                        next_or_bt!(matches)
                    }

                    &Insn::Jump { target } => {
                        ip = target as usize;
                        continue 'nextinsn;
                    }

                    &Insn::BeginCaptureGroup(cg_idx) => {
                        // Forwards we enter at the start; backwards, at the end.
                        debug_assert!(
                            if Dir::FORWARD {
                                self.s.groups.iat(cg_idx as usize).end.is_none()
                            } else {
                                self.s.groups.iat(cg_idx as usize).start.is_none()
                            },
                            "Should not have already exited capture group we are entering"
                        );
                        self.set_capture_bound_with_bt(cg_idx, Dir::FORWARD, Some(pos));
                        next_or_bt!(true)
                    }

                    &Insn::EndCaptureGroup(cg_idx) => {
                        // Forwards we exit at the end; backwards, at the start.
                        debug_assert!(
                            if Dir::FORWARD {
                                self.s.groups.iat(cg_idx as usize).start_matched()
                            } else {
                                self.s.groups.iat(cg_idx as usize).end_matched()
                            },
                            "Capture group should have been entered"
                        );
                        self.set_capture_bound_with_bt(cg_idx, !Dir::FORWARD, Some(pos));
                        next_or_bt!(true)
                    }

                    &Insn::ResetCaptureGroup(cg_idx) => {
                        self.set_capture_bound_with_bt(cg_idx, true, None);
                        self.set_capture_bound_with_bt(cg_idx, false, None);
                        next_or_bt!(true)
                    }

                    &Insn::BackRef {
                        group: cg_idx,
                        icase,
                    } => {
                        let cg = self.s.groups.mat(cg_idx as usize);
                        // Backreferences to a capture group that did not match always succeed (ES5
                        // 15.10.2.9).
                        // Note we may be in the capture group we are examining, e.g. /(abc\1)/.
                        let matched;
                        if let Some(orig_range) = cg.as_range() {
                            if icase {
                                matched = matchers::backref_icase(input, dir, orig_range, &mut pos);
                            } else {
                                matched = matchers::backref(input, dir, orig_range, &mut pos);
                            }
                        } else {
                            // This group has not been exited and so the match succeeds (ES6
                            // 21.2.2.9).
                            matched = true;
                        }
                        next_or_bt!(matched)
                    }

                    &Insn::Lookahead {
                        negate,
                        start_group,
                        end_group,
                        continuation,
                    } => {
                        // jlens-regress: the lookaround's attempt counts on, and
                        // interrupts through, `self.poll`.
                        if I::ENABLED {
                            self.poll.countdown = *countdown;
                        }
                        let matched = self.run_lookaround::<Forward>(
                            input,
                            ip + 1,
                            pos,
                            start_group,
                            end_group,
                            negate,
                        );
                        if I::ENABLED {
                            *countdown = self.poll.countdown;
                            if self.poll.interrupted {
                                self.bts.truncate(1);
                                return None;
                            }
                        }
                        if matched {
                            ip = continuation as IP;
                            continue 'nextinsn;
                        } else {
                            break 'backtrack;
                        }
                    }

                    &Insn::Lookbehind {
                        negate,
                        start_group,
                        end_group,
                        continuation,
                    } => {
                        // jlens-regress: the lookaround's attempt counts on, and
                        // interrupts through, `self.poll`.
                        if I::ENABLED {
                            self.poll.countdown = *countdown;
                        }
                        let matched = self.run_lookaround::<Backward>(
                            input,
                            ip + 1,
                            pos,
                            start_group,
                            end_group,
                            negate,
                        );
                        if I::ENABLED {
                            *countdown = self.poll.countdown;
                            if self.poll.interrupted {
                                self.bts.truncate(1);
                                return None;
                            }
                        }
                        if matched {
                            ip = continuation as IP;
                            continue 'nextinsn;
                        } else {
                            break 'backtrack;
                        }
                    }

                    &Insn::Alt { secondary } => {
                        self.push_backtrack(BacktrackInsn::SetPosition { ip: secondary, pos });
                        next_or_bt!(true);
                    }

                    Insn::EnterLoop(fields) => {
                        // Entering a loop, not re-entering it.
                        match self.run_loop(fields, pos, ip, true) {
                            Some(next_ip) => {
                                ip = next_ip;
                                continue 'nextinsn;
                            }
                            None => {
                                break 'backtrack;
                            }
                        }
                    }

                    &Insn::LoopAgain { begin } => {
                        if I::ENABLED && self.poll.tick(countdown) {
                            self.bts.truncate(1);
                            return None;
                        }
                        let act = match re.insns.iat(begin as IP) {
                            Insn::EnterLoop(fields) => {
                                self.run_loop(fields, pos, begin as IP, false)
                            }
                            _ => rs_unreachable!("EnterLoop should always refer to loop field"),
                        };
                        match act {
                            Some(next_ip) => {
                                ip = next_ip;
                                continue 'nextinsn;
                            }
                            None => break 'backtrack,
                        }
                    }

                    &Insn::Loop1CharBody {
                        id,
                        min_iters,
                        max_iters,
                        greedy,
                    } => {
                        let stop = if greedy {
                            self.run_greedy_loop1char(input, dir, pos, id, min_iters, max_iters, ip)
                        } else {
                            self.run_nongreedy_loop1char(
                                input, dir, pos, id, min_iters, max_iters, ip,
                            )
                        };
                        let Some(stop) = stop else {
                            break 'backtrack;
                        };
                        pos = stop;
                        // Oh no where is the continuation? It's one past the loop
                        // body, which is one past the loop. Strap in!
                        ip += 2;
                        continue 'nextinsn;
                    }

                    Insn::Goal => {
                        // Keep all but the initial give-up bts.
                        self.bts.truncate(1);
                        return Some(pos);
                    }

                    Insn::JustFail => {
                        break 'backtrack;
                    }
                }
            }

            // This after the backtrack loop.
            // A break 'backtrack will jump here.
            if I::ENABLED && self.poll.tick(countdown) {
                self.bts.truncate(1);
                return None;
            }
            if self.try_backtrack(input, &mut ip, &mut pos, dir) {
                continue 'nextinsn;
            } else {
                // We have exhausted the backtracking stack.
                debug_assert!(self.bts.len() == 1, "Should have exhausted backtrack stack");
                return None;
            }
        }

        // This is outside the nextinsn loop.
        // It is an error to get here.
        // Every instruction should either continue 'nextinsn, or break 'backtrack.
        {
            #![allow(unreachable_code)]
            rs_unreachable!("Should not fall to end of nextinsn loop")
        }
    }
}

#[derive(Debug)]
pub struct BacktrackExecutor<'r, Input: InputIndexer, I: Interrupt = NoInterrupt> {
    input: Input,
    matcher: MatchAttempter<'r, Input, I>,
}

impl<'r, 't, I: Interrupt> BacktrackExecutor<'r, Utf8Input<'t>, I> {
    pub(crate) fn with_interrupt(re: &'r CompiledRegex, text: &'t str, interrupt: I) -> Self {
        let input = Utf8Input::new(text, re.flags.unicode);
        Self {
            input,
            matcher: MatchAttempter::with_interrupt(re, input.left_end(), interrupt),
        }
    }

    /// jlens-regress: an executor that searches with the storage a completed search of the same
    /// text left, instead of allocating its own.
    pub(crate) fn with_storage(
        re: &'r CompiledRegex,
        text: &'t str,
        interrupt: I,
        storage: Storage<'t>,
    ) -> Self {
        Self {
            input: Utf8Input::new(text, re.flags.unicode),
            matcher: MatchAttempter {
                re,
                loop1char_ips: &re.loop1char_ips,
                bts: storage.bts,
                s: storage.s,
                poll: Poll::new(interrupt),
                lookarounds: storage.lookarounds,
            },
        }
    }

    /// jlens-regress: the storage, for the next search of the text. A completed search leaves
    /// it as a new executor has it: one `Exhausted` backtrack and no group set; one that was
    /// interrupted may not, and its storage is not kept.
    pub(crate) fn into_storage(self) -> Storage<'t> {
        Storage {
            bts: self.matcher.bts,
            s: self.matcher.s,
            lookarounds: self.matcher.lookarounds,
        }
    }
}

/// jlens-regress: the storage of a backtracker over a `&str`, which a
/// [`Searcher`](crate::Searcher) keeps between the searches of one text.
#[derive(Debug)]
pub(crate) struct Storage<'t> {
    bts: Vec<BacktrackInsn<Utf8Input<'t>>>,
    s: State<<Utf8Input<'t> as InputIndexer>::Position>,
    lookarounds: Option<Box<Lookarounds<Utf8Input<'t>>>>,
}

// jlens-regress: over any interrupt, for the interruptible searches of UTF-16 text.
#[cfg(feature = "utf16")]
impl<'r, Input: InputIndexer, I: Interrupt> BacktrackExecutor<'r, Input, I> {
    pub(crate) fn new(input: Input, matcher: MatchAttempter<'r, Input, I>) -> Self {
        Self { input, matcher }
    }
}

impl<Input: InputIndexer, I: Interrupt> BacktrackExecutor<'_, Input, I> {
    /// jlens-regress: whether the interrupt stopped the last match.
    pub(crate) fn interrupted(&self) -> bool {
        self.matcher.poll.interrupted
    }

    fn successful_match(&mut self, start: Input::Position, end: Input::Position) -> Match {
        // We want to simultaneously map our groups to offsets, and clear the groups.
        // A for loop is the easiest way to do this while satisfying the borrow checker.
        // TODO: avoid allocating so much.
        let mut captures = Vec::new();
        captures.reserve_exact(self.matcher.s.groups.len());
        for gd in self.matcher.s.groups.iter_mut() {
            captures.push(match gd.as_range() {
                Some(r) => Some(Range {
                    start: self.input.pos_to_offset(r.start),
                    end: self.input.pos_to_offset(r.end),
                }),
                None => None,
            });
            gd.start = None;
            gd.end = None;
        }
        Match {
            range: self.input.pos_to_offset(start)..self.input.pos_to_offset(end),
            captures,
            group_names: self.matcher.re.group_names.clone(),
        }
    }

    /// \return the next match for an anchored regex that only matches at the start.
    /// This avoids any string searching and only tries matching at the given position.
    #[cfg(not(feature = "utf16"))]
    fn next_match_anchored(
        &mut self,
        pos: Input::Position,
        next_start: &mut Option<Input::Position>,
    ) -> Option<Match> {
        let inp = self.input;
        // For anchored regexes, only try matching at the current position
        if let Some(end) = self.matcher.try_at_pos(inp, 0, pos, Forward::new()) {
            // If we matched the empty string, we have to increment.
            if end != pos {
                *next_start = Some(end)
            } else {
                *next_start = inp.next_right_pos(end);
            }
            Some(self.successful_match(pos, end))
        } else {
            // Anchored regex failed to match at this position, no more matches
            None
        }
    }

    /// \return the next match, searching the remaining bytes using the given
    /// prefix searcher to quickly find the first potential match location.
    fn next_match_with_prefix_search<PrefixSearch: bytesearch::ByteSearcher>(
        &mut self,
        mut pos: Input::Position,
        next_start: &mut Option<Input::Position>,
        prefix_search: &PrefixSearch,
    ) -> Option<Match> {
        let inp = self.input;
        loop {
            // Find the next start location, or None if none.
            // Don't try this unless CODE_UNITS_ARE_BYTES - i.e. don't do byte searches
            // on UTF-16 or UCS2.
            if Input::CODE_UNITS_ARE_BYTES {
                pos = inp.find_bytes(pos, prefix_search)?;
            }
            if let Some(end) = self.matcher.try_at_pos(inp, 0, pos, Forward::new()) {
                // If we matched the empty string, we have to increment.
                if end != pos {
                    *next_start = Some(end)
                } else {
                    *next_start = inp.next_right_pos(end);
                }
                return Some(self.successful_match(pos, end));
            }
            if I::ENABLED && self.matcher.poll.interrupted {
                return None;
            }
            // Didn't find it at this position, try the next one.
            pos = inp.next_right_pos(pos)?;
        }
    }
}

impl<Input: InputIndexer, I: Interrupt + core::fmt::Debug> exec::MatchProducer
    for BacktrackExecutor<'_, Input, I>
{
    type Position = Input::Position;

    fn initial_position(&self, offset: usize) -> Option<Self::Position> {
        self.input.try_move_right(self.input.left_end(), offset)
    }

    fn next_match(
        &mut self,
        pos: Input::Position,
        next_start: &mut Option<Input::Position>,
    ) -> Option<Match> {
        // When UTF-16 support is active prefix search is not used due to the different encoding.
        #[cfg(feature = "utf16")]
        return self.next_match_with_prefix_search(pos, next_start, &bytesearch::EmptyString {});

        #[cfg(not(feature = "utf16"))]
        match &self.matcher.re.start_pred {
            StartPredicate::Arbitrary => {
                self.next_match_with_prefix_search(pos, next_start, &bytesearch::EmptyString {})
            }
            StartPredicate::StartAnchored => self.next_match_anchored(pos, next_start),
            StartPredicate::ByteSet1(bytes) => {
                self.next_match_with_prefix_search(pos, next_start, bytes)
            }
            StartPredicate::ByteSet2(bytes) => {
                self.next_match_with_prefix_search(pos, next_start, bytes)
            }
            StartPredicate::ByteSet3(bytes) => {
                self.next_match_with_prefix_search(pos, next_start, bytes)
            }
            StartPredicate::ByteSeq(bytes) => {
                self.next_match_with_prefix_search(pos, next_start, bytes.as_ref())
            }
            StartPredicate::ByteBracket(bitmap) => {
                self.next_match_with_prefix_search(pos, next_start, bitmap)
            }
        }
    }
}

impl<'r, 't> exec::Executor<'r, 't> for BacktrackExecutor<'r, Utf8Input<'t>> {
    type AsAscii = BacktrackExecutor<'r, AsciiInput<'t>>;

    fn new(re: &'r CompiledRegex, text: &'t str) -> Self {
        let input = Utf8Input::new(text, re.flags.unicode);
        Self {
            input,
            matcher: MatchAttempter::new(re, input.left_end()),
        }
    }
}

impl<'r, 't> exec::Executor<'r, 't> for BacktrackExecutor<'r, AsciiInput<'t>> {
    type AsAscii = BacktrackExecutor<'r, AsciiInput<'t>>;

    fn new(re: &'r CompiledRegex, text: &'t str) -> Self {
        let input = AsciiInput::new(text, re.flags.unicode);
        Self {
            input,
            matcher: MatchAttempter::new(re, input.left_end()),
        }
    }
}
