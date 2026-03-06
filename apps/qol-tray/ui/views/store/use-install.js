import { useCallback } from 'preact/hooks';
import { installStorePlugin } from './data.js';

export function useStoreInstall(pluginsRef, loadPlugins, installing, setFeedback, clearFeedback) {
    const installPlugin = useCallback(async (id) => {
        if (installing.has(id)) return;
        const plugin = pluginsRef.current.find(p => p.id === id);
        const label = plugin?.name || id;
        clearFeedback();
        installing.add(id, label);
        try {
            await installStorePlugin(id);
            setFeedback('success', `Installed ${label}`);
        } catch (error) {
            setFeedback('error', `Failed to install ${label}: ${error.message}`);
        } finally {
            installing.remove(id);
            loadPlugins();
        }
    }, [pluginsRef, loadPlugins, installing, setFeedback, clearFeedback]);
    return { installPlugin };
}
