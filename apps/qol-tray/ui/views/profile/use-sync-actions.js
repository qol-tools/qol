import { useCallback, useState } from 'preact/hooks';
import { toast } from '../../lib/toast.js';
import { fetchGitHubAuthStatus, startGitHubAuth, waitForGitHubAuth } from '../../features/github-auth/actions.js';
import {
    acknowledgeProfileSync,
    bootstrapGitHubProfileSync,
    connectProfileSync,
    disconnectProfileSync,
    exportProfile,
    promptImportProfile,
    pullProfileSync,
    pushProfileSync,
} from './actions.js';
import { buildConnectPayload } from './form.js';

export function useSyncActions({
    activeProvider,
    applySyncStatus,
    form,
    refreshSyncStatus,
    refreshAuthHealth,
}) {
    const [busyAction, setBusyAction] = useState('');
    const [lastImport, setLastImport] = useState(null);
    const [authPrompt, setAuthPrompt] = useState(null);
    const syncBusy = isSyncBusy(busyAction);
    const importBusy = isImportBusy(busyAction);

    const handleReauthorize = useCallback(async () => {
        setBusyAction('reauth');
        try {
            const start = await startGitHubAuth();
            try { await navigator.clipboard.writeText(start.user_code); } catch (_) {}
            setAuthPrompt({
                userCode: start.user_code,
                verificationUri: start.verification_uri,
                copied: true,
            });
            await waitForGitHubAuth(start.session_id, start.interval);
            setAuthPrompt(null);
            await refreshAuthHealth?.();
            toast('success', 'GitHub reauthorized.');
        } catch (error) {
            setAuthPrompt(null);
            toast('error', `Reauthorization failed: ${error.message}`);
        }
        setBusyAction('');
    }, [refreshAuthHealth]);

    const openAuthLink = useCallback(() => {
        if (authPrompt?.verificationUri) {
            window.open(authPrompt.verificationUri, '_blank');
        }
    }, [authPrompt]);

    const handleExport = useCallback(async () => {
        setBusyAction('export');
        try {
            await exportProfile();
        } catch (error) {
            toast('error', `Failed to export profile: ${error.message}`);
        }
        setBusyAction('');
    }, []);

    const handleImport = useCallback(() => {
        promptImportProfile({
            onSelected: () => setBusyAction('import'),
            onImported: (result, file) => {
                setBusyAction('');
                setLastImport({
                    fileName: file?.name || 'qol-tray-profile.json',
                    result,
                });
                refreshSyncStatus?.();
            },
            onError: (error) => {
                setBusyAction('');
                toast('error', `Failed to import profile: ${error.message}`);
            },
        });
    }, [refreshSyncStatus]);

    const handleConnect = useCallback(async () => {
        setBusyAction('connect');
        try {
            const providerKind = activeProvider?.kind || form?.provider || '';
            if (providerKind === 'github') {
                const authStatus = await fetchGitHubAuthStatus();
                if (needsGitHubReauth(authStatus)) {
                    const start = await startGitHubAuth();
                    try { await navigator.clipboard.writeText(start.user_code); } catch (_) {}
                    setAuthPrompt({
                        userCode: start.user_code,
                        verificationUri: start.verification_uri,
                        copied: true,
                    });
                    await waitForGitHubAuth(start.session_id, start.interval);
                    setAuthPrompt(null);
                    await refreshAuthHealth?.();
                }
                const result = await bootstrapGitHubProfileSync();
                applySyncStatus(result.status);
                refreshSyncStatus?.();
                setBusyAction('');
                return;
            }
            const result = await connectProfileSync(buildConnectPayload(form, activeProvider));
            applySyncStatus(result.status);
        } catch (error) {
            setAuthPrompt(null);
            toast('error', `Failed to save cloud sync: ${error.message}`);
            refreshSyncStatus?.();
        }
        setBusyAction('');
    }, [activeProvider, applySyncStatus, form, refreshAuthHealth, refreshSyncStatus]);

    const handlePull = useCallback(async () => {
        setBusyAction('pull');
        try {
            const result = await pullProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to pull cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    const handlePush = useCallback(async () => {
        setBusyAction('push');
        try {
            const result = await pushProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to push cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    const handleDisconnect = useCallback(async () => {
        setBusyAction('disconnect');
        try {
            const result = await disconnectProfileSync();
            applySyncStatus(result.status);
            await refreshAuthHealth?.();
        } catch (error) {
            toast('error', `Failed to disconnect cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus, refreshAuthHealth]);

    const handleAcknowledge = useCallback(async () => {
        setBusyAction('acknowledge');
        try {
            const result = await acknowledgeProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to acknowledge cloud sync review: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    return {
        authPrompt,
        busyAction,
        handleAcknowledge,
        handleConnect,
        handleDisconnect,
        handleExport,
        handleImport,
        handlePull,
        handlePush,
        handleReauthorize,
        importBusy,
        lastImport,
        openAuthLink,
        syncBusy,
    };
}

function needsGitHubReauth(authStatus) {
    if (!authStatus?.connected) return true;
    if (authStatus?.source !== 'oauth') return true;
    const scopes = authStatus?.scopes || [];
    return !scopes.includes('repo');
}

function isSyncBusy(busyAction) {
    return busyAction === 'connect'
        || busyAction === 'pull'
        || busyAction === 'push'
        || busyAction === 'disconnect'
        || busyAction === 'acknowledge';
}

function isImportBusy(busyAction) {
    return busyAction === 'export' || busyAction === 'import';
}
