import { useCallback, useEffect } from 'preact/hooks';
import { readResponseText } from '../api/client.js';
import { postRestartTrigger } from './restart-trigger.js';

const RECOMPILE_ERRORS = {
    404: 'Connected daemon is older than this UI. Stop it and launch the current checkout.',
    409: 'Recompile already in progress'
};

export const SELF_UPDATE_EVENT = 'qol:self-update';

export function useSidebarActions({
    devEnabled,
    checkForUpdate,
    beginSelfUpdate,
    failSelfUpdate,
    beginDevRecompile,
    failDevRecompile,
    defaultBranchRef,
}) {
    const handler = useCallback(async (action) => {
        if (action === 'check-update') {
            if (devEnabled) {
                return;
            }
            checkForUpdate();
            return;
        }

        if (action === 'self-update') {
            beginSelfUpdate();
            await postRestartTrigger('/api/self-update', { method: 'POST' }, () => failSelfUpdate());
            return;
        }

        if (action !== 'dev-recompile') {
            return;
        }
        if (!beginDevRecompile()) {
            return;
        }

        const branch = defaultBranchRef?.current || null;
        const fetchOpts = branch
            ? { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ worktree_branch: branch }) }
            : { method: 'POST' };
        await postRestartTrigger('/api/dev/recompile-self', fetchOpts, async (res) => {
            const body = await readResponseText(res);
            failDevRecompile(
                RECOMPILE_ERRORS[res.status] || body || `Could not start recompile (${res.status})`
            );
        });
    }, [
        devEnabled,
        beginDevRecompile,
        beginSelfUpdate,
        checkForUpdate,
        failDevRecompile,
        failSelfUpdate
    ]);

    useEffect(() => {
        const listener = () => handler('self-update');
        document.addEventListener(SELF_UPDATE_EVENT, listener);
        return () => document.removeEventListener(SELF_UPDATE_EVENT, listener);
    }, [handler]);

    return handler;
}
