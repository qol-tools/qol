#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub x: f32,
    pub y: f32,
}

const NAMED: [(&str, Spot); 9] = [
    ("top-left", Spot { x: 0.0, y: 0.0 }),
    ("top-center", Spot { x: 0.5, y: 0.0 }),
    ("top-right", Spot { x: 1.0, y: 0.0 }),
    ("center-left", Spot { x: 0.0, y: 0.5 }),
    ("center", Spot { x: 0.5, y: 0.5 }),
    ("center-right", Spot { x: 1.0, y: 0.5 }),
    ("bottom-left", Spot { x: 0.0, y: 1.0 }),
    ("bottom-center", Spot { x: 0.5, y: 1.0 }),
    ("bottom-right", Spot { x: 1.0, y: 1.0 }),
];

const PRECISION: f32 = 1000.0;

impl Spot {
    pub const TOP_RIGHT: Spot = Spot { x: 1.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: fraction(x),
            y: fraction(y),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if let Some((_, spot)) = NAMED.iter().find(|(name, _)| *name == value) {
            return Some(*spot);
        }
        let (x, y) = value.split_once(',')?;
        let x = x.trim().parse::<f32>().ok()?;
        let y = y.trim().parse::<f32>().ok()?;
        let inside = |value: f32| (0.0..=1.0).contains(&value);
        (inside(x) && inside(y)).then(|| Self::new(x, y))
    }

    pub fn name(self) -> Option<&'static str> {
        NAMED
            .iter()
            .find(|(_, spot)| *spot == self)
            .map(|(name, _)| *name)
    }

    pub fn token(self) -> String {
        match self.name() {
            Some(name) => name.to_string(),
            None => format!("{},{}", self.x, self.y),
        }
    }

    pub fn snaps() -> impl Iterator<Item = Spot> {
        NAMED.iter().map(|(_, spot)| *spot)
    }
}

fn fraction(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    (value.clamp(0.0, 1.0) * PRECISION).round() / PRECISION
}

#[cfg(test)]
mod tests {
    use super::Spot;

    #[test]
    fn the_nine_snapping_spots_read_and_write_as_names() {
        for spot in Spot::snaps() {
            let token = spot.token();
            assert!(spot.name().is_some(), "{token}");
            assert_eq!(Spot::parse(&token), Some(spot), "{token}");
        }
        assert_eq!(Spot::parse("top-right"), Some(Spot::TOP_RIGHT));
        assert_eq!(
            Spot::parse("1,0").map(Spot::token),
            Some("top-right".into())
        );
    }

    #[test]
    fn a_free_spot_reads_and_writes_as_rounded_fractions() {
        let spot = Spot::new(0.372_44, 0.815_51);
        assert_eq!(spot.token(), "0.372,0.816");
        assert_eq!(Spot::parse(&spot.token()), Some(spot));
        assert_eq!(Spot::parse(" 0.25 , 0.75 "), Some(Spot::new(0.25, 0.75)));
    }

    #[test]
    fn anything_else_is_not_a_spot() {
        for value in [
            "", "middle", "1", "1,", ",1", "1.2,0", "0,-0.1", "a,b", "NaN,0",
        ] {
            assert_eq!(Spot::parse(value), None, "{value:?}");
        }
    }

    #[test]
    fn a_spot_stays_inside_the_room() {
        assert_eq!(Spot::new(-3.0, 9.0), Spot { x: 0.0, y: 1.0 });
        assert_eq!(Spot::new(f32::NAN, 0.5), Spot { x: 0.0, y: 0.5 });
    }
}
