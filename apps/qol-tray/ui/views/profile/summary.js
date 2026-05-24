export function importCounts(result) {
    const counts = {
        installed: 0,
        updated: 0,
        kept: 0,
        skipped: 0,
        failed: 0,
    };
    for (const plugin of result?.plugins || []) {
        if (plugin.status === 'install') counts.installed += 1;
        if (plugin.status === 'update') counts.updated += 1;
        if (plugin.status === 'kept') counts.kept += 1;
        if (plugin.status === 'skipped') counts.skipped += 1;
        if (plugin.status === 'failed') counts.failed += 1;
    }
    return counts;
}

export function importSummary(result) {
    const counts = importCounts(result);
    const parts = [];
    if (counts.installed) parts.push(`${counts.installed} installed`);
    if (counts.updated) parts.push(`${counts.updated} updated`);
    if (counts.kept) parts.push(`${counts.kept} unchanged`);
    if (counts.skipped) parts.push(`${counts.skipped} skipped`);
    if (counts.failed) parts.push(`${counts.failed} failed`);
    if (parts.length === 0) {
        return result?.success ? 'Profile imported' : 'Profile imported with warnings';
    }
    if (result?.success) {
        return `Profile imported: ${parts.join(', ')}`;
    }
    return `Profile imported with warnings: ${parts.join(', ')}`;
}

export function buildBadges(counts) {
    const badges = [];
    if (counts.installed) badges.push({ label: `${counts.installed} installed`, className: 'profile-badge-install' });
    if (counts.updated) badges.push({ label: `${counts.updated} updated`, className: 'profile-badge-update' });
    if (counts.kept) badges.push({ label: `${counts.kept} unchanged`, className: 'profile-badge-kept' });
    if (counts.skipped) badges.push({ label: `${counts.skipped} skipped`, className: 'profile-badge-skipped' });
    if (counts.failed) badges.push({ label: `${counts.failed} failed`, className: 'profile-badge-failed' });
    if (badges.length > 0) {
        return badges;
    }
    return [{ label: 'No plugin actions', className: 'profile-badge-kept' }];
}

export function formatTimestamp(value) {
    if (!value) {
        return '';
    }
    try {
        return new Date(value).toLocaleString();
    } catch {
        return value;
    }
}

export function profileHealthLabel(syncStatus) {
    const health = syncStatus?.health || 'not_configured';
    if (health === 'healthy') {
        return 'Synced and healthy';
    }
    if (health === 'attention') {
        return 'Review required';
    }
    if (health === 'error') {
        return 'Sync error';
    }
    return 'Cloud sync not configured';
}

export function profileRemoteSummary(syncStatus) {
    if (!syncStatus?.repo_url) {
        return 'Cloud sync is not configured yet';
    }
    return `GitHub · ${syncStatus.repo_url}`;
}

export function connectActionLabel(configured, _providerKind) {
    if (configured) {
        return 'Save and Sync';
    }
    return 'Set up sync for this profile';
}

const ACTION_BUSY_LABELS = {
    connect: { idle: '', busy: 'Connecting' },
    pull: { idle: 'Pull Now', busy: 'Pulling' },
    push: { idle: 'Push Now', busy: 'Pushing' },
    disconnect: { idle: 'Disconnect', busy: 'Disconnecting' },
    acknowledge: { idle: 'Acknowledge', busy: 'Acknowledging' },
    export: { idle: 'Export', busy: 'Exporting' },
    import: { idle: 'Import', busy: 'Importing' },
};

export function busyActionLabel(actionId, busy) {
    const labels = ACTION_BUSY_LABELS[actionId];
    if (!labels) {
        return '';
    }
    return busy ? `${labels.busy}…` : labels.idle;
}

export function profileLastSyncSummary(syncStatus) {
    if (!syncStatus?.last_sync_at) {
        return 'Last sync never';
    }
    return `Last sync ${formatTimestamp(syncStatus.last_sync_at)}`;
}

export function formatBackupPreview(content) {
    try {
        return JSON.stringify(JSON.parse(content), null, 2);
    } catch {
        return content;
    }
}

export function formatBytes(value) {
    if (!Number.isFinite(value) || value <= 0) {
        return '0 B';
    }
    if (value < 1024) {
        return `${value} B`;
    }
    const kib = value / 1024;
    if (kib < 1024) {
        return `${kib.toFixed(1)} KB`;
    }
    const mib = kib / 1024;
    return `${mib.toFixed(1)} MB`;
}
