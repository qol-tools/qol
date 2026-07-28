pub(crate) fn open() -> std::io::Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(crate::config::plugin_id())
}
