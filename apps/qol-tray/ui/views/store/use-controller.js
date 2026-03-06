import { useEffect, useCallback, useRef } from 'preact/hooks';
import { useFeedback } from '../../hooks/useFeedback.js';
import { useInstalling } from '../../hooks/useInstalling.js';
import { useFooterShortcuts } from '../../hooks/useFooterShortcuts.js';
import { loadStoreTokenState } from './data.js';
import { useStoreData } from './use-data.js';
import { useTokenOps } from './use-token.js';
import { useStoreNav } from './use-nav.js';
import { useStoreInstall } from './use-install.js';
import { handleStoreKey } from './keys.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'install' },
    { key: 's', label: 'search' },
    { key: 't', label: 'token' },
    { key: '⌘R', label: 'refresh' }
];

export function useStoreController() {
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const installing = useInstalling();
    const searchRef = useRef(null);
    const tokenInputRef = useRef(null);
    const loadRef = useRef(null);
    useFooterShortcuts(SHORTCUTS);
    const token = useTokenOps(tokenInputRef, loadRef, setFeedback, clearFeedback);
    const data = useStoreData(token.hasTokenRef, token.onLoadResult);
    loadRef.current = data.loadPlugins;
    const nav = useStoreNav(data.plugins);
    const install = useStoreInstall(data.pluginsRef, data.loadPlugins, installing, setFeedback, clearFeedback);
    useInitialLoad(token.setHasToken, data.loadPlugins);
    const handleKey = useKeyHandler(searchRef, token.showTokenInputRef, token.view, data, nav, install, installing.has);
    const handleCardClick = useCardClick(install.installPlugin, nav.setSelectedIndex, nav.selectedIndexRef);
    return {
        feedback, searchRef, tokenInputRef, handleKey, handleCardClick,
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

function useKeyHandler(searchRef, showTokenInputRef, tokenView, data, nav, install, isInstalling) {
    return useCallback(e => {
        handleStoreKey(e, searchRef, showTokenInputRef, {
            refreshPlugins: data.refreshPlugins,
            closeTokenInput: tokenView.closeTokenInput,
            openTokenInput: tokenView.openTokenInput,
            navigateInGrid: nav.navigateInGrid,
            filteredRef: nav.filteredRef,
            selectedIndexRef: nav.selectedIndexRef,
            isInstalling,
            installPlugin: install.installPlugin,
            searchRef
        });
    }, [searchRef, showTokenInputRef, tokenView, data, nav, install, isInstalling]);
}

function useCardClick(installPlugin, setSelectedIndex, selectedIndexRef) {
    return useCallback((event, index, pluginId) => {
        if (event.target.closest('button.install')) { installPlugin(pluginId); return; }
        if (index !== selectedIndexRef.current) setSelectedIndex(index);
    }, [installPlugin, setSelectedIndex, selectedIndexRef]);
}
