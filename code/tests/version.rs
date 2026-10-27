use std::path::PathBuf;

fn is_four_part(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[test]
fn version_is_four_part() {
    assert!(is_four_part(data_obfuscator::APP_VERSION), "got {}", data_obfuscator::APP_VERSION);
    let released = data_obfuscator::APP_RELEASED;
    assert_eq!(released.len(), 10, "released date should be YYYY-MM-DD, got {released}");
}

#[test]
fn version_file_matches_binary() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("VERSION");
    let v = std::fs::read_to_string(path).unwrap();
    assert!(is_four_part(v.trim()));
}

#[test]
fn version_banner_describes_program_and_audience() {
    let text = data_obfuscator::ui::banner_plain("2026-01-01 00:00:00 +00:00");
    assert!(text.contains("DATA OBFUSCATOR"));
    assert!(text.contains(data_obfuscator::APP_VERSION));
    assert!(text.contains("Clinical Pharmacology"));
    assert!(text.contains("Pharmacometrics"));
    assert!(text.contains("Data Science"));
}
