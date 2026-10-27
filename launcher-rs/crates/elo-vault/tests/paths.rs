//! Where the keystore lives, read from the environment.
//!
//! One test in its own file, because it sets process environment variables:
//! a test binary runs its tests on threads, and a variable changed under a
//! neighbour's `getenv` is a race no assertion would catch.

/// **An empty or relative `XDG_CONFIG_HOME` is ignored**, as the XDG spec
/// says. Honoured, `""` + `elo/keystore.json` is a path under whatever
/// directory the launcher was started in — the key lands beside a download,
/// and the next run from anywhere else finds no account.
#[cfg(not(windows))]
#[test]
fn the_keystore_never_lands_under_the_current_directory() {
    std::env::remove_var("ELO_KEYSTORE");
    std::env::set_var("HOME", "/home/someone");
    for ignored in ["", "relative/config", "./config"] {
        std::env::set_var("XDG_CONFIG_HOME", ignored);
        assert_eq!(
            elo_vault::keystore_path(),
            std::path::PathBuf::from("/home/someone/.config/elo/keystore.json"),
            "XDG_CONFIG_HOME={ignored:?} must fall back to $HOME"
        );
    }
    std::env::set_var("XDG_CONFIG_HOME", "/srv/conf");
    assert_eq!(
        elo_vault::keystore_path(),
        std::path::PathBuf::from("/srv/conf/elo/keystore.json"),
        "an absolute one is still honoured"
    );
}
