//! The slow-op ledger — where one served op's wall clock went, kept on the
//! serving thread whatever `MRD_TIMING` says, for the engine's `slow op` line
//! (`docs/run-plane.md` § The slow-op line).
//!
//! The `MRD_TIMING` sink answers "where does the time go" for an operator who
//! switched it on BEFORE the incident. A stall on a production engine, where
//! the switch is off, left one line naming the op and its duration and nothing
//! about which wait held it. The ledger closes that gap: while a served op is
//! in dispatch, every phase completed on its thread adds its microseconds
//! here, and the engine prints the ledger only when the op crossed the
//! threshold — whether it will is known only at the end, so the ledger is
//! kept on every op.
//!
//! **Cost.** A thread with no open ledger reads no clock for a ledger-only
//! [`phase`], and [`crate::phase`] reads one only when the sink is on, as
//! before. Inside a ledger each completed phase is two clock reads and one
//! linear scan of a vector holding a few names. Sites are coarse — a lock, a
//! pass, a row — never per member.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::marker::PhantomData;
use std::time::Duration;

use crate::Phase;

/// One name's total inside a ledger.
#[derive(Debug, Clone, Copy)]
struct Charge {
    name: &'static str,
    us: u64,
    count: u32,
}

thread_local! {
    /// The open ledger on this thread, if a served op is in dispatch here.
    static LEDGER: RefCell<Option<Vec<Charge>>> = const { RefCell::new(None) };
}

/// The open ledger of one served op. Opened by [`begin`]; closed when it
/// drops. Not `Send`: the ledger lives in this thread's local storage, and
/// the guard must close it on the thread that opened it.
#[derive(Debug)]
#[must_use = "the ledger closes when this guard drops"]
pub struct Ledger {
    /// `false` for a nested [`begin`]: the outer op owns the ledger, closes
    /// it, and prints it.
    owner: bool,
    _thread_bound: PhantomData<*const ()>,
}

/// Open a ledger for the op this thread is about to serve. A ledger already
/// open on this thread (an op served inside another) stays the one charged,
/// and the returned guard neither resets nor closes it.
pub fn begin() -> Ledger {
    let owner = LEDGER
        .try_with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_some() {
                false
            } else {
                *slot = Some(Vec::new());
                true
            }
        })
        .unwrap_or(false);
    Ledger {
        owner,
        _thread_bound: PhantomData,
    }
}

impl Ledger {
    /// The ledger as the slow-op line prints it: `name=us` pairs, space
    /// separated, in the order each name first completed; `xN` is appended
    /// to the value (`name=usxN`) when the name completed N > 1 times. Empty
    /// when nothing was charged.
    #[must_use]
    pub fn render(&self) -> String {
        LEDGER
            .try_with(|cell| {
                let slot = cell.borrow();
                let mut out = String::new();
                for charge in slot.iter().flatten() {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    let _ = write!(out, "{}={}", charge.name, charge.us);
                    if charge.count > 1 {
                        let _ = write!(out, "x{}", charge.count);
                    }
                }
                out
            })
            .unwrap_or_default()
    }
}

impl Drop for Ledger {
    fn drop(&mut self) {
        if self.owner {
            let _ = LEDGER.try_with(|cell| cell.borrow_mut().take());
        }
    }
}

/// Is a ledger open on this thread?
#[inline]
pub(crate) fn active() -> bool {
    LEDGER
        .try_with(|cell| cell.borrow().is_some())
        .unwrap_or(false)
}

/// Add one completed phase to this thread's open ledger; nothing when none
/// is open.
pub(crate) fn charge(name: &'static str, elapsed: Duration) {
    let us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    let _ = LEDGER.try_with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(charges) = slot.as_mut() else {
            return;
        };
        if let Some(existing) = charges.iter_mut().find(|c| c.name == name) {
            existing.us = existing.us.saturating_add(us);
            existing.count = existing.count.saturating_add(1);
        } else {
            charges.push(Charge { name, us, count: 1 });
        }
    });
}

/// Start a phase measured for the ledger ONLY — never written to the
/// `MRD_TIMING` sink. For waits taken so often (every memo borrow, every
/// barrier) that a sink line each would drown the sink. Outside a ledger
/// this reads no clock and its stop does nothing.
#[inline]
pub fn phase(name: &'static str) -> Phase {
    Phase {
        name,
        started: active().then(crate::now),
        sink: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{begin, phase};
    use std::time::Duration;

    #[test]
    fn a_ledger_sums_repeats_and_keeps_first_completion_order() {
        let ledger = begin();
        super::charge("memo.wait", Duration::from_micros(5));
        super::charge("currency.vouched", Duration::from_micros(40));
        super::charge("memo.wait", Duration::from_micros(7));
        assert_eq!(ledger.render(), "memo.wait=12x2 currency.vouched=40");
    }

    #[test]
    fn nothing_is_kept_outside_a_ledger() {
        super::charge("memo.wait", Duration::from_micros(5));
        let span = phase("memo.wait");
        assert!(span.started.is_none(), "no ledger: no clock read");
        span.stop();
        let ledger = begin();
        assert_eq!(ledger.render(), "", "a fresh ledger carries nothing");
    }

    #[test]
    fn a_nested_begin_charges_the_outer_ledger_and_leaves_it_open() {
        let outer = begin();
        {
            let inner = begin();
            super::charge("run.row", Duration::from_micros(3));
            drop(inner);
        }
        super::charge("run.row", Duration::from_micros(4));
        assert_eq!(outer.render(), "run.row=7x2");
        drop(outer);
        assert!(!super::active(), "the owner closes the ledger");
    }

    /// A sink phase completed inside a ledger lands there with the sink off:
    /// that is what the production slow-op line rests on.
    #[test]
    fn a_sink_phase_feeds_the_ledger_with_the_sink_off() {
        let _ = crate::SINK.set(crate::Sink::Off);
        assert!(!crate::on());
        let ledger = begin();
        let span = crate::phase("currency.vouched");
        assert!(span.started.is_some(), "inside a ledger the clock is read");
        span.stop();
        crate::phase("currency.floor").stop_as("currency.floor.no_feed");
        let line = ledger.render();
        assert!(line.starts_with("currency.vouched="), "{line}");
        assert!(line.contains(" currency.floor.no_feed="), "{line}");
    }
}
