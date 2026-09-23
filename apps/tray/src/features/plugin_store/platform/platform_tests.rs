use super::*;

#[test]
fn host_os_data_preserves_contract_tokens_and_labels() {
    let cases = [
        (HostOs::Linux, "linux", "Linux", ""),
        (HostOs::Macos, "macos", "macOS", ""),
        (HostOs::Windows, "windows", "Windows", ".exe"),
        (
            HostOs::Unsupported("dragonfly"),
            "dragonfly",
            "dragonfly",
            "",
        ),
    ];

    for (host, token, label, extension) in cases {
        assert_eq!(host.manifest_token(), token);
        assert_eq!(host.display_label(), label);
        assert_eq!(host.executable_extension(), extension);
    }
}

#[test]
fn unsupported_hosts_fail_release_resolution() {
    let os_error =
        SupportedOs::from_token(HostOs::Unsupported("dragonfly").manifest_token()).unwrap_err();
    assert_eq!(
        os_error.to_string(),
        "unsupported OS for release asset resolution: dragonfly"
    );

    let arch_error =
        SupportedArch::from_token(HostArch::Unsupported("riscv64").manifest_token()).unwrap_err();
    assert_eq!(
        arch_error.to_string(),
        "unsupported CPU architecture for release assets: riscv64"
    );
}

#[test]
fn dependency_binary_paths_preserve_host_conventions() {
    let plugin_dir = Path::new("plugin-root");
    let cases = [
        (HostOs::Linux, "runner", "runner"),
        (HostOs::Macos, "runner", "runner"),
        (HostOs::Windows, "runner", "runner.exe"),
        (HostOs::Windows, "runner.cmd", "runner.cmd"),
        (HostOs::Unsupported("dragonfly"), "runner", "runner"),
    ];

    for (host_os, binary_name, expected_name) in cases {
        assert_eq!(
            dependency_binary_output_path_for(host_os, plugin_dir, binary_name),
            plugin_dir.join(expected_name)
        );
    }
}

#[test]
fn windows_source_build_checks_plain_then_executable_name() {
    let plugin_dir = Path::new("plugin-root");
    let release_dir = plugin_dir.join("target").join("release");
    let candidates = built_binary_candidates_for(HostOs::Windows, plugin_dir, "runner");
    assert_eq!(
        candidates,
        vec![release_dir.join("runner"), release_dir.join("runner.exe")]
    );
}
