#[path = "support/transport_matrix/mod.rs"]
mod transport_matrix;

#[test]
#[ignore]
fn matrix_probe() {
    transport_matrix::run_probe();
}

#[test]
#[ignore]
fn matrix_smoke() {
    transport_matrix::run_smoke();
}

#[test]
#[ignore]
fn matrix_perf_smoke() {
    transport_matrix::run_perf_smoke();
}

#[test]
#[ignore]
fn matrix_stability() {
    transport_matrix::run_stability();
}

#[test]
#[ignore]
fn matrix_cleanup() {
    transport_matrix::run_cleanup();
}
