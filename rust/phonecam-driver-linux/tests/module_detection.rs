use std::fs;

use phonecam_driver_linux::{
    ensure_v4l2loopback_loaded_in, is_v4l2loopback_loaded_in, DriverError,
};

#[test]
fn module_detection_checks_sys_module_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let module_root = temp.path().join("sys/module");
    fs::create_dir_all(module_root.join("v4l2loopback")).expect("create module dir");

    assert!(is_v4l2loopback_loaded_in(&module_root));
}

#[test]
fn module_not_loaded_returns_structured_error_with_installation_help() {
    let temp = tempfile::tempdir().expect("tempdir");
    let module_root = temp.path().join("sys/module");
    fs::create_dir_all(&module_root).expect("create module root");

    let err = ensure_v4l2loopback_loaded_in(&module_root).expect_err("must fail");
    let DriverError::ModuleNotLoaded { instructions } = err else {
        panic!("unexpected error variant");
    };

    assert!(instructions.contains("apt-get install v4l2loopback-dkms"));
    assert!(instructions.contains("dnf install v4l2loopback"));
    assert!(instructions.contains("pacman -S v4l2loopback-dkms"));
    assert!(instructions.contains("exclusive_caps=1"));
}
