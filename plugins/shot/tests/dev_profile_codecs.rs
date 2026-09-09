use std::fs;
use std::path::Path;

fn opt_level_problem(packages: &toml::Value, name: &str) -> Option<String> {
    let Some(entry) = packages.get(name) else {
        return Some(format!(
            "{name}: missing [profile.dev.package.{name}] entry"
        ));
    };
    match entry.get("opt-level") {
        Some(value) if value.as_integer() == Some(2) => None,
        Some(value) => Some(format!("{name}: opt-level is {value}, expected 2")),
        None => Some(format!("{name}: missing opt-level, expected 2")),
    }
}

#[test]
fn dev_profile_png_codec_entries_stay_optimized() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let body =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let parsed: toml::Value =
        toml::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e));
    let packages = parsed
        .get("profile")
        .and_then(|profile| profile.get("dev"))
        .and_then(|dev| dev.get("package"))
        .unwrap_or_else(|| panic!("missing [profile.dev.package] table in {}", path.display()));

    let expected = [
        "image",
        "png",
        "fdeflate",
        "flate2",
        "miniz_oxide",
        "simd-adler32",
    ];
    let mut problems = Vec::new();
    for name in expected {
        if let Some(problem) = opt_level_problem(packages, name) {
            problems.push(problem);
        }
    }
    assert!(
        problems.is_empty(),
        "dev-profile PNG codec entries regressed in {}:\n{}",
        path.display(),
        problems.join("\n")
    );
}
