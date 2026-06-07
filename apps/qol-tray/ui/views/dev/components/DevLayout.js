import { html } from '../../../lib/html.js';
import { PageShell } from '../../../components/PageShell.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreSection } from './CoreSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';
import { ToolingGhAccountSection } from './ToolingGhAccountSection.js';

export function DevLayout({ ctrl, containerRef }) {
    return html`
        <${PageShell} frame=${false} className="dev-view-shell">
            <div class="dev-columns" ref=${containerRef}>
                <div class="dev-col-primary">
                    <${PluginsSection} ctrl=${ctrl} />
                </div>
                <div class="dev-col-secondary">
                    <${CoreSection} />
                    <${CoreLogSection} ctrl=${ctrl} />
                    <${ActionsSection} ctrl=${ctrl} />
                    <${ToolingGhAccountSection} />
                </div>
            </div>
        <//>
    `;
}
