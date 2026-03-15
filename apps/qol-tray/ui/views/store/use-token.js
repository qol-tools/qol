import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { saveStoreToken, deleteStoreToken } from './data.js';
import { looksLikeGithubAuthFailure } from './reducer.js';
import { toast } from '../../lib/toast.js';

export function useTokenOps(tokenInputRef, loadRef) {
    const [hasToken, setHasToken, hasTokenRef] = useStateRef(false);
    const [showTokenInput, setShowTokenInput, showTokenInputRef] = useStateRef(false);
    const [rateLimited, setRateLimited] = useStateRef(false);
    const focusToken = useCallback(() => setTimeout(() => tokenInputRef.current?.focus(), 0), []);
    const openTokenInput = useCallback(() => { setShowTokenInput(true); focusToken(); }, [focusToken]);
    const closeTokenInput = useCallback(() => setShowTokenInput(false), []);
    const saveToken = useSaveToken(tokenInputRef, loadRef, setHasToken, setShowTokenInput, setRateLimited);
    const deleteToken = useDeleteToken(setHasToken, setShowTokenInput);
    const onLoadResult = useLoadResultHandler(setShowTokenInput, setRateLimited, openTokenInput);
    return {
        hasTokenRef, setHasToken, showTokenInputRef, onLoadResult,
        view: { hasToken, showTokenInput, rateLimited, openTokenInput, closeTokenInput, saveToken, deleteToken }
    };
}

function useSaveToken(tokenInputRef, loadRef, setHasToken, setShowTokenInput, setRateLimited) {
    return useCallback(async () => {
        const input = tokenInputRef.current;
        const value = input?.value?.trim();
        if (!value) { toast('error', 'Token cannot be empty'); return; }
        try {
            await saveStoreToken(value);
            setHasToken(true); setShowTokenInput(false); setRateLimited(false);
            toast('success', 'GitHub token saved');
            loadRef.current?.({ hasToken: true });
        } catch (error) {
            toast('error', `Failed to save token: ${error.message}`);
            input?.focus(); input?.select();
        }
    }, [tokenInputRef, loadRef, setHasToken, setShowTokenInput, setRateLimited]);
}

function useDeleteToken(setHasToken, setShowTokenInput) {
    return useCallback(async () => {
        try {
            await deleteStoreToken();
            setHasToken(false); setShowTokenInput(false);
            toast('success', 'GitHub token removed');
        } catch (error) {
            toast('error', `Failed to delete token: ${error.message}`);
        }
    }, [setHasToken, setShowTokenInput]);
}

function useLoadResultHandler(setShowTokenInput, setRateLimited, openTokenInput) {
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
            toast('error', `Failed to load plugins: ${result.error.message}`);
        }
    }, [setShowTokenInput, setRateLimited, openTokenInput]);
}
