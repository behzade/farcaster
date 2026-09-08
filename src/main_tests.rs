use super::*;

#[test]
fn reported_errors_return_failure_even_when_stderr_write_succeeds() {
    assert_eq!(
        fail_to(Vec::new(), "failed"),
        std::process::ExitCode::from(1)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn native_linux_disables_dzn_without_overriding_driver_configuration() {
    assert!(should_disable_dzn(false, |_| false));
    assert!(!should_disable_dzn(true, |_| false));
    for configured_name in VULKAN_DRIVER_CONFIGURATION {
        assert!(!should_disable_dzn(false, |name| name == configured_name));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn recognizes_wsl_kernel_versions() {
    assert!(kernel_version_reports_wsl(
        "5.15.90.1-microsoft-standard-WSL2"
    ));
    assert!(kernel_version_reports_wsl("4.4.0-Microsoft"));
    assert!(!kernel_version_reports_wsl("7.1.4"));
}
