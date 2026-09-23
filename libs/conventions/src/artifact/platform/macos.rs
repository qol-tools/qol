pub(super) fn link_section() -> String {
    format!("__DATA,{}", super::super::MACHO_SECTION_NAME)
}
