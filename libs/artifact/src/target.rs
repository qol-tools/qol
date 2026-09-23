use target_lexicon::{BinaryFormat, Environment, OperatingSystem, Triple, Vendor};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetPlatform {
    vendor: Vendor,
    operating_system: OperatingSystem,
    environment: Environment,
    binary_format: BinaryFormat,
}

pub(crate) fn same_platform(left: &str, right: &str) -> Result<bool, String> {
    Ok(parse(left)? == parse(right)?)
}

fn parse(target: &str) -> Result<TargetPlatform, String> {
    let triple = target
        .parse::<Triple>()
        .map_err(|error| format!("{target:?}: {error}"))?;
    Ok(TargetPlatform {
        vendor: triple.vendor,
        operating_system: triple.operating_system,
        environment: triple.environment,
        binary_format: triple.binary_format,
    })
}
