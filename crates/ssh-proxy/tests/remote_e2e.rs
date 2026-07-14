#[path = "support/remote_e2e/mod.rs"]
mod remote_e2e;

#[test]
#[ignore]
fn remote_probe() {
    remote_e2e::run_probe();
}

#[test]
#[ignore]
fn remote_smoke() {
    remote_e2e::run_smoke();
}

#[test]
#[ignore]
fn remote_full() {
    remote_e2e::run_full();
}
