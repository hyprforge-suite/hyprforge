//! Manual, read-only smoke test against a real running compositor. Not run
//! by default (`cargo test`) — only via `cargo test --test live_smoke --
//! --ignored`, and only ever calls `list_outputs`, never
//! `apply_configuration`, so it can't disturb whatever session it runs in.
use hyprforge_displayd::backend::wlr::WlrBackend;
use hyprforge_displayd::backend::OutputBackend;

#[test]
#[ignore]
fn connects_and_lists_real_outputs() {
    let backend = WlrBackend::connect().expect("failed to connect to compositor");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let outputs = backend.list_outputs().expect("failed to list outputs");
    eprintln!("{outputs:#?}");
    assert!(!outputs.is_empty(), "expected at least one connected output");
}
