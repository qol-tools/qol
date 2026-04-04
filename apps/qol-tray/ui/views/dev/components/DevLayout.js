import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { useRegisterViewKeyboard } from '../../../components/app/view-keyboard-context.js';
import { ViewTabs } from '../../../components/ViewTabs.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';
import { useSidebarContext } from '../../../components/app/sidebar-context.js';
import { useHashSubPath } from '../../../hooks/useHashSubPath.js';
import { ComponentsCatalog } from './ComponentsCatalog.js';

const TABS = [
    { id: 'dev', label: 'Dev' },
    { id: 'components', label: 'Components' },
];

const CATALOG_ITEMS = [
    { id: 'buttons', label: 'Buttons' },
    { id: 'status', label: 'Status' },
    { id: 'spinner', label: 'Spinner' },
    { id: 'empty-state', label: 'Empty State' },
    { id: 'dropdown', label: 'Dropdown' },
    { id: 'expander', label: 'Expander' },
    { id: 'toggle', label: 'Toggle' },
    { id: 'modal', label: 'Modal' },
    { id: 'depth-diver', label: 'Depth Diver' },
    { id: 'dev-plugin-row', label: 'Dev Plugin Row' },
    { id: 'log-row', label: 'Log Row' },
    { id: 'suppressed-row', label: 'Suppressed Row' },
    { id: 'backup-row', label: 'Backup Row' },
    { id: 'hotkey-row', label: 'Hotkey Row' },
    { id: 'shortcut-row', label: 'Shortcut Row' },
    { id: 'store-card', label: 'Store Card' },
];

export function DevLayout({ ctrl, containerRef }) {
    const vtRef = useRef(null);
    const { setItems, resetSidebar } = useSidebarContext();
    const [subPath, setSubPath] = useHashSubPath('dev');
    const [activeTab, setActiveTab] = useState(subPath[0] === 'components' ? 'components' : 'dev');
    const [catalogId, setCatalogIdRaw] = useState(subPath[1] || 'buttons');
    const tokenRef = useRef(0);

    const setCatalogId = useCallback((id) => {
        setCatalogIdRaw(id);
        setSubPath(['components', id]);
    }, [setSubPath]);

    useEffect(() => {
        if (activeTab !== 'components') {
            resetSidebar(tokenRef.current);
            setSubPath([]);
            return;
        }
        setSubPath(['components', catalogId]);
        tokenRef.current = setItems(CATALOG_ITEMS.map(item => ({
            type: 'item',
            key: item.id,
            id: item.id,
            label: item.label,
            active: item.id === catalogId,
            onClick: () => setCatalogId(item.id),
        })));
        return () => resetSidebar(tokenRef.current);
    }, [activeTab, catalogId, setItems, resetSidebar, setSubPath]);

    const onTabActivate = useCallback((tabId) => {
        setActiveTab(tabId);
        if (tabId !== 'components') setSubPath([]);
        ctrl.setSelectedIndex(0);
    }, [ctrl.setSelectedIndex, setSubPath]);

    const onContentBlur = useCallback(() => {
        ctrl.setSelectedIndex(-1);
    }, [ctrl.setSelectedIndex]);

    const handleKey = useCallback((event) => {
        if (document.activeElement?.closest('[role="tablist"]')) return;
        if (vtRef.current?.activeTab === 'dev') ctrl.handleKey(event);
    }, [ctrl.handleKey]);

    useRegisterViewKeyboard('dev', handleKey);

    return html`
        <${ViewTabs} title="Developer Control" scramble=${true}
            tabs=${TABS} vtRef=${vtRef} className="dev-view-shell" containerRef=${containerRef}
            initialTab=${activeTab} onActivate=${onTabActivate} onContentBlur=${onContentBlur}>
            ${(vt) => html`
                ${vt.activeTab === 'dev' && html`
                    <${PluginsSection} ctrl=${ctrl} />
                    <${CoreLogSection} ctrl=${ctrl} />
                    <${ActionsSection} ctrl=${ctrl} />
                `}
                ${vt.activeTab === 'components' && html`
                    <${ComponentsCatalog} activeId=${catalogId} />
                `}
            `}
        <//>
    `;
}
