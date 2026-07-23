use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wslc-compose"))
}

#[test]
fn config_lists_services_in_dependency_order() {
    let temp = tempfile::tempdir().unwrap();
    let compose = temp.path().join("compose.yaml");
    std::fs::write(
        &compose,
        "services:\n  db:\n    image: alpine:latest\n  api:\n    image: alpine:latest\n    depends_on: [db]\n",
    )
    .unwrap();

    let output = binary()
        .args(["-f", compose.to_str().unwrap(), "config", "--services"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "db\napi\n");
}

#[test]
fn config_reports_required_interpolation_variables() {
    let temp = tempfile::tempdir().unwrap();
    let compose = temp.path().join("compose.yaml");
    std::fs::write(
        &compose,
        "services:\n  app:\n    image: ${WSLC_COMPOSE_TEST_IMAGE:?set image}\n",
    )
    .unwrap();

    let output = binary()
        .args(["-f", compose.to_str().unwrap(), "config", "--quiet"])
        .env_remove("WSLC_COMPOSE_TEST_IMAGE")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("set image"));
}

#[test]
fn short_version_does_not_require_a_compose_file_or_wslc() {
    let output = binary().args(["version", "--short"]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0.1.0\n");
}
