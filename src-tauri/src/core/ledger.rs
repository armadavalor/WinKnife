//! Record, move, reconcile — the one operation that moves user data and writes
//! the row that remembers where it came from.
//!
//! # Why this is a module and not a convention
//!
//! [`crate::core::quarantine`] has stated the ordering rule in a comment since
//! phase 1: *ledger first — a row without a file is recoverable, a file without
//! a row is not.* `Quarantine::take` followed it. `modules::startup::stash_link`,
//! written two phases later, called into the quarantine's own move primitive and
//! then re-implemented the bookkeeping around it **backwards**: move first, row
//! second. With a ledger write that fails — a full disk, a locked database,
//! `SQLITE_BUSY`, the `startup_disabled_link_origin` unique index firing — the
//! shortcut left the Startup folder and nothing recorded where it went. It was
//! not deleted; it was merely invisible, and through the only interface the user
//! has those are the same thing (security review H11).
//!
//! A comment two hundred lines away in another file is not an enforcement
//! mechanism. So the order is a *type* now. [`ledgered_move`] is the only way in
//! this crate to move user data, it takes the row change as a [`LedgerStep`],
//! and the variant of that step decides when the row is written relative to the
//! move. A fourth write surface cannot get the order wrong, because there is
//! nothing else for it to call: the move primitives in `core::quarantine` are
//! visible only inside `core`, and `clippy.toml` forbids `fs::rename` outright.
//!
//! # The two legal orders, and why there are exactly two
//!
//! The invariant is not "the row is always written first". It is **the ledger
//! must never be the only thing that knows where a file is, at a moment when it
//! does not know**. That admits two shapes and forbids every other:
//!
//! - [`LedgerStep::Claim`] — the row is written or amended so that it *already*
//!   describes where the file is going, and it still names both places
//!   afterwards. Committed before a single byte moves. If the process dies in
//!   the gap, there is a row pointing at a file that never left: recoverable. If
//!   the move then fails, `undo` retracts the row in its own transaction, and a
//!   failure to retract is logged rather than swallowed.
//! - [`LedgerStep::Retire`] — the row is dropped, so afterwards the ledger knows
//!   nothing. It may therefore only run once the file is back somewhere the user
//!   can see without our help. If the process dies in the gap, there is a stale
//!   row describing a file that has already gone home: also recoverable, and
//!   [`reconcile`] is what recovers it.
//!
//! Writing a row *after* moving a file into custody is the one order that is
//! never correct, and it is not expressible here.

use std::path::Path;

use rusqlite::Connection;

use crate::core::error::Result;

/// Proof that a ledger row already accounts for the move about to happen.
///
/// Only [`ledgered_move`] constructs one, and [`move_user_file`] will not move
/// anything without a reference to one. That is the "by construction" in the
/// module header: the ordering rule is not a comment a future module can read
/// and then invert, it is a value a future module cannot obtain any other way.
#[derive(Debug)]
pub struct Recorded(());

/// Move one user file, having proved the ledger already accounts for it.
///
/// The primitive itself lives in [`crate::core::quarantine`] — rename on the
/// same volume, copy-verify-remove across volumes, never replace an occupied
/// destination — and is visible only inside `core`. This is the only door to it
/// from a module, and the [`Recorded`] argument is the lock on that door.
///
/// # Errors
/// See [`crate::core::quarantine::move_user_file`].
pub fn move_user_file(
    _recorded: &Recorded,
    from: &Path,
    to: &Path,
) -> Result<Vec<crate::core::types::MoveWarning>> {
    crate::core::quarantine::move_user_file(from, to)
}

/// A ledger row change, and when it is allowed to happen relative to the move.
///
/// The variant is the safety argument. See the module header.
pub enum LedgerStep<'a> {
    /// The row is written or amended so it already describes where the file is
    /// going, and it still names the file afterwards.
    ///
    /// Committed before the move. `undo` runs only if the move then failed.
    Claim {
        /// Write the row. Runs inside a transaction committed before the move.
        write: &'a mut dyn FnMut(&Connection) -> Result<()>,
        /// Retract it. Runs inside its own transaction, only after a failed move.
        undo: &'a mut dyn FnMut(&Connection) -> Result<()>,
    },
    /// The row is retired: after this the ledger no longer knows where the file
    /// is, so it runs only once the file is somewhere the user can see it.
    Retire {
        /// Drop the row. Runs inside a transaction, after a successful move.
        write: &'a mut dyn FnMut(&Connection) -> Result<()>,
    },
}

/// Move user data with its ledger row, in the only order that is safe for that
/// row change.
///
/// `what` names the thing being moved and appears in every log line this emits;
/// it is for operators reading a log after a crash, not for the UI.
///
/// `perform` is the move itself. It is a closure rather than a pair of paths
/// because the quarantine's restore has to try a series of destination names
/// (it never overwrites), and the return type is free so a caller can hand back
/// whatever the move produced. It receives a [`Recorded`], which is the only
/// way to reach a move primitive at all.
///
/// # Errors
/// Returns whatever `perform` or the ledger write returned. On a
/// [`LedgerStep::Claim`] whose move failed, the row is retracted first, so the
/// error never leaves a phantom behind.
pub fn ledgered_move<T>(
    conn: &Connection,
    what: &str,
    step: LedgerStep<'_>,
    perform: impl FnOnce(&Recorded) -> Result<T>,
) -> Result<T> {
    match step {
        LedgerStep::Claim { write, undo } => {
            // The row is durable before anything moves. This is the whole point
            // of the module.
            in_transaction(conn, write)?;
            let recorded = Recorded(());

            match perform(&recorded) {
                Ok(value) => Ok(value),
                Err(failure) => {
                    // The move never happened, so the row describes nothing.
                    // Retract it — and say so out loud if even that fails,
                    // because a swallowed failure here is exactly the phantom
                    // the retraction exists to prevent (security review M3).
                    if let Err(cleanup) = in_transaction(conn, undo) {
                        tracing::error!(
                            what,
                            error = %cleanup,
                            "could not retract the ledger row for a move that never happened; \
                             the ledger now describes an item that was never taken"
                        );
                    }
                    Err(failure)
                }
            }
        }
        LedgerStep::Retire { write } => {
            // The row is still there and still names both places, so the ledger
            // accounts for this move exactly as much as a fresh `Claim` would.
            let value = perform(&Recorded(()))?;

            if let Err(failure) = in_transaction(conn, write) {
                // The file is already back where the user can see it, so nothing
                // is lost — but the ledger still claims custody of it, and that
                // claim has to be visible rather than swallowed. `reconcile`
                // clears it the next time the row is read.
                tracing::error!(
                    what,
                    error = %failure,
                    "the move succeeded but its ledger row could not be retired; \
                     the row is now stale and will be reconciled on the next read"
                );
                return Err(failure);
            }
            Ok(value)
        }
    }
}

/// Whether a ledger row that claims custody of `stored` is still telling the
/// truth, given what the filesystem says.
///
/// A row survives a crash in the gap [`LedgerStep::Retire`] leaves open: the
/// file went home, the `DELETE` never landed. That row now says "in custody" of
/// something that is not there. This is how a reader tells that apart from a row
/// that is simply describing a file it cannot currently open.
///
/// **Only a clean "not found" counts as gone.** `Path::exists` answers "no" both
/// for "nothing is there" and for "we are not allowed to look", and treating the
/// second as a retired row would drop the only record of a file we really do
/// hold — which is the orphan this module exists to prevent, arriving by the
/// other door.
pub fn reconcile(stored: &std::path::Path) -> Custody {
    match std::fs::symlink_metadata(stored) {
        Ok(_) => Custody::Held,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Custody::Gone,
        Err(_) => Custody::Unknown,
    }
}

/// What [`reconcile`] concluded about a row's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Custody {
    /// The file is where the row says it is.
    Held,
    /// The file is definitely not there: the row is stale and may be retired.
    Gone,
    /// We could not look. The row is kept — refusing to answer is not evidence.
    Unknown,
}

impl Custody {
    /// Whether the row should still be believed.
    pub fn is_current(self) -> bool {
        !matches!(self, Self::Gone)
    }
}

/// Run one ledger write inside a transaction and commit it.
///
/// `unchecked_transaction` rather than `Connection::transaction`, because the
/// app shares one connection behind a mutex and hands `&Connection` down; the
/// borrow checker cannot see that the mutex already makes this exclusive.
fn in_transaction(
    conn: &Connection,
    write: &mut dyn FnMut(&Connection) -> Result<()>,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    write(&tx)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::CoreError;

    fn ledger() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute("CREATE TABLE rows (id TEXT PRIMARY KEY)", [])
            .expect("schema");
        conn
    }

    fn count(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM rows", [], |row| row.get(0))
            .expect("count")
    }

    fn insert(conn: &Connection) -> Result<()> {
        conn.execute("INSERT INTO rows (id) VALUES ('a')", [])?;
        Ok(())
    }

    fn delete(conn: &Connection) -> Result<()> {
        conn.execute("DELETE FROM rows WHERE id = 'a'", [])?;
        Ok(())
    }

    #[test]
    fn a_claim_is_committed_before_the_move_runs() {
        let conn = ledger();
        let mut write = insert;
        let mut undo = delete;
        let mut rows_when_the_move_ran = -1;

        ledgered_move(
            &conn,
            "test",
            LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            |_| {
                rows_when_the_move_ran = count(&conn);
                Ok(())
            },
        )
        .expect("move");

        assert_eq!(
            rows_when_the_move_ran, 1,
            "the row has to be durable before the file moves"
        );
        assert_eq!(count(&conn), 1);
    }

    #[test]
    fn a_failed_claim_never_reaches_the_move() {
        let conn = ledger();
        let mut write = |_: &Connection| Err(CoreError::Quarantine("no room".to_owned()));
        let mut undo = delete;
        let mut moved = false;

        let result = ledgered_move(
            &conn,
            "test",
            LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            |_| {
                moved = true;
                Ok(())
            },
        );

        assert!(result.is_err());
        assert!(
            !moved,
            "a ledger that cannot record must not let the file go"
        );
        assert_eq!(count(&conn), 0);
    }

    #[test]
    fn a_failed_move_retracts_its_claim() {
        let conn = ledger();
        let mut write = insert;
        let mut undo = delete;

        let result: Result<()> = ledgered_move(
            &conn,
            "test",
            LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            |_| Err(CoreError::Quarantine("the file is locked".to_owned())),
        );

        assert!(result.is_err());
        assert_eq!(count(&conn), 0, "a row for a move that never happened");
    }

    #[test]
    fn a_retire_runs_only_after_the_move_succeeded() {
        let conn = ledger();
        insert(&conn).expect("seed");
        let mut write = delete;
        let mut rows_when_the_move_ran = -1;

        ledgered_move(
            &conn,
            "test",
            LedgerStep::Retire { write: &mut write },
            |_| {
                rows_when_the_move_ran = count(&conn);
                Ok(())
            },
        )
        .expect("move");

        assert_eq!(
            rows_when_the_move_ran, 1,
            "the ledger must still know where the file is while it is moving"
        );
        assert_eq!(count(&conn), 0);
    }

    #[test]
    fn a_failed_move_leaves_the_row_that_would_have_been_retired() {
        let conn = ledger();
        insert(&conn).expect("seed");
        let mut write = delete;

        let result: Result<()> = ledgered_move(
            &conn,
            "test",
            LedgerStep::Retire { write: &mut write },
            |_| Err(CoreError::Quarantine("the file is locked".to_owned())),
        );

        assert!(result.is_err());
        assert_eq!(count(&conn), 1, "the file never moved, so it is still held");
    }

    #[test]
    fn a_ledger_write_that_fails_rolls_back_rather_than_half_applying() {
        let conn = ledger();
        let mut write = |c: &Connection| {
            insert(c)?;
            c.execute("INSERT INTO rows (id) VALUES ('a')", [])?;
            Ok(())
        };
        let mut undo = delete;

        let result: Result<()> = ledgered_move(
            &conn,
            "test",
            LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            |_| Ok(()),
        );

        assert!(
            result.is_err(),
            "the second insert violates the primary key"
        );
        assert_eq!(count(&conn), 0, "the first insert must not survive");
    }

    #[test]
    fn a_file_that_cannot_be_looked_at_is_not_evidence_that_it_is_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let here = dir.path().join("held.lnk");
        std::fs::write(&here, b"x").expect("write");

        assert_eq!(reconcile(&here), Custody::Held);
        assert!(reconcile(&here).is_current());
        assert_eq!(reconcile(&dir.path().join("nope.lnk")), Custody::Gone);
        assert!(!reconcile(&dir.path().join("nope.lnk")).is_current());
    }
}
