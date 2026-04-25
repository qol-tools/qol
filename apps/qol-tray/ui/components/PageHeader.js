import { html } from '../lib/html.js';
import { usePaletteContext } from '../palette/context.js';
import { ScrambleText } from '../lib/components/ScrambleText.js';
import { NoiseBorder } from '../lib/components/NoiseBorder.js';
import { NoiseReveal } from '../lib/components/NoiseReveal.js';

export function PageHeader({ title = '', subtitle = '', badge = null, scramble = false, noiseReveal = false, className = '' }) {
    const { active } = usePaletteContext();
    // Titles for root views and plugin-config sections are rendered by the
    // world-region-label above the page — don't duplicate them inside the
    // page body. Detail/editor views (no region label) still pass a title.
    // When a view passes no title, subtitle, or badge, render nothing at all —
    // avoids a blank 48px strip at the top of body-only views.
    if (!title && !subtitle && !badge) return null;
    const cls = ['page-header', className].filter(Boolean).join(' ');
    const Title = title ? (scramble ? html`<${ScrambleText} text=${title} />` : title) : null;
    const Sub = scramble && subtitle ? html`<${ScrambleText} text=${subtitle} delay=${40} />` : subtitle;
    return html`
        <div class=${cls}>
            ${noiseReveal && html`<${NoiseReveal} variant="bubble" />`}
            <div class="page-header-top">
                <div class="page-header-main">
                    ${Title ? html`<h1>${Title}</h1>` : ''}
                    ${Sub ? html`<p>${Sub}</p>` : ''}
                </div>
                ${badge ? html`<div class="page-header-badge">${badge}</div>` : ''}
                <${NoiseBorder} active=${active} />
            </div>
        </div>
    `;
}
