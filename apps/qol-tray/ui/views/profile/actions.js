import { apiJson, apiResponse, jsonRequest, readResponseText } from '../../api/client.js';
import { toast } from '../../lib/toast.js';
import { importSummary } from './summary.js';

export async function exportProfile() {
    const bundle = await apiJson('/api/config/export');
    downloadBundle(bundle);
    toast('success', 'Profile exported');
    return bundle;
}

export async function fetchSyncStatus() {
    return apiJson('/api/sync/status');
}

export async function fetchSyncProviders() {
    return apiJson('/api/sync/providers');
}

export async function bootstrapGitHubProfileSync() {
    const result = await apiJson('/api/sync/github/bootstrap', { method: 'POST' });
    toast(syncToastKind(result.status?.health), result.message);
    return result;
}

export async function connectProfileSync(payload) {
    const result = await apiJson('/api/sync/connect', jsonRequest('POST', payload));
    toast(syncToastKind(result.status?.health), result.message);
    return result;
}

export async function pullProfileSync() {
    const result = await apiJson('/api/sync/pull', { method: 'POST' });
    toast(syncToastKind(result.status?.health), result.message);
    return result;
}

export async function pushProfileSync() {
    const result = await apiJson('/api/sync/push', { method: 'POST' });
    toast(syncToastKind(result.status?.health), result.message);
    return result;
}

export async function disconnectProfileSync() {
    const result = await apiJson('/api/sync/disconnect', { method: 'POST' });
    toast('info', result.message);
    return result;
}

export async function acknowledgeProfileSync() {
    const result = await apiJson('/api/sync/acknowledge', { method: 'POST' });
    toast('success', result.message);
    return result;
}

export async function openProfileBackupsDir() {
    const response = await apiResponse('/api/sync/backups/open-dir', { method: 'POST' });
    if (response.ok) {
        toast('success', 'Opened backups folder');
        return;
    }
    const message = (await readResponseText(response)) || 'Failed to open backups folder';
    throw new Error(message);
}

export async function openProfileBackupFile(fileName) {
    if (!fileName) throw new Error('Backup file is required');
    const response = await apiResponse(
        `/api/sync/backups/${encodeURIComponent(fileName)}/open`,
        { method: 'POST' },
    );
    if (response.ok) return;
    const message = (await readResponseText(response)) || 'Failed to open backup';
    throw new Error(message);
}

export async function fetchProfileBackups() {
    return apiJson('/api/sync/backups');
}

export async function fetchProfileBackupPreview(fileName) {
    if (!fileName) {
        throw new Error('Backup file is required');
    }
    return apiJson(`/api/sync/backups/${encodeURIComponent(fileName)}`);
}

export async function importProfileFile(file) {
    if (!file) throw new Error('No file selected');
    const text = await file.text();
    return importProfileText(text);
}

export async function importProfileText(text) {
    if (!text) throw new Error('Backup is empty');
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

function defaultImportError(error) {
    toast('error', `Failed to import profile: ${error.message}`);
}

function syncToastKind(health) {
    if (health === 'error') return 'error';
    if (health === 'attention') return 'info';
    return 'success';
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
