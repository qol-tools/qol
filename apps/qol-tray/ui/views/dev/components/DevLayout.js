import { html } from '../../../lib/html.js';
import { PageHeader } from '../../../components/PageHeader.js';
import { SurfaceContainer } from '../../../lib/components/SurfaceContainer.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';
import { ToolingGhAccountSection } from './ToolingGhAccountSection.js';

export function DevLayout({ ctrl, containerRef }) {
    return html`
        <div class="view-container content-shell dev-view-shell" ref=${containerRef}>
            <${PageHeader} />
            <${SurfaceContainer} className="view-body">
                <div class="dev-columns">
                    <div class="dev-col-primary">
                        <${PluginsSection} ctrl=${ctrl} />
                    </div>
                    <div class="dev-col-secondary">
                        <${CoreLogSection} ctrl=${ctrl} />
                        <${ActionsSection} ctrl=${ctrl} />
                        <${ToolingGhAccountSection} />
                    </div>
                </div>
            <//>
        </div>
    `;
}
