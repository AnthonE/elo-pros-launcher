//! The Windows door's access rules, read off its source — the one half of the
//! transport no Linux test run can execute.
//!
//! l-keys#6: a null security descriptor gave the pipe the default DACL, which
//! also lets Everyone and anonymous open it for reading (enough to hold every
//! instance a player's games need), and creating an instance per accept let
//! the name lapse between connections for another user to take. Each law here
//! failed against that code.

const SRC: &str = include_str!("../src/transport.rs");

fn windows_half() -> &'static str {
    let at = SRC.find("mod imp_windows").expect("the windows transport exists");
    &SRC[at..]
}

#[test]
fn the_pipe_is_created_with_an_owner_only_dacl() {
    let w = windows_half();
    assert!(w.contains(r#""D:P(A;;GA;;;OW)(A;;GA;;;SY)""#),
            "the door's DACL names its owner and SYSTEM, protected from inheritance");
    assert!(w.contains("ConvertStringSecurityDescriptorToSecurityDescriptorW"));
    // The security argument to CreateNamedPipeW is the descriptor, never null.
    let call = &w[w.rfind("CreateNamedPipeW(").expect("the call site")..];
    let call = &call[..call.find(")\n").unwrap()];
    assert!(call.contains("&mut sa"), "CreateNamedPipeW gets the DACL: {call}");
    assert!(!call.contains("null_mut()"), "and no default descriptor: {call}");
}

#[test]
fn the_name_is_claimed_at_bind_and_never_lapses() {
    let w = windows_half();
    assert!(w.contains("FILE_FLAG_FIRST_PIPE_INSTANCE"),
            "the first instance fails if someone else already made the name");
    let bind = &w[w.find("pub fn bind").unwrap()..w.find("pub fn accept").unwrap()];
    assert!(bind.contains("instance(&name, true)"), "bind claims the name: {bind}");
    let accept = &w[w.find("pub fn accept").unwrap()..w.find("pub fn endpoint").unwrap()];
    let next = accept.find("instance(&self.name, false)").expect("accept makes the next instance");
    let handed = accept.find("from_raw_handle(handle)").expect("accept hands one off");
    assert!(next < handed, "the next instance exists before the connected one is handed off");
}

#[test]
fn a_failed_next_instance_never_leaves_the_connected_handle_pending() {
    // If the next instance cannot be made, the connected handle is still the
    // listener's pending one; returning with the client attached hands the
    // same handle to a second File on the next accept (two owners, two
    // closes). The client is dropped and the instance reset instead.
    let w = windows_half();
    let accept = &w[w.find("pub fn accept").unwrap()..w.find("pub fn endpoint").unwrap()];
    let next = &accept[accept.find("instance(&self.name, false)").unwrap()..];
    let failed = &next[next.find("Err(e)").expect("the failure is handled, not `?`")..];
    let failed = &failed[..failed.find("return Err(e)").expect("and reported")];
    assert!(failed.contains("DisconnectNamedPipe(handle)"),
            "the connected client is dropped before the error returns: {failed}");
}
