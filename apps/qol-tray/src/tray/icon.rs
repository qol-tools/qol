use tray_icon::Icon;

const ICON_SIZE: u32 = 64;
const ICON_DATA: &[u8] = include_bytes!("../../assets/icon.rgba");

pub fn create_icon() -> Icon {
    Icon::from_rgba(ICON_DATA.to_vec(), ICON_SIZE, ICON_SIZE).unwrap()
}

pub fn create_icon_with_dot() -> Icon {
    let mut data = ICON_DATA.to_vec();
    add_notification_dot(&mut data, ICON_SIZE);
    Icon::from_rgba(data, ICON_SIZE, ICON_SIZE).unwrap()
}

fn add_notification_dot(data: &mut [u8], size: u32) {
    let dot_radius = 8i32;
    let dot_center_x = (size as i32) - dot_radius - 2;
    let dot_center_y = dot_radius + 2;

    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let dx = x - dot_center_x;
            let dy = y - dot_center_y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq <= dot_radius * dot_radius {
                let idx = ((y as u32 * size + x as u32) * 4) as usize;
                data[idx] = 230;
                data[idx + 1] = 150;
                data[idx + 2] = 0;
                data[idx + 3] = 255;
            }
        }
    }
}
