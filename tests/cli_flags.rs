use std::process::Command;

#[test]
fn root_and_ctl_help_version_aliases_work_without_claude() {
    let home = tempfile::tempdir().unwrap();
    for flag in ["--help", "--h", "-help", "-h"] {
        for prefix in [vec![], vec!["ctl"]] {
            let result = Command::new(env!("CARGO_BIN_EXE_ccword"))
                .env("HOME", home.path())
                .args(&prefix)
                .arg(flag)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{prefix:?} {flag}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(String::from_utf8_lossy(&result.stdout).contains("ccword ctl config"));
        }
    }
    for flag in ["--version", "--v", "-version", "-v", "-V"] {
        for prefix in [vec![], vec!["ctl"]] {
            let result = Command::new(env!("CARGO_BIN_EXE_ccword"))
                .env("HOME", home.path())
                .args(&prefix)
                .arg(flag)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{prefix:?} {flag}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&result.stdout).trim(),
                "ccword 0.1.0"
            );
        }
    }
}

#[test]
fn update_reports_missing_cargo_without_replacing_installed_binary() {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join("ccword");
    std::fs::copy(env!("CARGO_BIN_EXE_ccword"), &exe).unwrap();
    let before = std::fs::read(&exe).unwrap();
    for flag in ["--update", "--u", "-update", "-u"] {
        let result = Command::new(&exe)
            .env("PATH", &bin)
            .arg(flag)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("Cargo is required"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(&exe).unwrap(), before);
    }
}

#[test]
fn source_build_update_refuses_to_overwrite_build_output() {
    let result = Command::new(env!("CARGO_BIN_EXE_ccword"))
        .args(["ctl", "update"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("update requires an installed ccword"),
        "{error}"
    );
    assert!(error.contains("cargo install --path"), "{error}");
}
