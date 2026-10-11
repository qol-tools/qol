use std::path::PathBuf;

const ICONS: [(u32, &str); 3] = [
    (64, "assets/icons/64.png"),
    (128, "assets/icons/128.png"),
    (256, "assets/icons/256.png"),
];
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const MOVEABLE_DISCARDABLE: u16 = 0x1010;
const MOVEABLE_PURE_DISCARDABLE: u16 = 0x1030;

pub(crate) fn embed_windows_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let pngs: Vec<(u32, Vec<u8>)> = ICONS
        .iter()
        .map(|&(size, path)| {
            println!("cargo:rerun-if-changed={path}");
            let png = std::fs::read(path).unwrap_or_else(|error| panic!("{path}: {error}"));
            (size, png)
        })
        .collect();
    let images: Vec<(u32, &[u8])> = pngs
        .iter()
        .map(|(size, png)| (*size, png.as_slice()))
        .collect();
    let icon = crate::ico::ico_from_pngs(&images);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out_dir.join("qol-tray.ico"), &icon).expect("write qol-tray.ico");
    let resources = out_dir.join("qol-tray.res");
    std::fs::write(&resources, icon_resources(&icon)).expect("write qol-tray.res");
    println!("cargo:rustc-link-arg-bins={}", resources.display());
}

fn icon_resources(icon: &[u8]) -> Vec<u8> {
    let count = u16::from_le_bytes([icon[4], icon[5]]);
    let mut group = icon[..6].to_vec();
    let mut resources = resource(0, 0, 0, &[]);
    for id in 1..=count {
        let entry = &icon[6 + usize::from(id - 1) * 16..][..16];
        let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let image = &icon[offset..offset + length];
        resources.extend(resource(RT_ICON, id, MOVEABLE_DISCARDABLE, image));
        group.extend_from_slice(&entry[..12]);
        group.extend(id.to_le_bytes());
    }
    resources.extend(resource(
        RT_GROUP_ICON,
        1,
        MOVEABLE_PURE_DISCARDABLE,
        &group,
    ));
    resources
}

fn resource(kind: u16, id: u16, flags: u16, data: &[u8]) -> Vec<u8> {
    let mut resource = Vec::new();
    resource.extend((data.len() as u32).to_le_bytes());
    resource.extend(32u32.to_le_bytes());
    for ordinal in [kind, id] {
        resource.extend(u16::MAX.to_le_bytes());
        resource.extend(ordinal.to_le_bytes());
    }
    resource.extend(0u32.to_le_bytes());
    resource.extend(flags.to_le_bytes());
    resource.extend(0u16.to_le_bytes());
    resource.extend(0u64.to_le_bytes());
    resource.extend_from_slice(data);
    resource.resize(resource.len().next_multiple_of(4), 0);
    resource
}
