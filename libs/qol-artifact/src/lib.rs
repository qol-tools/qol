mod inspection;
mod target;
mod verification;

pub use inspection::{
    inspect_bytes, inspect_path, ArtifactArchitecture, ArtifactFormat, InspectedArtifact,
    InspectedSlice, InspectionError,
};
pub use verification::{verify_identity, verify_path, ArtifactExpectation, VerificationError};
