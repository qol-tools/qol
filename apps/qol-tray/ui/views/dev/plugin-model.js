function getLogControl(logControls, pluginId) {
    const control = logControls?.[pluginId];
    if (!control || typeof control !== 'object') {
        return { muted: false, suppress_patterns: [] };
    }
    return {
        muted: !!control.muted,
        suppress_patterns: Array.isArray(control.suppress_patterns) ? control.suppress_patterns : []
    };
}

function localEntry(plugin, control) {
    return {
        id: plugin.id,
        name: plugin.name,
        path: plugin.path,
        status: 'local',
        has_cargo: false,
        supports_platform: true,
        needs_rebuild: false,
        rebuild_reason: '',
        fingerprint: null,
        last_built_fingerprint: null,
        logs_muted: control.muted,
        suppressed_log_patterns: control.suppress_patterns
    };
}

function linkedEntry(plugin, logsMuted, suppressedPatterns) {
    return {
        id: plugin.id,
        name: plugin.name,
        path: plugin.source,
        status: 'linked',
        has_cargo: !!plugin.has_cargo,
        supports_platform: plugin.supports_platform !== false,
        needs_rebuild: !!plugin.needs_rebuild,
        rebuild_reason: plugin.rebuild_reason || '',
        fingerprint: plugin.fingerprint || null,
        last_built_fingerprint: plugin.last_built_fingerprint || null,
        logs_muted: logsMuted,
        suppressed_log_patterns: suppressedPatterns
    };
}

function applyLinked(existing, plugin, logsMuted, suppressedPatterns) {
    existing.status = 'linked';
    existing.path = plugin.source || existing.path;
    existing.has_cargo = !!plugin.has_cargo;
    existing.supports_platform = plugin.supports_platform !== false;
    existing.needs_rebuild = !!plugin.needs_rebuild;
    existing.rebuild_reason = plugin.rebuild_reason || '';
    existing.fingerprint = plugin.fingerprint || null;
    existing.last_built_fingerprint = plugin.last_built_fingerprint || null;
    existing.logs_muted = logsMuted;
    existing.suppressed_log_patterns = suppressedPatterns;
}

export function mergePlugins(discovered, linkedPlugins, logControls = {}) {
    const unified = new Map();
    for (const plugin of discovered) {
        unified.set(plugin.id, localEntry(plugin, getLogControl(logControls, plugin.id)));
    }
    for (const plugin of linkedPlugins) {
        const control = getLogControl(logControls, plugin.id);
        const logsMuted = !!plugin.logs_muted || control.muted;
        const suppressedPatterns = Array.isArray(plugin.suppressed_log_patterns)
            ? plugin.suppressed_log_patterns
            : control.suppress_patterns;
        const existing = unified.get(plugin.id);
        if (existing) { applyLinked(existing, plugin, logsMuted, suppressedPatterns); continue; }
        unified.set(plugin.id, linkedEntry(plugin, logsMuted, suppressedPatterns));
    }
    return Array.from(unified.values()).sort((left, right) => left.name.localeCompare(right.name));
}

