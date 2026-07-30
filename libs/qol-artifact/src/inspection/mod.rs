use object::read::macho::{FatArch, MachOFatFile32, MachOFatFile64};
use object::read::FileKind;
use object::{BinaryFormat, Object, ObjectSection};
use qol_conventions::artifact::{
    decode_frame, BuildIdentity, DecodeError, ELF_SECTION_NAME, MACHO_SECTION_NAME, PE_SECTION_NAME,
};
use std::fmt;
use std::path::Path;
use target_lexicon::{Architecture, BinaryFormat as TargetBinaryFormat, Triple};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactFormat {
    Elf,
    MachO,
    MachOUniversal,
    Pe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactArchitecture {
    Aarch64,
    Arm,
    I386,
    X86_64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedSlice {
    pub architecture: ArtifactArchitecture,
    pub identity: BuildIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedArtifact {
    pub format: ArtifactFormat,
    pub slices: Vec<InspectedSlice>,
}

#[derive(Debug)]
pub enum InspectionError {
    Io(std::io::Error),
    Object(object::Error),
    UnsupportedFormat(BinaryFormat),
    UnsupportedArchitecture(String),
    InvalidTargetTriple {
        target: String,
        reason: String,
    },
    UnsupportedTargetArchitecture(String),
    UnsupportedTargetFormat(String),
    MissingIdentity,
    MultipleIdentitySections(usize),
    InvalidIdentity(DecodeError),
    EmptyUniversalArtifact,
    TargetArchitectureMismatch {
        architecture: ArtifactArchitecture,
        target: String,
    },
    TargetFormatMismatch {
        format: ArtifactFormat,
        target: String,
    },
    UniversalArchitectureMismatch {
        index: usize,
        header: ArtifactArchitecture,
        slice: ArtifactArchitecture,
    },
    UniversalIdentityMismatch(usize),
    InvalidUniversalSlice {
        index: usize,
        source: Box<InspectionError>,
    },
}

impl fmt::Display for InspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "cannot read artifact: {error}"),
            Self::Object(error) => write!(formatter, "cannot parse artifact: {error}"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "artifact format {format:?} is unsupported")
            }
            Self::UnsupportedArchitecture(architecture) => {
                write!(formatter, "artifact architecture {architecture} is unsupported")
            }
            Self::InvalidTargetTriple { target, reason } => {
                write!(formatter, "artifact target {target:?} is invalid: {reason}")
            }
            Self::UnsupportedTargetArchitecture(architecture) => {
                write!(formatter, "target architecture {architecture} is unsupported")
            }
            Self::UnsupportedTargetFormat(format) => {
                write!(formatter, "target binary format {format} is unsupported")
            }
            Self::MissingIdentity => formatter.write_str("artifact identity section is missing"),
            Self::MultipleIdentitySections(count) => {
                write!(formatter, "artifact contains {count} identity sections")
            }
            Self::InvalidIdentity(error) => {
                write!(formatter, "artifact identity is invalid: {error}")
            }
            Self::EmptyUniversalArtifact => {
                formatter.write_str("universal artifact contains no slices")
            }
            Self::TargetArchitectureMismatch {
                architecture,
                target,
            } => write!(
                formatter,
                "artifact architecture {architecture:?} does not match target {target:?}"
            ),
            Self::TargetFormatMismatch { format, target } => write!(
                formatter,
                "artifact format {format:?} does not match target {target:?}"
            ),
            Self::UniversalArchitectureMismatch {
                index,
                header,
                slice,
            } => write!(
                formatter,
                "universal artifact slice {index} header architecture {header:?} does not match inner architecture {slice:?}"
            ),
            Self::UniversalIdentityMismatch(index) => write!(
                formatter,
                "universal artifact slice {index} has different logical identity"
            ),
            Self::InvalidUniversalSlice { index, source } => write!(
                formatter,
                "universal artifact slice {index} is invalid: {source}"
            ),
        }
    }
}

impl std::error::Error for InspectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Object(error) => Some(error),
            Self::InvalidIdentity(error) => Some(error),
            Self::InvalidUniversalSlice { source, .. } => Some(source),
            Self::UnsupportedFormat(_)
            | Self::UnsupportedArchitecture(_)
            | Self::InvalidTargetTriple { .. }
            | Self::UnsupportedTargetArchitecture(_)
            | Self::UnsupportedTargetFormat(_)
            | Self::MissingIdentity
            | Self::MultipleIdentitySections(_)
            | Self::EmptyUniversalArtifact
            | Self::TargetArchitectureMismatch { .. }
            | Self::TargetFormatMismatch { .. }
            | Self::UniversalArchitectureMismatch { .. }
            | Self::UniversalIdentityMismatch(_) => None,
        }
    }
}

impl From<std::io::Error> for InspectionError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<object::Error> for InspectionError {
    fn from(error: object::Error) -> Self {
        Self::Object(error)
    }
}

pub fn inspect_path(path: impl AsRef<Path>) -> Result<InspectedArtifact, InspectionError> {
    inspect_bytes(&std::fs::read(path)?)
}

pub fn inspect_bytes(bytes: &[u8]) -> Result<InspectedArtifact, InspectionError> {
    let kind = FileKind::parse(bytes)?;
    if kind == FileKind::MachOFat32 {
        return inspect_universal(MachOFatFile32::parse(bytes)?, bytes);
    }
    if kind == FileKind::MachOFat64 {
        return inspect_universal(MachOFatFile64::parse(bytes)?, bytes);
    }
    inspect_thin(bytes)
}

fn inspect_universal<Fat>(
    universal: object::read::macho::MachOFatFile<'_, Fat>,
    bytes: &[u8],
) -> Result<InspectedArtifact, InspectionError>
where
    Fat: FatArch,
{
    let mut slices = Vec::<InspectedSlice>::with_capacity(universal.arches().len());
    for (index, architecture) in universal.arches().iter().enumerate() {
        let header_architecture =
            artifact_architecture(architecture.architecture()).map_err(|source| {
                InspectionError::InvalidUniversalSlice {
                    index,
                    source: Box::new(source),
                }
            })?;
        let slice = architecture.data(bytes)?;
        let inspected =
            inspect_thin(slice).map_err(|source| InspectionError::InvalidUniversalSlice {
                index,
                source: Box::new(source),
            })?;
        if inspected.format != ArtifactFormat::MachO {
            return Err(InspectionError::InvalidUniversalSlice {
                index,
                source: Box::new(InspectionError::UnsupportedFormat(binary_format(slice)?)),
            });
        }
        let inspected_slice = inspected
            .slices
            .into_iter()
            .next()
            .expect("thin artifacts contain one slice");
        if header_architecture != inspected_slice.architecture {
            return Err(InspectionError::UniversalArchitectureMismatch {
                index,
                header: header_architecture,
                slice: inspected_slice.architecture,
            });
        }
        if let Some(first) = slices.first() {
            if !same_logical_identity(&first.identity, &inspected_slice.identity) {
                return Err(InspectionError::UniversalIdentityMismatch(index));
            }
        }
        slices.push(inspected_slice);
    }
    if slices.is_empty() {
        return Err(InspectionError::EmptyUniversalArtifact);
    }
    Ok(InspectedArtifact {
        format: ArtifactFormat::MachOUniversal,
        slices,
    })
}

fn inspect_thin(bytes: &[u8]) -> Result<InspectedArtifact, InspectionError> {
    let file = object::File::parse(bytes)?;
    let format = artifact_format(file.format())?;
    let architecture = artifact_architecture(file.architecture())?;
    let section_name = section_name(format);
    let mut sections = Vec::new();
    for section in file.sections() {
        if section.name()? != section_name {
            continue;
        }
        sections.push(section.data()?);
    }
    let identity = inspect_sections(&sections)?;
    ensure_target_contract(format, architecture, &identity.target)?;
    Ok(InspectedArtifact {
        format,
        slices: vec![InspectedSlice {
            architecture,
            identity,
        }],
    })
}

fn binary_format(bytes: &[u8]) -> Result<BinaryFormat, InspectionError> {
    Ok(object::File::parse(bytes)?.format())
}

fn artifact_format(format: BinaryFormat) -> Result<ArtifactFormat, InspectionError> {
    if format == BinaryFormat::Elf {
        return Ok(ArtifactFormat::Elf);
    }
    if format == BinaryFormat::MachO {
        return Ok(ArtifactFormat::MachO);
    }
    if matches!(format, BinaryFormat::Pe | BinaryFormat::Coff) {
        return Ok(ArtifactFormat::Pe);
    }
    Err(InspectionError::UnsupportedFormat(format))
}

fn section_name(format: ArtifactFormat) -> &'static str {
    match format {
        ArtifactFormat::Elf => ELF_SECTION_NAME,
        ArtifactFormat::MachO | ArtifactFormat::MachOUniversal => MACHO_SECTION_NAME,
        ArtifactFormat::Pe => PE_SECTION_NAME,
    }
}

fn artifact_architecture(
    architecture: object::Architecture,
) -> Result<ArtifactArchitecture, InspectionError> {
    if architecture == object::Architecture::Aarch64 {
        return Ok(ArtifactArchitecture::Aarch64);
    }
    if architecture == object::Architecture::Arm {
        return Ok(ArtifactArchitecture::Arm);
    }
    if architecture == object::Architecture::I386 {
        return Ok(ArtifactArchitecture::I386);
    }
    if architecture == object::Architecture::X86_64 {
        return Ok(ArtifactArchitecture::X86_64);
    }
    Err(InspectionError::UnsupportedArchitecture(format!(
        "{architecture:?}"
    )))
}

fn ensure_target_contract(
    format: ArtifactFormat,
    architecture: ArtifactArchitecture,
    target: &str,
) -> Result<(), InspectionError> {
    let triple =
        target
            .parse::<Triple>()
            .map_err(|error| InspectionError::InvalidTargetTriple {
                target: target.to_string(),
                reason: error.to_string(),
            })?;
    let target_architecture = match triple.architecture {
        Architecture::Aarch64(_) => ArtifactArchitecture::Aarch64,
        Architecture::Arm(_) => ArtifactArchitecture::Arm,
        Architecture::X86_32(_) => ArtifactArchitecture::I386,
        Architecture::X86_64 | Architecture::X86_64h => ArtifactArchitecture::X86_64,
        other => {
            return Err(InspectionError::UnsupportedTargetArchitecture(format!(
                "{other:?}"
            )));
        }
    };
    if architecture != target_architecture {
        return Err(InspectionError::TargetArchitectureMismatch {
            architecture,
            target: target.to_string(),
        });
    }
    let target_format = match triple.binary_format {
        TargetBinaryFormat::Elf => ArtifactFormat::Elf,
        TargetBinaryFormat::Macho => ArtifactFormat::MachO,
        TargetBinaryFormat::Coff => ArtifactFormat::Pe,
        other => {
            return Err(InspectionError::UnsupportedTargetFormat(format!(
                "{other:?}"
            )));
        }
    };
    if format != target_format {
        return Err(InspectionError::TargetFormatMismatch {
            format,
            target: target.to_string(),
        });
    }
    Ok(())
}

fn same_logical_identity(left: &BuildIdentity, right: &BuildIdentity) -> bool {
    left.schema == right.schema
        && left.binary == right.binary
        && left.role == right.role
        && left.package == right.package
        && left.version == right.version
        && left.intent == right.intent
        && left.flavor == right.flavor
        && left.compiler == right.compiler
        && left.features == right.features
        && left.source == right.source
        && crate::target::same_platform(&left.target, &right.target).unwrap_or(false)
}

fn inspect_sections(sections: &[&[u8]]) -> Result<BuildIdentity, InspectionError> {
    match sections {
        [] => Err(InspectionError::MissingIdentity),
        [section] => decode_frame(trim_padding(section)).map_err(InspectionError::InvalidIdentity),
        multiple => Err(InspectionError::MultipleIdentitySections(multiple.len())),
    }
}

fn trim_padding(section: &[u8]) -> &[u8] {
    let length = section
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    &section[..length]
}

#[cfg(test)]
mod tests {
    use super::{inspect_bytes, inspect_sections, ArtifactFormat, InspectionError};
    use object::write::{Object, Symbol, SymbolSection};
    use object::{
        Architecture, BinaryFormat, Endianness, SectionKind, SymbolFlags, SymbolKind, SymbolScope,
    };
    use qol_conventions::artifact::{
        BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole, CompilerFacts,
        SourceIdentity, FRAME_MAGIC, SCHEMA_VERSION,
    };

    fn frame(target: &str) -> Vec<u8> {
        let identity = BuildIdentity {
            schema: SCHEMA_VERSION,
            binary: "foo".to_string(),
            role: BuildRole::Host,
            package: "foo".to_string(),
            version: "1.0.0".to_string(),
            target: target.to_string(),
            intent: BuildIntent::Unspecified,
            flavor: BuildFlavor {
                profile: BuildProfile::Debug,
                dev_features: false,
            },
            compiler: CompilerFacts {
                cargo_profile: "debug".to_string(),
                opt_level: "0".to_string(),
                debuginfo: true,
                debug_assertions: true,
                overflow_checks: None,
                test: false,
            },
            features: Vec::new(),
            source: SourceIdentity::Unspecified,
        };
        let mut frame = FRAME_MAGIC.as_bytes().to_vec();
        frame.extend(serde_json::to_vec(&identity).unwrap());
        frame
    }

    fn object_with_identity(
        format: BinaryFormat,
        architecture: Architecture,
        section: &str,
        target: &str,
    ) -> Vec<u8> {
        let mut object = Object::new(format, architecture, Endianness::Little);
        let segment = if format == BinaryFormat::MachO {
            b"__DATA".to_vec()
        } else {
            Vec::new()
        };
        let section = object.add_section(
            segment,
            section.as_bytes().to_vec(),
            SectionKind::ReadOnlyData,
        );
        let frame = frame(target);
        let size = frame.len() as u64;
        object.set_section_data(section, frame, 1);
        object.add_symbol(Symbol {
            name: b"qol_build_identity".to_vec(),
            value: 0,
            size,
            kind: SymbolKind::Data,
            scope: SymbolScope::Compilation,
            weak: false,
            section: SymbolSection::Section(section),
            flags: SymbolFlags::None,
        });
        object.write().unwrap()
    }

    fn fat_macho(slices: &[(Architecture, Vec<u8>)]) -> Vec<u8> {
        let header_length = 8 + slices.len() * 20;
        let mut offset = header_length;
        let mut output = Vec::new();
        output.extend(0xcafebabe_u32.to_be_bytes());
        output.extend((slices.len() as u32).to_be_bytes());
        for (architecture, slice) in slices {
            let (cpu_type, cpu_subtype) = match architecture {
                Architecture::Aarch64 => (0x0100000c_u32, 0_u32),
                Architecture::X86_64 => (0x01000007_u32, 3_u32),
                other => panic!("unsupported test architecture: {other:?}"),
            };
            output.extend(cpu_type.to_be_bytes());
            output.extend(cpu_subtype.to_be_bytes());
            output.extend((offset as u32).to_be_bytes());
            output.extend((slice.len() as u32).to_be_bytes());
            output.extend(0_u32.to_be_bytes());
            offset += slice.len();
        }
        for (_, slice) in slices {
            output.extend(slice);
        }
        output
    }

    #[test]
    fn section_count_is_exact() {
        let frame = frame("x86_64-unknown-linux-gnu");
        let cases = [
            (Vec::<&[u8]>::new(), "missing"),
            (vec![frame.as_slice(), frame.as_slice()], "multiple"),
        ];
        for (sections, expected) in cases {
            let error = inspect_sections(&sections).unwrap_err();
            let actual = match error {
                InspectionError::MissingIdentity => "missing",
                InspectionError::MultipleIdentitySections(_) => "multiple",
                other => panic!("unexpected error: {other}"),
            };
            assert_eq!(actual, expected, "sections: {}", sections.len());
        }
    }

    #[test]
    fn section_padding_is_ignored() {
        let mut frame = frame("x86_64-unknown-linux-gnu");
        frame.extend([0; 64]);
        let identity = inspect_sections(&[&frame]).unwrap();
        assert_eq!(identity.binary, "foo");
    }

    #[test]
    fn supported_object_formats_use_their_canonical_section() {
        let cases = [
            (
                BinaryFormat::Elf,
                qol_conventions::artifact::ELF_SECTION_NAME,
                ArtifactFormat::Elf,
                "x86_64-unknown-linux-gnu",
            ),
            (
                BinaryFormat::MachO,
                qol_conventions::artifact::MACHO_SECTION_NAME,
                ArtifactFormat::MachO,
                "x86_64-apple-darwin",
            ),
            (
                BinaryFormat::Coff,
                qol_conventions::artifact::PE_SECTION_NAME,
                ArtifactFormat::Pe,
                "x86_64-pc-windows-msvc",
            ),
        ];

        for (binary_format, section, expected_format, target) in cases {
            let bytes = object_with_identity(binary_format, Architecture::X86_64, section, target);
            let inspected = inspect_bytes(&bytes).unwrap();
            assert_eq!(
                inspected.format, expected_format,
                "format: {binary_format:?}"
            );
            assert_eq!(inspected.slices.len(), 1, "format: {binary_format:?}");
            assert_eq!(
                inspected.slices[0].identity.target, target,
                "format: {binary_format:?}"
            );
        }
    }

    #[test]
    fn object_architecture_must_match_embedded_target() {
        let bytes = object_with_identity(
            BinaryFormat::Elf,
            Architecture::X86_64,
            qol_conventions::artifact::ELF_SECTION_NAME,
            "aarch64-unknown-linux-gnu",
        );
        assert!(matches!(
            inspect_bytes(&bytes),
            Err(InspectionError::TargetArchitectureMismatch { .. })
        ));
    }

    #[test]
    fn object_format_must_match_embedded_target() {
        let bytes = object_with_identity(
            BinaryFormat::Coff,
            Architecture::X86_64,
            qol_conventions::artifact::PE_SECTION_NAME,
            "x86_64-unknown-linux-gnu",
        );
        assert!(matches!(
            inspect_bytes(&bytes),
            Err(InspectionError::TargetFormatMismatch { .. })
        ));
    }

    #[test]
    fn universal_macho_requires_valid_identity_in_every_slice() {
        let x86_64 = object_with_identity(
            BinaryFormat::MachO,
            Architecture::X86_64,
            qol_conventions::artifact::MACHO_SECTION_NAME,
            "x86_64-apple-darwin",
        );
        let aarch64 = object_with_identity(
            BinaryFormat::MachO,
            Architecture::Aarch64,
            qol_conventions::artifact::MACHO_SECTION_NAME,
            "aarch64-apple-darwin",
        );
        let bytes = fat_macho(&[
            (Architecture::X86_64, x86_64),
            (Architecture::Aarch64, aarch64),
        ]);

        let inspected = inspect_bytes(&bytes).unwrap();
        assert_eq!(inspected.format, ArtifactFormat::MachOUniversal);
        assert_eq!(inspected.slices.len(), 2);
        assert_eq!(
            inspected.slices[0].architecture,
            super::ArtifactArchitecture::X86_64
        );
        assert_eq!(
            inspected.slices[1].architecture,
            super::ArtifactArchitecture::Aarch64
        );

        let ios = object_with_identity(
            BinaryFormat::MachO,
            Architecture::Aarch64,
            qol_conventions::artifact::MACHO_SECTION_NAME,
            "aarch64-apple-ios",
        );
        let mixed_platforms = fat_macho(&[
            (
                Architecture::X86_64,
                object_with_identity(
                    BinaryFormat::MachO,
                    Architecture::X86_64,
                    qol_conventions::artifact::MACHO_SECTION_NAME,
                    "x86_64-apple-darwin",
                ),
            ),
            (Architecture::Aarch64, ios),
        ]);
        assert!(matches!(
            inspect_bytes(&mixed_platforms),
            Err(InspectionError::UniversalIdentityMismatch(1))
        ));
    }
}
