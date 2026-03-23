pub fn activate_app(_cx: &mut gpui::App) {
    set_activation_policy();
}

pub fn set_activation_policy() {
    qol_plugin_api::activation::set_accessory_policy();
}
