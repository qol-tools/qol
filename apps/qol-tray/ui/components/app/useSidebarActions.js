import { useCallback } from 'preact/hooks';
import { readResponseText } from '../../api/client.js';

const RECOMPILE_ERRORS = {
    404: 'Connected daemon is older than this UI. Stop it and launch the current checkout.',
    409: 'Recompile already in progress'
};

export function useSidebarActions({
    checkForUpdate,
    beginSelfUpdate,
    failSelfUpdate,
    beginDevRecompile,
    failDevRecompile,
    defaultWorktreeRef
}) {
    return useCallback(async (action) => {
        if (action === 'check-update') {
            checkForUpdate();
            return;
        }

        if (action === 'self-update') {
            beginSelfUpdate();
            try {
                await fetch('/api/self-update', { method: 'POST' });
            } catch {
                failSelfUpdate();
            }
            return;
        }

        if (action !== 'dev-recompile') {
            return;
        }
        if (!beginDevRecompile()) {
            return;
        }

        try {
            const worktreePath = defaultWorktreeRef?.current || null;
            const fetchOpts = worktreePath
                ? { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ worktree_path: worktreePath }) }
                : { method: 'POST' };
            const res = await fetch('/api/dev/recompile-self', fetchOpts);
            if (!res.ok) {
                const body = await readResponseText(res);
                throw new Error(
                    RECOMPILE_ERRORS[res.status]
                        || body
                        || `Could not start recompile (${res.status})`
                );
            }
        } catch (error) {
            failDevRecompile(error?.message || 'Could not start recompile');
        }
    }, [
        beginDevRecompile,
        beginSelfUpdate,
        checkForUpdate,
        failDevRecompile,
        failSelfUpdate
    ]);
}
