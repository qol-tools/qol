mod inspection;

pub use inspection::{
    inspect_bytes, inspect_path, ArtifactArchitecture, ArtifactFormat, InspectedArtifact,
    InspectedSlice, InspectionError,
};
