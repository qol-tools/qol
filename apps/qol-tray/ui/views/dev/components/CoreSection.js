import { html } from '../../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { Table } from '../../../lib/components/TableRow.js';
import { DevPluginRow } from '../../../components/domain-rows/DevPluginRow.js';
import { useListSelection } from '../../../lib/hooks/useListSelection.js';
import { diveViaSelector } from '../../../lib/world-navigation-singleton.js';

const CORE_AREAS = [
    {
        id: 'gpui',
        name: 'GPUI',
        description: 'Global ghost opacity, debug color, shared GPUI runtime settings',
        diveSelector: '[data-dive-source="dev-gpui"]',
    },
];

function CoreAreaRow({ area, index, selected, onSelect }) {
    const activate = useCallback((event) => {
        const sourceEl = event?.currentTarget?.closest?.('[data-selected-surface]');
        if (sourceEl instanceof HTMLElement) sourceEl.setAttribute('data-dive-source', '');
        diveViaSelector(area.diveSelector);
    }, [area.diveSelector]);
    return html`
        <${DevPluginRow}
            name=${area.name}
            path=${area.description}
            status="linked"
            index=${index}
            selected=${selected}
            onSelect=${onSelect}
            onActivate=${activate}
            className="core-area-row"
            data-core-area=${area.id}
        />
    `;
}

export function CoreSection() {
    const sel = useListSelection();
    return html`
        <section class="dev-section">
            <div class="section-header"><h2>Core</h2></div>
            <div class="plugin-list-container">
                <${Table} className="plugin-list" onDeselect=${sel.deselect}>
                    ${CORE_AREAS.map((area, i) => html`
                        <${CoreAreaRow}
                            key=${area.id}
                            area=${area}
                            index=${i}
                            selected=${sel.selected(i)}
                            onSelect=${sel.select} />
                    `)}
                <//>
            </div>
        </section>
    `;
}
