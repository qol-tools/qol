import { html } from '../lib/html.js';

/**
 * Marks a region as keyboard-navigable. Only surfaces inside a
 * SurfaceContainer participate in global arrow-key navigation.
 *
 * Any component that needs navigable children should compose this.
 * ViewTabs uses it for the content panel. Future containers derive from it.
 */
export function SurfaceContainer({ className, children }) {
    const classes = className || '';
    return html`<div class=${classes} data-surface-container="">${children}</div>`;
}
