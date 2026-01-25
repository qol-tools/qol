use proptest::prelude::*;

pub const CASES: u32 = 200;

pub fn config() -> ProptestConfig {
    ProptestConfig::with_cases(CASES)
}
