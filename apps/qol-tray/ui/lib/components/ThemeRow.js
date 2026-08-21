import { html } from '../html.js';
import { Surface } from './Surface.js';
import { DEFAULT_THEME, THEMES } from '../theme-presets.js';

export function ThemeRow({ value, onPick }) {
    const effective = value ?? DEFAULT_THEME;
    return html`
        <div class="wsp-accent">
            <span class="wsp-label">Web UI theme</span>
            <div class="wsp-swatches">
                ${THEMES.map((theme) => html`
                    <${Surface} as="button" key=${theme.key}
                        className=${`wsp-swatch wsp-theme-swatch${effective === theme.key ? ' is-active' : ''}`}
                        data-qol-theme-preview=${theme.key} title=${theme.label}
                        onActivate=${() => onPick(theme.key)} />
                `)}
            </div>
        </div>`;
}
