function shortFingerprint(value) {
    if (!value) return '';
    return value.slice(0, 8);
}

export function mergePlugins(discovered, linkedPlugins, logControls = {}) {
    const unified = new Map();

    const controlFor = pluginId => {
        const control = logControls?.[pluginId];
        if (!control || typeof control !== 'object') {
            return { muted: false, suppress_patterns: [] };
        }
        return {
            muted: !!control.muted,
            suppress_patterns: Array.isArray(control.suppress_patterns) ? control.suppress_patterns : []
        };
    };

    for (const discoveredPlugin of discovered) {
        const control = controlFor(discoveredPlugin.id);
        unified.set(discoveredPlugin.id, {
            id: discoveredPlugin.id,
            name: discoveredPlugin.name,
            path: discoveredPlugin.path,
            status: 'local',
            has_cargo: false,
            supports_platform: true,
            needs_rebuild: false,
            rebuild_reason: '',
            fingerprint: null,
            last_built_fingerprint: null,
            logs_muted: control.muted,
            suppressed_log_patterns: control.suppress_patterns
        });
    }

    for (const linkedPlugin of linkedPlugins) {
        const existing = unified.get(linkedPlugin.id);
        const control = controlFor(linkedPlugin.id);
        if (existing) {
            existing.status = 'linked';
            existing.path = linkedPlugin.source || existing.path;
            existing.has_cargo = !!linkedPlugin.has_cargo;
            existing.supports_platform = linkedPlugin.supports_platform !== false;
            existing.needs_rebuild = !!linkedPlugin.needs_rebuild;
            existing.rebuild_reason = linkedPlugin.rebuild_reason || '';
            existing.fingerprint = linkedPlugin.fingerprint || null;
            existing.last_built_fingerprint = linkedPlugin.last_built_fingerprint || null;
            existing.logs_muted = !!linkedPlugin.logs_muted || control.muted;
            existing.suppressed_log_patterns = Array.isArray(linkedPlugin.suppressed_log_patterns)
                ? linkedPlugin.suppressed_log_patterns
                : control.suppress_patterns;
        } else {
            unified.set(linkedPlugin.id, {
                id: linkedPlugin.id,
                name: linkedPlugin.name,
                path: linkedPlugin.source,
                status: 'linked',
                has_cargo: !!linkedPlugin.has_cargo,
                supports_platform: linkedPlugin.supports_platform !== false,
                needs_rebuild: !!linkedPlugin.needs_rebuild,
                rebuild_reason: linkedPlugin.rebuild_reason || '',
                fingerprint: linkedPlugin.fingerprint || null,
                last_built_fingerprint: linkedPlugin.last_built_fingerprint || null,
                logs_muted: !!linkedPlugin.logs_muted || control.muted,
                suppressed_log_patterns: Array.isArray(linkedPlugin.suppressed_log_patterns)
                    ? linkedPlugin.suppressed_log_patterns
                    : control.suppress_patterns
            });
        }
    }

    return Array.from(unified.values()).sort((left, right) => left.name.localeCompare(right.name));
}

export function renderPluginBuildMeta(plugin) {
    if (plugin.status !== 'linked') {
        return '<span class="plugin-build-meta plugin-build-meta-placeholder" aria-hidden="true">_</span>';
    }

    if (!plugin.supports_platform) {
        return `<span class="plugin-build-meta muted">${plugin.rebuild_reason || 'Unsupported platform'}</span>`;
    }

    if (!plugin.has_cargo) {
        return '<span class="plugin-build-meta muted">Not buildable: Cargo.toml missing</span>';
    }

    const current = shortFingerprint(plugin.fingerprint);
    const last = shortFingerprint(plugin.last_built_fingerprint);
    const reason = plugin.rebuild_reason || (plugin.needs_rebuild ? 'Source changed' : 'Up to date');
    const parts = [];
    if (plugin.needs_rebuild && reason) parts.push(reason);
    if (current) parts.push(`fp ${current}`);
    if (last) parts.push(`last ${last}`);
    return `<span class="plugin-build-meta">${parts.join(' • ')}</span>`;
}

export function renderBuildResults(buildResults) {
    if (!buildResults) return '';

    const failed = buildResults.filter(result => !result.success);
    const skipped = buildResults.filter(result => result.skipped);
    if (buildResults.length === 0 || skipped.length === buildResults.length) {
        return '<span class="build-success">All linked plugins are up to date</span>';
    }

    const allSuccess = failed.length === 0;
    if (allSuccess) {
        const skippedText = skipped.length ? ` (${skipped.length} skipped)` : '';
        return `<span class="build-success">Build succeeded${skippedText}</span>`;
    }

    return `<span class="build-error">Build failed: ${failed.map(result => result.plugin_id).join(', ')}</span>`;
}
