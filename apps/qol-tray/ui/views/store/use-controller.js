import { useEffect, useCallback, useRef } from 'preact/hooks';
import { useInstalling } from '../../hooks/useInstalling.js';
import { usePaletteContext } from '../../palette/context.js';

import { loadStoreTokenState } from './data.js';
import { useStoreData } from './use-data.js';
import { useTokenOps } from './use-token.js';
import { useStoreNav } from './use-nav.js';
import { useStoreInstall } from './use-install.js';
import { handleStoreKey } from './keys.js';


export function useStoreController() {
    const { searchQuery } = usePaletteContext();
    const installing = useInstalling();
    const loadRef = useRef(null);

    const token = useTokenOps(loadRef);
    const data = useStoreData(token.hasTokenRef, token.onLoadResult);
    loadRef.current = data.loadPlugins;
    const nav = useStoreNav(data.plugins, searchQuery);
    const install = useStoreInstall(data.pluginsRef, data.loadPlugins, installing);
    useInitialLoad(token.setHasToken, data.loadPlugins);
    const handleKey = useKeyHandler(token.showTokenInputRef, token.view, nav, install, installing.has);
    const handleCardClick = useCardClick(install, nav.setSelectedIndex, nav.selectedIndexRef);
    return {
        handleKey, handleCardClick,
        ...token.view, ...data, ...nav, isInstalling: installing.has,
        installPlugin: install.installPlugin,
        updatePlugin: install.updatePlugin
    };
}

function useInitialLoad(setHasToken, loadPlugins) {
    useEffect(() => {
        (async () => {
            const tokenState = await loadStoreTokenState();
            setHasToken(tokenState);
            loadPlugins({ hasToken: tokenState });
        })();
    }, [loadPlugins]);
}

function useKeyHandler(showTokenInputRef, tokenView, nav, install, isInstalling) {
    return useCallback(e => {
        handleStoreKey(e, showTokenInputRef, {
            closeTokenInput: tokenView.closeTokenInput,
            navigateInGrid: nav.navigateInGrid,
            filteredRef: nav.filteredRef,
            selectedIndexRef: nav.selectedIndexRef,
            isInstalling,
            installPlugin: install.installPlugin,
            updatePlugin: install.updatePlugin,
        });
    }, [showTokenInputRef, tokenView, nav, install, isInstalling]);
}

function useCardClick(install, setSelectedIndex, selectedIndexRef) {
    return useCallback((event, index, pluginId) => {
        if (event.target.closest('button.install')) { install.installPlugin(pluginId); return; }
        if (event.target.closest('button.update')) { install.updatePlugin(pluginId); return; }
        if (index !== selectedIndexRef.current) setSelectedIndex(index);
    }, [install, setSelectedIndex, selectedIndexRef]);
}
