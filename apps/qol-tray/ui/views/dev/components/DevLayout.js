import { html } from '../../../lib/html.js';
import { useCallback, useRef } from 'preact/hooks';
import { useRegisterViewKeyboard } from '../../../components/app/view-keyboard-context.js';
import { ViewTabs } from '../../../components/ViewTabs.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';
import { ComponentsCatalog } from './ComponentsCatalog.js';

const TABS = [
    { id: 'dev', label: 'Dev' },
    { id: 'components', label: 'Components' },
];

export function DevLayout({ ctrl, containerRef }) {
    const vtRef = useRef(null);

    const onTabActivate = useCallback(() => {
        ctrl.setSelectedIndex(0);
    }, [ctrl.setSelectedIndex]);

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
            onActivate=${onTabActivate} onContentBlur=${onContentBlur}>
            ${(vt) => html`
                ${vt.activeTab === 'dev' && html`
                    <${PluginsSection} ctrl=${ctrl} />
                    <${CoreLogSection} ctrl=${ctrl} />
                    <${ActionsSection} ctrl=${ctrl} />
                `}
                ${vt.activeTab === 'components' && html`
                    <${ComponentsCatalog} />
                `}
            `}
        <//>
    `;
}
