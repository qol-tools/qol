use tray_icon::Icon;

const ICON_SIZE: u32 = 64;
const ICON_DATA: &[u8] = include_bytes!("../../assets/icon.rgba");
const DOT_RADIUS: i32 = 8;
const DOT_COLOR: [u8; 4] = [230, 150, 0, 255];

const _: () = assert!(
    ICON_DATA.len() == (ICON_SIZE * ICON_SIZE * 4) as usize,
    "icon.rgba must be 64x64 RGBA"
);

pub fn create_icon() -> Icon {
    Icon::from_rgba(ICON_DATA.to_vec(), ICON_SIZE, ICON_SIZE).expect("embedded icon.rgba is valid")
}

pub fn create_icon_with_dot() -> Icon {
    let mut data = ICON_DATA.to_vec();
    add_notification_dot(&mut data, ICON_SIZE);
    Icon::from_rgba(data, ICON_SIZE, ICON_SIZE).expect("embedded icon.rgba is valid")
}

fn add_notification_dot(data: &mut [u8], size: u32) {
    let center_x = (size as i32) - DOT_RADIUS - 2;
    let center_y = DOT_RADIUS + 2;
    let radius_sq = DOT_RADIUS * DOT_RADIUS;

    let pixels = (0..size as i32).flat_map(|y| (0..size as i32).map(move |x| (x, y)));
    pixels
        .filter(|&(x, y)| is_within_dot(x, y, center_x, center_y, radius_sq))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_within_dot_includes_centre_and_radius_boundary() {
        let cx = 10;
        let cy = 10;
        let radius_sq = 4 * 4;
        assert!(is_within_dot(cx, cy, cx, cy, radius_sq), "centre is in");
        assert!(
            is_within_dot(cx + 4, cy, cx, cy, radius_sq),
            "boundary is in"
        );
        assert!(!is_within_dot(cx + 5, cy, cx, cy, radius_sq), "outside out");
        assert!(
            is_within_dot(cx + 2, cy + 2, cx, cy, radius_sq),
            "diagonal 2,2 in"
        );
        assert!(
            !is_within_dot(cx + 3, cy + 3, cx, cy, radius_sq),
            "diagonal 3,3 out (18 > 16)"
        );
    }

    #[test]
    fn set_pixel_writes_full_rgba_quad_at_correct_offset() {
        let size = 4u32;
        let mut data = vec![0u8; (size * size * 4) as usize];
        set_pixel(&mut data, 1, 2, size, [10, 20, 30, 40]);
        let idx = ((2 * size + 1) * 4) as usize;
        assert_eq!(&data[idx..idx + 4], &[10, 20, 30, 40]);
        for (i, &byte) in data.iter().enumerate() {
            if (idx..idx + 4).contains(&i) {
                continue;
            }
            assert_eq!(byte, 0, "byte {i} should be untouched");
        }
    }

    #[test]
    fn add_notification_dot_paints_only_inside_disk_in_top_right_quadrant() {
        let size = 32u32;
        let mut data = vec![0u8; (size * size * 4) as usize];
        add_notification_dot(&mut data, size);

        let centre_x = size as i32 - DOT_RADIUS - 2;
        let centre_y = DOT_RADIUS + 2;
        let mut painted = 0usize;
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                let idx = ((y as u32 * size + x as u32) * 4) as usize;
                let rgba = &data[idx..idx + 4];
                if rgba == DOT_COLOR {
                    painted += 1;
                    let dx = x - centre_x;
                    let dy = y - centre_y;
                    assert!(
                        dx * dx + dy * dy <= DOT_RADIUS * DOT_RADIUS,
                        "painted pixel ({x},{y}) is outside the dot circle",
                    );
                }
            }
        }
        let area_lower = (DOT_RADIUS * DOT_RADIUS * 2) as usize;
        let area_upper = ((DOT_RADIUS + 1) * (DOT_RADIUS + 1) * 4) as usize;
        assert!(
            (area_lower..=area_upper).contains(&painted),
            "painted={painted} outside plausible disk-area window [{area_lower},{area_upper}]",
        );
    }

    #[test]
    fn create_icon_decodes_embedded_64x64_rgba() {
        let _icon = create_icon();
        let _icon_with_dot = create_icon_with_dot();
    }
}
