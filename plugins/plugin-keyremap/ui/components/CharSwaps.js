import { html } from '../lib/html.js';
import { useState, useCallback } from 'preact/hooks';

export function CharSwaps({ swaps, onChange }) {
    const [charA, setCharA] = useState('');
    const [charB, setCharB] = useState('');

    const addSwap = useCallback(() => {
        const a = charA.trim();
        const b = charB.trim();
        if (!a || !b || a === b) return;
        const dup = swaps.some(([x, y]) => (x === a && y === b) || (x === b && y === a));
        if (dup) return;
        onChange([...swaps, [a, b]]);
        setCharA(''); setCharB('');
    }, [charA, charB, swaps, onChange]);

    const removeSwap = useCallback((index) => {
        onChange(swaps.filter((_, i) => i !== index));
    }, [swaps, onChange]);

    return html`
        <section class="card">
            <h2>Character Swaps</h2>
            <p>Swap two characters everywhere — always global, bidirectional.</p>
            <div class="rules-list">
                ${swaps.length === 0 && html`<div class="empty-state">No character swaps defined.</div>`}
                ${swaps.map(([a, b], i) => html`
                    <div class="rule-row">
                        <div class="rule-side"><span class="key-label char-output">${a}</span></div>
                        <span class="arrow">\u21c4</span>
                        <div class="rule-side"><span class="key-label char-output">${b}</span></div>
                        <span class="global-badge">global</span>
                        <button class="btn-remove" onClick=${() => removeSwap(i)}>\u00d7</button>
                    </div>
                `)}
            </div>
            <div class="add-rule-row">
                <div class="rule-side">
                    <input type="text" class="key-input char-input" placeholder="char A"
                        value=${charA} onInput=${(e) => setCharA(e.target.value)}
                        onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), addSwap())} />
                </div>
                <span class="arrow">\u21c4</span>
                <div class="rule-side">
                    <input type="text" class="key-input char-input" placeholder="char B"
                        value=${charB} onInput=${(e) => setCharB(e.target.value)}
                        onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), addSwap())} />
                </div>
                <button class="btn-add" onClick=${addSwap}>+ Add</button>
            </div>
        </section>
    `;
}
