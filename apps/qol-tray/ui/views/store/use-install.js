import { useCallback } from 'preact/hooks';
import { installStorePlugin, updateStorePlugin } from './data.js';
import { toast } from '../../lib/toast.js';

export function useStoreInstall(pluginsRef, loadPlugins, installing) {
    const runJob = useCallback(async (id, run, verb) => {
        if (installing.has(id)) return;
        const label = pluginsRef.current.find(p => p.id === id)?.name || id;
        installing.add(id, label);
        try {
            await run(id);
            toast('success', `${verb}d ${label}`);
        } catch (error) {
            toast('error', `Failed to ${verb.toLowerCase()} ${label}: ${error.message}`);
        } finally {
            installing.remove(id);
            loadPlugins();
        }
    }, [pluginsRef, loadPlugins, installing]);
    const installPlugin = useCallback(id => runJob(id, installStorePlugin, 'Install'), [runJob]);
    const updatePlugin = useCallback(id => runJob(id, updateStorePlugin, 'Update'), [runJob]);
    return { installPlugin, updatePlugin };
}
