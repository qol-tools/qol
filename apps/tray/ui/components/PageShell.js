import { html } from '../lib/html.js';
import { PageHeader } from './PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';

export function PageShell({
    subtitle = '',
    aside = null,
    badge = null,
    header = null,
    className = '',
    frameClassName = '',
    frameId = null,
    frameRef = null,
    frame = true,
    children,
}) {
    const containerCls = ['view-container', 'content-shell', className].filter(Boolean).join(' ');
    const headerNode = header != null
        ? header
        : html`<${PageHeader} subtitle=${subtitle} aside=${aside} badge=${badge} />`;
    const body = frame
        ? html`
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} id=${frameId} containerRef=${frameRef} className=${['content-frame', frameClassName].filter(Boolean).join(' ')}>
                        ${children}
                    <//>
                </div>
            </div>`
        : html`<${SurfaceContainer} id=${frameId} containerRef=${frameRef} className=${['view-body', frameClassName].filter(Boolean).join(' ')}>${children}<//>`;
    return html`
        <div class=${containerCls}>
            ${headerNode}
            ${body}
        </div>
    `;
}
