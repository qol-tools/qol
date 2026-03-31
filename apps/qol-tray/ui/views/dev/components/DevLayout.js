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

    const handleKey = useCallback((event) => {
        const vt = vtRef.current;
        if (vt?.handleKey(event)) return;
        if (vt?.activeTab === 'dev') ctrl.handleKey(event);
    }, [ctrl.handleKey]);

    useRegisterViewKeyboard('dev', handleKey);

    return html`
        <${ViewTabs} title="Developer Control" scramble=${true}
            tabs=${TABS} vtRef=${vtRef} className="dev-view-shell" containerRef=${containerRef}>
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
