use tray_icon::Icon;

pub fn create_icon() -> Icon {
    Icon::from_rgba(include_bytes!("../../../assets/icon.rgba").to_vec(), 64, 64).unwrap()
}
