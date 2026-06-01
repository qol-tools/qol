import { html } from '../lib/html.js';
import { usePaletteContext } from '../palette/context.js';
import { ScrambleText } from '../lib/components/ScrambleText.js';
import { NoiseBorder } from '../lib/components/NoiseBorder.js';
import { NoiseReveal } from '../lib/components/NoiseReveal.js';

export function PageHeader({ title = '', subtitle = '', badge = null, aside = null, scramble = false, noiseReveal = false, className = '' }) {
    const { active } = usePaletteContext();
    if (!title && !subtitle && !badge && !aside) return null;
    const cls = ['page-header', className].filter(Boolean).join(' ');
    const Title = title ? (scramble ? html`<${ScrambleText} text=${title} />` : title) : null;
    const Sub = scramble && subtitle ? html`<${ScrambleText} text=${subtitle} delay=${40} />` : subtitle;
    return html`
        <div class=${cls}>
            ${noiseReveal && html`<${NoiseReveal} variant="bubble" />`}
            <div class="page-header-top">
                <div class="page-header-main">
                    ${Title ? html`<h1>${Title}</h1>` : ''}
                    ${(Sub || aside) ? html`
                        <div class="page-header-sub">
                            ${Sub ? html`<p>${Sub}</p>` : ''}
                            ${aside ? html`<div class="page-header-aside">${aside}</div>` : ''}
                        </div>
                    ` : ''}
                </div>
                ${badge ? html`<div class="page-header-badge">${badge}</div>` : ''}
                <${NoiseBorder} active=${active} />
            </div>
        </div>
    `;
}
