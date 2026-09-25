#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    Trace,
    Wash,
    Halo,
    Edge,
    Veil,
    Strong,
}

impl Alpha {
    pub const ALL: [Self; 6] = [
        Self::Trace,
        Self::Wash,
        Self::Halo,
        Self::Edge,
        Self::Veil,
        Self::Strong,
    ];

    pub const fn percent(self) -> u16 {
        match self {
            Self::Trace => 5,
            Self::Wash => 9,
            Self::Halo => 16,
            Self::Edge => 24,
            Self::Veil => 40,
            Self::Strong => 70,
        }
    }

    pub const fn milli(self) -> u16 {
        self.percent() * 10
    }

    pub const fn byte(self) -> u8 {
        ((self.percent() as u32 * 255 + 50) / 100) as u8
    }

    pub fn unit(self) -> f32 {
        f32::from(self.percent()) / 100.0
    }
}

pub const fn translucent(rgb: u32, alpha: Alpha) -> u32 {
    (rgb << 8) | alpha.byte() as u32
}

pub const OPACITY_DISABLED: f32 = 0.4;
pub const OPACITY_REST: f32 = 0.6;

pub const LINE: f32 = 1.0;
pub const FOCUS_RING_EDGE: f32 = 1.5;
pub const FOCUS_RING_HALO: f32 = 4.0;
pub const STATUS_DOT: f32 = 7.0;
pub const STATUS_DOT_HALO: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowLayer {
    pub y: u8,
    pub blur: u8,
    pub alpha: Alpha,
}

pub type Shadow = [ShadowLayer; 2];

pub const SHADOW_FLOAT: Shadow = [
    ShadowLayer {
        y: 1,
        blur: 2,
        alpha: Alpha::Trace,
    },
    ShadowLayer {
        y: 8,
        blur: 20,
        alpha: Alpha::Wash,
    },
];

pub const SHADOW_RAISED: Shadow = [
    ShadowLayer {
        y: 1,
        blur: 2,
        alpha: Alpha::Wash,
    },
    ShadowLayer {
        y: 6,
        blur: 16,
        alpha: Alpha::Wash,
    },
];

pub const fn clear(rgb: u32) -> u32 {
    rgb << 8
}

pub const fn solid(rgb: u32) -> u32 {
    (rgb << 8) | 0xff
}
