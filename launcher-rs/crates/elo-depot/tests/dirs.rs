//! Where the client keeps its state, read from the environment.
//!
//! One test in its own file, because it sets process environment variables:
//! a test binary runs its tests on threads, and a variable changed under a
//! neighbour's `getenv` is a race no assertion would catch.

/// **An empty or relative XDG directory is ignored**, as the XDG spec says.
/// Honoured, it is a path under whatever directory the launcher was started
/// in — sessions and games land beside a download, and the next run from
/// anywhere else finds none of them.
#[cfg(not(windows))]
#[test]
fn an_xdg_dir_counts_only_when_it_is_absolute() {
    use elo_depot::dirs::{home_based, state_root};
    use std::path::PathBuf;

    std::env::remove_var("ELO_STATE");
    std::env::set_var("HOME", "/home/someone");
    for var in ["XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
        for ignored in ["", "relative/dir", "./dir"] {
            std::env::set_var(var, ignored);
            assert_eq!(
                home_based(var, ".local/x"),
                PathBuf::from("/home/someone/.local/x"),
                "{var}={ignored:?} must fall back to $HOME"
            );
        }
        std::env::set_var(var, "/srv/x");
        assert_eq!(home_based(var, ".local/x"), PathBuf::from("/srv/x"), "an absolute {var} is honoured");
    }
    std::env::set_var("XDG_STATE_HOME", "");
    assert_eq!(state_root(), PathBuf::from("/home/someone/.local/state/elo"));
}
