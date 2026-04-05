import { html } from '../lib/html.js';
import { usePaletteContext } from '../palette/context.js';
import { ScrambleText } from './ScrambleText.js';
import { NoiseBorder } from './NoiseBorder.js';
import { NoiseReveal } from './NoiseReveal.js';

export function PageHeader({ title, subtitle = '', badge = null, scramble = false, noiseReveal = false, className = '' }) {
    const { active } = usePaletteContext();
    const cls = ['page-header', className].filter(Boolean).join(' ');
    const Title = scramble ? html`<${ScrambleText} text=${title} />` : title;
    const Sub = scramble && subtitle ? html`<${ScrambleText} text=${subtitle} delay=${40} />` : subtitle;
    return html`
        <div class=${cls}>
            ${noiseReveal && html`<${NoiseReveal} variant="bubble" />`}
            <div class="page-header-top">
                <div class="page-header-main">
                    <h1>${Title}</h1>
                    ${Sub ? html`<p>${Sub}</p>` : ''}
                </div>
                ${badge ? html`<div class="page-header-badge">${badge}</div>` : ''}
                <${NoiseBorder} active=${active} />
            </div>
        </div>
    `;
}
