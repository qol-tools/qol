import { html } from '../../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { useRegisterViewKeyboard } from '../../../app/view-keyboard-context.js';
import { PageHeader } from '../../../components/PageHeader.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';
import { ToolingGhAccountSection } from './ToolingGhAccountSection.js';

export function DevLayout({ ctrl, containerRef }) {
    const handleKey = useCallback((event) => {
        ctrl.handleKey(event);
    }, [ctrl.handleKey]);

    useRegisterViewKeyboard('dev', handleKey);

    return html`
        <div class="view-container content-shell dev-view-shell" ref=${containerRef}>
            <${PageHeader} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
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
                </div>
            </div>
        </div>
    `;
}
