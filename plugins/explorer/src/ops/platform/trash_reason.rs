use trash::Error;

pub(super) fn describe(error: &Error) -> String {
    if let Error::Os { description, .. } | Error::Unknown { description } = error {
        return description.to_lowercase();
    }
    match error {
        Error::TargetedRoot => "the top of a drive cannot go to the Trash".into(),
        Error::CouldNotAccess { .. } => "it is gone or you are not allowed to touch it".into(),
        Error::CanonicalizePath { .. } => "its location could not be worked out".into(),
        Error::ConvertOsString { .. } => "its name is not valid text".into(),
        Error::RestoreCollision { .. } => {
            "something with that name is already in its old folder".into()
        }
        Error::RestoreTwins { .. } => "two items from the same place were put back at once".into(),
        _ => "the Trash refused it".into(),
    }
}
