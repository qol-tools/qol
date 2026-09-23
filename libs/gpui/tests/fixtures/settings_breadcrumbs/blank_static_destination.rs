use qol_gpui::settings_panel::SettingsDestination;

const BLANK_DESTINATION: SettingsDestination =
    SettingsDestination::from_static(" \u{3000}\u{0085}");

fn main() {
    let _ = BLANK_DESTINATION.label();
}
