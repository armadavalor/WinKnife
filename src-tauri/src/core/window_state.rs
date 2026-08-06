//! Remember where the window was.
//!
//! A utility that is opened, used and closed several times a day should come
//! back where it was left. Done here rather than with a plugin because it is
//! twenty lines against the `settings` table this app already has, and the
//! alternative is a dependency for one struct.
//!
//! Deliberately forgiving: a saved position that no longer exists — a monitor
//! that has been unplugged, a resolution that changed — is discarded rather
//! than restored, because a window opening off-screen looks exactly like an app
//! that failed to start.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::core::error::Result;
use crate::core::settings;

/// The saved geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    /// Physical pixels.
    pub width: u32,
    /// Physical pixels.
    pub height: u32,
    /// Physical pixels from the left of the virtual desktop.
    pub x: i32,
    /// Physical pixels from the top of the virtual desktop.
    pub y: i32,
    /// Whether it was maximised.
    pub maximized: bool,
}

/// Smallest window worth restoring; below this something went wrong.
pub const MIN_SIZE: u32 = 400;

impl WindowState {
    /// Whether this is worth restoring.
    ///
    /// Guards the two shapes that produce an unusable window: a size of nearly
    /// nothing (a minimised window reports one on some Windows builds) and a
    /// position far outside any plausible desktop.
    pub fn is_usable(&self) -> bool {
        const FAR_AWAY: i32 = 32_000;
        self.width >= MIN_SIZE
            && self.height >= MIN_SIZE
            && self.x > -FAR_AWAY
            && self.y > -FAR_AWAY
            && self.x < FAR_AWAY
            && self.y < FAR_AWAY
    }
}

/// Read the saved geometry, or `None` when there is none worth using.
///
/// # Errors
/// Returns an error if the settings table cannot be read.
pub fn load(conn: &Connection) -> Result<Option<WindowState>> {
    Ok(
        settings::get_json::<WindowState>(conn, settings::KEY_WINDOW_STATE)?
            .filter(WindowState::is_usable),
    )
}

/// Save the geometry.
///
/// # Errors
/// Returns an error if the settings table cannot be written.
pub fn save(conn: &Connection, state: &WindowState) -> Result<()> {
    if !state.is_usable() {
        // Refusing to save beats saving something that will be refused on the
        // way back in: the previous good position survives.
        return Ok(());
    }
    settings::set_json(conn, settings::KEY_WINDOW_STATE, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn state() -> WindowState {
        WindowState {
            width: 1100,
            height: 720,
            x: 120,
            y: 80,
            maximized: false,
        }
    }

    #[test]
    fn geometry_round_trips() {
        let conn = db::open_in_memory().expect("db");
        assert_eq!(load(&conn).expect("read"), None);
        save(&conn, &state()).expect("save");
        assert_eq!(load(&conn).expect("read"), Some(state()));
    }

    #[test]
    fn a_tiny_window_is_neither_saved_nor_restored() {
        let conn = db::open_in_memory().expect("db");
        let tiny = WindowState {
            width: 10,
            height: 10,
            ..state()
        };
        save(&conn, &tiny).expect("save");
        assert_eq!(
            load(&conn).expect("read"),
            None,
            "an app must not reopen as a ten-pixel square"
        );
    }

    #[test]
    fn a_position_on_a_monitor_that_is_gone_is_discarded() {
        let conn = db::open_in_memory().expect("db");
        settings::set_json(
            &conn,
            settings::KEY_WINDOW_STATE,
            &WindowState {
                x: -40_000,
                ..state()
            },
        )
        .expect("write");
        assert_eq!(
            load(&conn).expect("read"),
            None,
            "a window opening off-screen looks like an app that failed to start"
        );
    }

    #[test]
    fn a_corrupt_value_falls_back_rather_than_failing() {
        let conn = db::open_in_memory().expect("db");
        settings::set_raw(&conn, settings::KEY_WINDOW_STATE, "{ nope }").expect("write");
        assert_eq!(load(&conn).expect("read"), None);
    }

    #[test]
    fn saving_a_bad_state_keeps_the_last_good_one() {
        let conn = db::open_in_memory().expect("db");
        save(&conn, &state()).expect("save");
        save(
            &conn,
            &WindowState {
                width: 1,
                height: 1,
                ..state()
            },
        )
        .expect("save");
        assert_eq!(load(&conn).expect("read"), Some(state()));
    }
}
