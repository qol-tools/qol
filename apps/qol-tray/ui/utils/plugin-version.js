export function resolvePluginVersion(plugin) {
    if (!plugin) return null;
    if (plugin.installed) {
        return plugin.running_version ?? plugin.installed_version ?? plugin.version ?? null;
    }
    return plugin.version ?? null;
}

export function formatPluginVersionLabel(version, hasUpdate) {
    if (version == null) return '';
    if (typeof version === 'string') return version ? `v${version}` : '';
    if (typeof version !== 'object') return '';
    if (hasUpdate && version.from && version.to) return `v${version.from} -> v${version.to}`;
    const current = version.current ?? version.from;
    return current ? `v${current}` : '';
}
