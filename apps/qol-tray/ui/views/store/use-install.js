import { useCallback } from 'preact/hooks';
import { installStorePlugin, updateStorePlugin } from './data.js';
import { toast } from '../../lib/toast.js';

export function useStoreInstall(pluginsRef, loadPlugins, installing, markUpdated) {
    const runJob = useCallback(async (id, run, verb, onSuccess) => {
        if (installing.has(id)) return false;
        const label = pluginsRef.current.find(p => p.id === id)?.name || id;
        installing.add(id, label);
        try {
            await run(id);
            if (onSuccess) onSuccess(id);
            toast('success', `${verb}d ${label}`);
        } catch (error) {
            toast('error', `Failed to ${verb.toLowerCase()} ${label}: ${error.message}`);
        } finally {
            installing.remove(id);
        }
        return true;
    }, [pluginsRef, installing]);
    const installPlugin = useCallback(async id => {
        if (await runJob(id, installStorePlugin, 'Install')) loadPlugins();
    }, [runJob, loadPlugins]);
    const updatePlugin = useCallback(
        id => runJob(id, updateStorePlugin, 'Update', markUpdated),
        [runJob, markUpdated]
    );
    return { installPlugin, updatePlugin };
}
