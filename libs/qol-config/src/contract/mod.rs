mod v1;

pub use v1::{
    parse_spec, parse_spec_str, ConfigSpec, ConfigSpecV1, FieldDefault, FieldKind, FieldSpec,
    NumberConstraints, ParseSpecError, SectionSpec,
};
