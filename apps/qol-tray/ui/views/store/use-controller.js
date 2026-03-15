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
    const tokenInputRef = useRef(null);
    const loadRef = useRef(null);

    const token = useTokenOps(tokenInputRef, loadRef);
    const data = useStoreData(token.hasTokenRef, token.onLoadResult);
    loadRef.current = data.loadPlugins;
    const nav = useStoreNav(data.plugins, searchQuery);
    const install = useStoreInstall(data.pluginsRef, data.loadPlugins, installing);
    useInitialLoad(token.setHasToken, data.loadPlugins);
    const handleKey = useKeyHandler(token.showTokenInputRef, token.view, data, nav, install, installing.has);
    const handleCardClick = useCardClick(install.installPlugin, nav.setSelectedIndex, nav.selectedIndexRef);
    return {
        tokenInputRef, handleKey, handleCardClick,
        ...token.view, ...data, ...nav, isInstalling: installing.has,
        installPlugin: install.installPlugin
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

function useKeyHandler(showTokenInputRef, tokenView, data, nav, install, isInstalling) {
    return useCallback(e => {
        handleStoreKey(e, showTokenInputRef, {
            refreshPlugins: data.refreshPlugins,
            closeTokenInput: tokenView.closeTokenInput,
            navigateInGrid: nav.navigateInGrid,
            filteredRef: nav.filteredRef,
            selectedIndexRef: nav.selectedIndexRef,
            isInstalling,
            installPlugin: install.installPlugin,
        });
    }, [showTokenInputRef, tokenView, data, nav, install, isInstalling]);
}

function useCardClick(installPlugin, setSelectedIndex, selectedIndexRef) {
    return useCallback((event, index, pluginId) => {
        if (event.target.closest('button.install')) { installPlugin(pluginId); return; }
        if (index !== selectedIndexRef.current) setSelectedIndex(index);
    }, [installPlugin, setSelectedIndex, selectedIndexRef]);
}
