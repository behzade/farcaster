use super::*;

#[test]
fn reported_errors_return_failure_even_when_stderr_write_succeeds() {
    assert_eq!(
        fail_to(Vec::new(), "failed"),
        std::process::ExitCode::from(1)
    );
}
