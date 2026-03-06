export function renderStatusBadges(plugin, statusToken) {
    return `
        <div class="plugin-status-badges">
            ${statusBadge(statusToken)}
            ${buildBadge(plugin, statusToken)}
            ${plugin.hasStoreInstall ? '<span class="badge badge-installed-dim">+Store</span>' : ''}
        </div>
    `;
}

function statusBadge(statusToken) {
    return {
        linked: '<span class="badge badge-linked">Linked</span>',
        installed: '<span class="badge badge-installed">Installed</span>',
        local: '<span class="badge badge-local">Local Clone</span>'
    }[statusToken] || '';
}

function buildBadge(plugin, statusToken) {
    if (statusToken === 'linked' && !plugin.supports_platform) {
        return '<span class="badge badge-build-skip">Unsupported</span>';
    }
    if (statusToken === 'linked' && plugin.supports_platform && !plugin.has_cargo) {
        return '<span class="badge badge-build-skip">No Cargo</span>';
    }
    return '';
}
