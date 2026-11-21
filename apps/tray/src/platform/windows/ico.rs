pub(crate) fn ico_from_pngs(images: &[(u32, &[u8])]) -> Vec<u8> {
    const HEADER: usize = 6;
    const ENTRY: usize = 16;
    let mut icon = Vec::new();
    icon.extend(0u16.to_le_bytes());
    icon.extend(1u16.to_le_bytes());
    icon.extend((images.len() as u16).to_le_bytes());
    let mut offset = HEADER + ENTRY * images.len();
    for (size, png) in images {
        let dimension = if *size >= 256 { 0 } else { *size as u8 };
        icon.extend([dimension, dimension, 0, 0]);
        icon.extend(1u16.to_le_bytes());
        icon.extend(32u16.to_le_bytes());
        icon.extend((png.len() as u32).to_le_bytes());
        icon.extend((offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        icon.extend_from_slice(png);
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_directory_points_at_each_embedded_png() {
        let images: [(u32, &[u8]); 2] = [(64, b"first"), (256, b"second!")];
        let icon = ico_from_pngs(&images);
        let u16_at = |at: usize| u16::from_le_bytes([icon[at], icon[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(icon[at..at + 4].try_into().unwrap());
        assert_eq!((u16_at(0), u16_at(2), u16_at(4)), (0, 1, 2));
        for (index, (size, png)) in images.iter().enumerate() {
            let entry = 6 + index * 16;
            let expected_dimension = if *size >= 256 { 0 } else { *size as u8 };
            assert_eq!(icon[entry], expected_dimension);
            assert_eq!(u32_at(entry + 8) as usize, png.len());
            let offset = u32_at(entry + 12) as usize;
            assert_eq!(&icon[offset..offset + png.len()], *png);
        }
    }
}
