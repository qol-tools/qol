import { html } from '../../lib/html.js';
import { Surface } from '../Surface.js';

function formatTimestamp(ts) {
    if (!ts) return '?';
    return ts.replace('T', ' ');
}

export function SuppressedRow({ sigKey, entry, expanded, index, selected, onSelect, onToggle, onUnsuppress, ...rest }) {
    const cls = ['suppressed-entry', expanded && 'expanded'].filter(Boolean).join(' ');
    return html`
        <${Surface} className=${cls} role="listitem"
            index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onToggle} ...${rest}>
            <div class="suppressed-header">
                <span class="suppressed-expand-icon">${expanded ? '\u25be' : '\u25b8'}</span>
                <span class="suppressed-key">${sigKey}</span>
                <span class="suppressed-count-badge">${'\u00d7'}${entry.count}</span>
                ${onUnsuppress && html`<button class="btn btn-sm suppressed-unsuppress" tabIndex="-1"
                    onClick=${(e) => { e.stopPropagation(); onUnsuppress(sigKey); }}>Unsuppress</button>`}
            </div>
            ${expanded && html`
                ${entry.last_message && html`<div class="suppressed-msg">${entry.last_message}</div>`}
                ${(entry.source || entry.location) && html`
                    <div class="suppressed-detail">
                        ${entry.source && html`
                            <div class="suppressed-detail-row">
                                <span class="suppressed-meta-label">Source</span>
                                <span class="suppressed-detail-value">${entry.source}</span>
                            </div>
                        `}
                        ${entry.location && html`
                            <div class="suppressed-detail-row">
                                <span class="suppressed-meta-label">Location</span>
                                <span class="suppressed-detail-value mono">${entry.location}</span>
                            </div>
                        `}
                    </div>
                `}
                <div class="suppressed-meta">
                    <span class="suppressed-meta-item">
                        <span class="suppressed-meta-label">First</span>
                        <span>${formatTimestamp(entry.first_seen)}</span>
                    </span>
                    <span class="suppressed-meta-sep">${'\u00b7'}</span>
                    <span class="suppressed-meta-item">
                        <span class="suppressed-meta-label">Last</span>
                        <span>${formatTimestamp(entry.last_seen)}</span>
                    </span>
                    ${entry.version && html`
                        <span class="suppressed-meta-sep">${'\u00b7'}</span>
                        <span class="suppressed-version">${entry.version}</span>
                    `}
                </div>
            `}
        <//>
    `;
}
