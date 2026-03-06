import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { saveStoreToken, deleteStoreToken } from './data.js';
import { looksLikeGithubAuthFailure } from './reducer.js';

export function useTokenOps(tokenInputRef, loadRef, setFeedback, clearFeedback) {
    const [hasToken, setHasToken, hasTokenRef] = useStateRef(false);
    const [showTokenInput, setShowTokenInput, showTokenInputRef] = useStateRef(false);
    const [rateLimited, setRateLimited] = useStateRef(false);
    const focusToken = useCallback(() => setTimeout(() => tokenInputRef.current?.focus(), 0), []);
    const openTokenInput = useCallback(() => { setShowTokenInput(true); focusToken(); }, [focusToken]);
    const closeTokenInput = useCallback(() => setShowTokenInput(false), []);
    const saveToken = useSaveToken(tokenInputRef, loadRef, setHasToken, setShowTokenInput, setRateLimited, setFeedback, clearFeedback);
    const deleteToken = useDeleteToken(setHasToken, setShowTokenInput, setFeedback, clearFeedback);
    const onLoadResult = useLoadResultHandler(setShowTokenInput, setRateLimited, openTokenInput, setFeedback);
    return {
        hasTokenRef, setHasToken, showTokenInputRef, onLoadResult,
        view: { hasToken, showTokenInput, rateLimited, openTokenInput, closeTokenInput, saveToken, deleteToken }
    };
}

function useSaveToken(tokenInputRef, loadRef, setHasToken, setShowTokenInput, setRateLimited, setFeedback, clearFeedback) {
    return useCallback(async () => {
        const input = tokenInputRef.current;
        const value = input?.value?.trim();
        if (!value) { setFeedback('error', 'Token cannot be empty'); return; }
        clearFeedback();
        try {
            await saveStoreToken(value);
            setHasToken(true); setShowTokenInput(false); setRateLimited(false);
            setFeedback('success', 'GitHub token saved');
            loadRef.current?.({ hasToken: true });
        } catch (error) {
            setFeedback('error', `Failed to save token: ${error.message}`);
            input?.focus(); input?.select();
        }
    }, [tokenInputRef, loadRef, setHasToken, setShowTokenInput, setRateLimited, setFeedback, clearFeedback]);
}

function useDeleteToken(setHasToken, setShowTokenInput, setFeedback, clearFeedback) {
    return useCallback(async () => {
        clearFeedback();
        try {
            await deleteStoreToken();
            setHasToken(false); setShowTokenInput(false);
            setFeedback('success', 'GitHub token removed');
        } catch (error) {
            setFeedback('error', `Failed to delete token: ${error.message}`);
        }
    }, [setHasToken, setShowTokenInput, setFeedback, clearFeedback]);
}

function useLoadResultHandler(setShowTokenInput, setRateLimited, openTokenInput, setFeedback) {
    return useCallback(result => {
        if (result.data) {
            setRateLimited(result.data.rateLimited);
            if (!result.data.rateLimited) setShowTokenInput(false);
        }
        if (result.error) {
            if (looksLikeGithubAuthFailure(result.error?.message)) {
                setRateLimited(true);
                openTokenInput();
            }
            setFeedback('error', `Failed to load plugins: ${result.error.message}`);
        }
    }, [setShowTokenInput, setRateLimited, openTokenInput, setFeedback]);
}
