import { apiJson } from '../../api/client.js';
import { toast } from '../../lib/toast.js';

export async function exportProfile() {
    const bundle = await apiJson('/api/config/export');
    downloadBundle(bundle);
    toast('success', 'Profile exported');
    return bundle;
}

export async function importProfileFile(file) {
    if (!file) throw new Error('No file selected');
    const text = await file.text();
    JSON.parse(text);
    const result = await apiJson('/api/config/import', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: text,
    });
    toast(result.success ? 'success' : 'info', importSummary(result));
    return result;
}

export function promptImportProfile(options = {}) {
    const onImported = options.onImported || (() => {});
    const onError = options.onError || defaultImportError;
    const onSelected = options.onSelected || (() => {});
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json,application/json';
    input.onchange = async () => {
        const file = input.files?.[0];
        if (!file) return;
        onSelected(file);
        try {
            const result = await importProfileFile(file);
            onImported(result, file);
        } catch (error) {
            onError(error, file);
        }
    };
    input.click();
}

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

function defaultImportError(error) {
    toast('error', `Failed to import profile: ${error.message}`);
}

function downloadBundle(bundle) {
    const blob = new Blob([JSON.stringify(bundle, null, 2)], {
        type: 'application/json',
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = exportFilename(bundle);
    link.click();
    URL.revokeObjectURL(url);
}

function exportFilename(bundle) {
    const date = bundle?.exported_at?.slice(0, 10);
    if (date) return `qol-tray-profile-${date}.json`;
    return 'qol-tray-profile.json';
}
