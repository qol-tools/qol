pub fn activate_app(cx: &mut gpui::App) {
    set_activation_policy();
    cx.activate(true);
}

pub fn set_activation_policy() {
    qol_plugin_api::activation::set_accessory_policy();
}
