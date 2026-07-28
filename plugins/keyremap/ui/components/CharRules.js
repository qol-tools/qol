import { html } from '../lib/html.js';
import { useState, useEffect, useCallback } from 'preact/hooks';
import { ModChips, ModChipsStatic } from './ModChips.js';
import { normalizeMods } from '../hooks/useConfig.js';

const CHAR_MODS = ['ctrl', 'shift', 'alt', 'ralt', 'cmd'];
const SCHEMAS_BASE = 'schemas';

export function CharRules({ rules, onChange }) {
    const [schemas, setSchemas] = useState([]);
    const [selectedSchema, setSelectedSchema] = useState('');
    const [schemaStatus, setSchemaStatus] = useState('');
    const [fromMods, setFromMods] = useState([]);
    const [fromKey, setFromKey] = useState('');
    const [toChar, setToChar] = useState('');
    const [global, setGlobal] = useState(false);

    useEffect(() => {
        fetch(`${SCHEMAS_BASE}/manifest.json`)
            .then(r => r.ok ? r.json() : [])
            .then(manifest => {
                setSchemas(manifest);
                const lang = (navigator.language || '').split('-')[0].toLowerCase();
                const match = manifest.find(s => s.lang.includes(lang));
                if (match) setSelectedSchema(match.id);
            })
            .catch(() => {});
    }, []);

    const applySchema = useCallback(async () => {
        if (!selectedSchema) return;
        try {
            const res = await fetch(`${SCHEMAS_BASE}/${selectedSchema}.json`);
            if (!res.ok) throw new Error('Failed to load schema');
            const schemaRules = await res.json();

            let added = 0;
            const next = [...rules];
            for (const rule of schemaRules) {
                const dup = next.some(r =>
                    JSON.stringify(r.from_mods) === JSON.stringify(rule.from_mods) && r.from_key === rule.from_key
                );
                if (dup) continue;
                next.push({
                    from_mods: normalizeMods(rule.from_mods),
                    from_key: String(rule.from_key),
                    to_char: String(rule.to_char),
                    global: !!rule.global,
                });
                added++;
            }
            onChange(next);
            setSchemaStatus(added > 0 ? `Added ${added} rules` : 'No new rules (all exist)');
            setTimeout(() => setSchemaStatus(''), 2000);
        } catch {
            setSchemaStatus('Failed to load schema');
        }
    }, [selectedSchema, rules, onChange]);

    const addRule = useCallback(() => {
        const fk = fromKey.trim().toLowerCase();
        if (!fk || !toChar) return;
        onChange([...rules, { from_mods: fromMods, from_key: fk, to_char: toChar, global }]);
        setFromMods([]); setFromKey(''); setToChar(''); setGlobal(false);
    }, [fromMods, fromKey, toChar, global, rules, onChange]);

    const removeRule = useCallback((index) => {
        onChange(rules.filter((_, i) => i !== index));
    }, [rules, onChange]);

    return html`
        <section class="card">
            <h2>Char Rules</h2>
            <p>Map key combinations to specific characters. Global rules work even in excluded apps.</p>
            <div class="schema-row">
                <label for="schema-select">Locale schema:</label>
                <select class="key-input schema-select" value=${selectedSchema}
                    onChange=${(e) => setSelectedSchema(e.target.value)}>
                    <option value="">-- select --</option>
                    ${schemas.map(s => html`<option value=${s.id}>${s.name}</option>`)}
                </select>
                <button class="btn-add" onClick=${applySchema}>Apply</button>
                <span class="schema-status">${schemaStatus}</span>
            </div>
            <div class="rules-list">
                ${rules.length === 0 && html`<div class="empty-state">No char rules defined.</div>`}
                ${rules.map((rule, i) => html`
                    <div class="rule-row">
                        <div class="rule-side"><${ModChipsStatic} mods=${rule.from_mods} /> <span class="key-label">${rule.from_key}</span></div>
                        <span class="arrow">\u2192</span>
                        <div class="rule-side"><span class="key-label char-output">${rule.to_char}</span></div>
                        ${rule.global && html`<span class="global-badge">global</span>`}
                        <button class="btn-remove" onClick=${() => removeRule(i)}>\u00d7</button>
                    </div>
                `)}
            </div>
            <div class="add-rule-row">
                <div class="rule-side">
                    <${ModChips} selected=${fromMods} onChange=${setFromMods} mods=${CHAR_MODS} />
                    <input type="text" class="key-input" placeholder="from key"
                        value=${fromKey} onInput=${(e) => setFromKey(e.target.value)}
                        onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), addRule())} />
                </div>
                <span class="arrow">\u2192</span>
                <div class="rule-side">
                    <input type="text" class="key-input char-input" placeholder="output char"
                        value=${toChar} onInput=${(e) => setToChar(e.target.value)}
                        onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), addRule())} />
                </div>
                <label class="global-toggle">
                    <input type="checkbox" checked=${global} onChange=${(e) => setGlobal(e.target.checked)} />
                    <span>global</span>
                </label>
                <button class="btn-add" onClick=${addRule}>+ Add</button>
            </div>
        </section>
    `;
}
