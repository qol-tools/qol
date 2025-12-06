use tray_icon::Icon;

const ICON_SIZE: u32 = 64;
const ICON_DATA: &[u8] = include_bytes!("../../assets/icon.rgba");
const DOT_RADIUS: i32 = 8;
const DOT_COLOR: [u8; 4] = [230, 150, 0, 255];

pub fn create_icon() -> Icon {
    Icon::from_rgba(ICON_DATA.to_vec(), ICON_SIZE, ICON_SIZE)
        .expect("embedded icon.rgba is valid")
}

pub fn create_icon_with_dot() -> Icon {
    let mut data = ICON_DATA.to_vec();
    add_notification_dot(&mut data, ICON_SIZE);
    Icon::from_rgba(data, ICON_SIZE, ICON_SIZE)
        .expect("embedded icon.rgba is valid")
}

fn add_notification_dot(data: &mut [u8], size: u32) {
    let center_x = (size as i32) - DOT_RADIUS - 2;
    let center_y = DOT_RADIUS + 2;
    let radius_sq = DOT_RADIUS * DOT_RADIUS;

    let pixels = (0..size as i32).flat_map(|y| (0..size as i32).map(move |x| (x, y)));
    pixels.filter(|&(x, y)| is_within_dot(x, y, center_x, center_y, radius_sq))
        .for_each(|(x, y)| set_pixel(data, x, y, size, DOT_COLOR));
}

fn is_within_dot(x: i32, y: i32, cx: i32, cy: i32, radius_sq: i32) -> bool {
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius_sq
}

fn set_pixel(data: &mut [u8], x: i32, y: i32, size: u32, color: [u8; 4]) {
    let idx = ((y as u32 * size + x as u32) * 4) as usize;
    data[idx..idx + 4].copy_from_slice(&color);
}
