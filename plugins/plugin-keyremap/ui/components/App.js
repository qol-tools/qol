import { html } from '../lib/html.js';
import { useEffect, useCallback } from 'preact/hooks';
import { useConfig } from '../hooks/useConfig.js';
import { useApps } from '../hooks/useApps.js';
import { EnableToggle } from './EnableToggle.js';
import { ExcludedApps } from './ExcludedApps.js';
import { KeyRules } from './KeyRules.js';
import { CharRules } from './CharRules.js';
import { MouseRules } from './MouseRules.js';
import { ScrollRules } from './ScrollRules.js';
import { SaveBar } from './SaveBar.js';

export function App() {
    const { config, setConfig, loading, save, saveStatus, saveError, warnings, clearWarnings } = useConfig();
    const { apps: allApps } = useApps();

    useEffect(() => {
        const handler = (e) => {
            if (e.key === 's' && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                save();
            }
        };
        document.addEventListener('keydown', handler);
        return () => document.removeEventListener('keydown', handler);
    }, [save]);

    const updateField = useCallback((field, value) => {
        setConfig(prev => ({ ...prev, [field]: value }));
    }, [setConfig]);

    if (loading) return html`<main class="panel"><p>Loading...</p></main>`;

    return html`
        <main class="panel">
            <header class="hero">
                <h1>Key Remap Settings</h1>
                <p>Configure keyboard, mouse, and scroll remapping rules for a Windows-like experience on macOS.</p>
            </header>

            <${EnableToggle}
                enabled=${config.enabled}
                onChange=${(v) => updateField('enabled', v)}
            />

            <${ExcludedApps}
                apps=${config.excluded_apps}
                allApps=${allApps}
                onAdd=${(bid) => updateField('excluded_apps', [...config.excluded_apps, bid])}
                onRemove=${(i) => updateField('excluded_apps', config.excluded_apps.filter((_, idx) => idx !== i))}
            />

            <${KeyRules}
                rules=${config.key_rules}
                onChange=${(v) => updateField('key_rules', v)}
                onClearWarnings=${clearWarnings}
            />

            <${CharRules}
                rules=${config.char_rules}
                onChange=${(v) => updateField('char_rules', v)}
            />

            <${MouseRules}
                rules=${config.mouse_rules}
                onChange=${(v) => updateField('mouse_rules', v)}
            />

            <${ScrollRules}
                rules=${config.scroll_rules}
                onChange=${(v) => updateField('scroll_rules', v)}
            />

            <${SaveBar}
                onSave=${save}
                saveStatus=${saveStatus}
                saveError=${saveError}
                warnings=${warnings}
            />
        </main>
    `;
}
