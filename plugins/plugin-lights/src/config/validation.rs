use super::model::{BackendConfig, PluginConfig, PresetConfig, PresetSlot};

pub fn validate(config: &PluginConfig) -> Result<(), String> {
    validate_backend(&config.backend)?;
    validate_main_target(config)?;
    validate_presets(&config.presets)
}

fn validate_backend(backend: &BackendConfig) -> Result<(), String> {
    if backend.kind != "zigbee-direct" {
        return Err("backend kind must be zigbee-direct".into());
    }
    if backend.serial_port.trim().is_empty() {
        return Err("backend serial_port must not be empty".into());
    }
    if !(11..=26).contains(&backend.channel) {
        return Err("backend channel must be between 11 and 26".into());
    }
    Ok(())
}

fn validate_main_target(config: &PluginConfig) -> Result<(), String> {
    if config.main_target_type != "device" && config.main_target_type != "group" {
        return Err("main_target_type must be device or group".into());
    }
    Ok(())
}

fn validate_presets(presets: &PresetConfig) -> Result<(), String> {
    validate_preset_slot("preset_1", &presets.preset_1)?;
    validate_preset_slot("preset_2", &presets.preset_2)?;
    validate_preset_slot("preset_3", &presets.preset_3)?;
    validate_preset_slot("preset_4", &presets.preset_4)?;
    validate_preset_slot("preset_5", &presets.preset_5)?;
    validate_preset_slot("preset_6", &presets.preset_6)?;
    validate_preset_slot("preset_7", &presets.preset_7)?;
    validate_preset_slot("preset_8", &presets.preset_8)?;
    Ok(())
}

fn validate_preset_slot(field: &str, slot: &PresetSlot) -> Result<(), String> {
    if slot.name.trim().is_empty() {
        return Err(format!("{} name must not be empty", field));
    }
    if slot.brightness > 100 {
        return Err(format!("{} brightness must be between 0 and 100", field));
    }
    if !is_hex_color(&slot.color_hex) {
        return Err(format!("{} color_hex must be a 6-digit hex color", field));
    }
    if slot.mirek == 0 {
        return Err(format!("{} mirek must be greater than 0", field));
    }
    Ok(())
}

fn is_hex_color(value: &str) -> bool {
    if value.len() != 6 {
        return false;
    }

    value.chars().all(|ch| ch.is_ascii_hexdigit())
}
