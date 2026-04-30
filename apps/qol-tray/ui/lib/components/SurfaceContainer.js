import { html } from '../html.js';

export function SurfaceContainer({ className, children, containerRef, ...rest }) {
    return html`<div ref=${containerRef} ...${rest} class=${className} data-surface-container="">${children}</div>`;
}
