/// Hardcoded `id -> uid` for core plugins that may not be installed locally on
/// the machine running the migration.
///
/// A machine that syncs a profile from another machine can hold lock entries,
/// hotkeys, and config files for plugins it never installed. Those have no
/// local `plugin.toml` to read a uid from, so the uid is pinned here from the
/// canonical manifests in `plugins/*/plugin.toml`.
///
/// Installed-manifest entries take precedence over this table on conflict; the
/// table is only the fallback for ids with no local install.
///
/// The final pair is the historical rename alias: `plugin-screen-recorder` was
/// renamed to `qol-shot`, so a stale `plugin-screen-recorder` artifact must
/// re-key onto qol-shot's uid to collapse into the renamed plugin's state.
pub(super) const LEGACY_ID_TO_UID: &[(&str, &str)] = &[
    ("plugin-alt-tab", "a7f48ac7-3cd5-4402-a1fe-d517fbce0fd6"),
    (
        "plugin-cli-sessions",
        "98f8b9fe-2115-4890-a8cc-89b6d4483d75",
    ),
    (
        "plugin-ide-checkout",
        "b61195c4-a0a8-4be5-a507-9aeb45edb060",
    ),
    ("plugin-keyremap", "e1bc6f9b-95e0-46c5-951b-6cc5de5c6d87"),
    ("plugin-launcher", "5cc75f62-2e3b-463c-ac7b-ae269cff1ef1"),
    ("plugin-lights", "368871df-de60-4a7a-ab7c-8d33fcd22511"),
    ("plugin-os-themes", "c0b6aa8d-2b51-4b03-a478-6ce3db8883eb"),
    ("plugin-pointz", "9cb88d65-1d43-4fde-95b6-105761f0a14b"),
    ("plugin-removeapp", "37aae6d0-ae74-4487-946a-32a635a9ef03"),
    (
        "plugin-window-actions",
        "9fb4b550-714a-4723-b342-7a62c766cf56",
    ),
    ("qol-shot", "e8208e3e-58b3-4f8c-ad4b-ddbecafa3375"),
    (
        "plugin-screen-recorder",
        "e8208e3e-58b3-4f8c-ad4b-ddbecafa3375",
    ),
];

/// Historical ids that were renamed and share another plugin's uid. When a
/// renamed id and its new id both have lock entries that coalesce onto the same
/// uid, the renamed (alias) entry loses to the canonical one. Listed here so the
/// rename knowledge lives in one place alongside the table that encodes it.
pub(super) const LEGACY_RENAMED_IDS: &[&str] = &["plugin-screen-recorder"];
