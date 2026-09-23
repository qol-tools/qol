import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { disconnectGitHubAuth, startGitHubAuth, waitForGitHubAuth } from '../../features/github-auth/actions.js';
import { looksLikeGithubAuthFailure } from './reducer.js';
import { toast } from '../../lib/toast.js';

export function useTokenOps(loadRef) {
    const [hasToken, setHasToken, hasTokenRef] = useStateRef(false);
    const [showTokenInput, setShowTokenInput, showTokenInputRef] = useStateRef(false);
    const [rateLimited, setRateLimited] = useStateRef(false);
    const openTokenInput = useCallback(async () => {
        try {
            const start = await startGitHubAuth();
            try { await navigator.clipboard.writeText(start.user_code); } catch (_) {}
            toast('info', `Code: ${start.user_code} — paste it on GitHub`);
            window.open(start.verification_uri, '_blank');
            await waitForGitHubAuth(start.session_id, start.interval);
            setHasToken(true);
            setShowTokenInput(false);
            setRateLimited(false);
            toast('success', 'GitHub connected');
            loadRef.current?.({ hasToken: true });
        } catch (error) {
            toast('error', `Failed to connect GitHub: ${error.message}`);
        }
    }, [loadRef, setHasToken, setRateLimited, setShowTokenInput]);
    const closeTokenInput = useCallback(() => setShowTokenInput(false), []);
    const deleteToken = useDeleteToken(setHasToken, setShowTokenInput);
    const onLoadResult = useLoadResultHandler(setShowTokenInput, setRateLimited);
    return {
        hasTokenRef, setHasToken, showTokenInputRef, onLoadResult,
        view: { hasToken, showTokenInput, rateLimited, openTokenInput, closeTokenInput, saveToken: openTokenInput, deleteToken }
    };
}

function useDeleteToken(setHasToken, setShowTokenInput) {
    return useCallback(async () => {
        try {
            await disconnectGitHubAuth();
            setHasToken(false); setShowTokenInput(false);
            toast('success', 'GitHub disconnected');
        } catch (error) {
            toast('error', `Failed to disconnect GitHub: ${error.message}`);
        }
    }, [setHasToken, setShowTokenInput]);
}

function useLoadResultHandler(setShowTokenInput, setRateLimited) {
    return useCallback(result => {
        if (result.data) {
            setRateLimited(result.data.rateLimited);
            if (!result.data.rateLimited) setShowTokenInput(false);
        }
        if (result.error) {
            if (looksLikeGithubAuthFailure(result.error?.message)) {
                setRateLimited(true);
            }
            toast('error', `Failed to load plugins: ${result.error.message}`);
        }
    }, [setShowTokenInput, setRateLimited]);
}
