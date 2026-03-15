import { useCallback } from 'preact/hooks';
import { installStorePlugin } from './data.js';
import { toast } from '../../lib/toast.js';

export function useStoreInstall(pluginsRef, loadPlugins, installing) {
    const installPlugin = useCallback(async (id) => {
        if (installing.has(id)) return;
        const plugin = pluginsRef.current.find(p => p.id === id);
        const label = plugin?.name || id;
        installing.add(id, label);
        try {
            await installStorePlugin(id);
            toast('success', `Installed ${label}`);
        } catch (error) {
            toast('error', `Failed to install ${label}: ${error.message}`);
        } finally {
            installing.remove(id);
            loadPlugins();
        }
    }, [pluginsRef, loadPlugins, installing]);
    return { installPlugin };
}
