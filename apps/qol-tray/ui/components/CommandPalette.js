import { html } from '../lib/html.js';
import { useRef, useState, useEffect, useMemo, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { getContextualCommands, subscribeRegistry, getRegistryVersion } from '../palette/registry.js';
import init, { fuzzy_match as wasmFuzzyMatch } from '../wasm/qol_wasm.js';
import { Surface } from '../lib/components/Surface.js';
import { Peripheral } from './shell/Peripheral.js';

function filterCommands(commands, query, useWasm) {
    if (!query) return commands;
    if (useWasm) {
        return commands
            .map(c => ({ cmd: c, match: wasmFuzzyMatch(query, c.label) }))
            .filter(({ match }) => match !== null)
            .sort((a, b) => a.match.score - b.match.score)
            .map(({ cmd }) => cmd);
    }
    const q = query.toLowerCase();
    return commands.filter(c => c.label.toLowerCase().includes(q));
}

export function CommandPalette({ camera, navigation }) {
    const {
        active, query, mode, actionQuery, activeViewId, committedFilter,
        activate, deactivate, setQuery, commitFilter, clearFilter, reopenFilter,
    } = usePaletteContext();
    const inputRef = useRef(null);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [wasmLoaded, setWasmLoaded] = useState(false);
    const [registryVersion, setRegistryVersion] = useState(getRegistryVersion);

    useEffect(() => subscribeRegistry(setRegistryVersion), []);

    useEffect(() => {
        let cancelled = false;
        init()
            .then(() => {
                if (cancelled) return;
                setWasmLoaded(true);
            })
            .catch(console.error);
        return () => {
            cancelled = true;
        };
    }, []);

    const commands = useMemo(() => {
        if (!active || mode !== 'action') return [];
        return filterCommands(getContextualCommands(activeViewId), actionQuery, wasmLoaded);
    }, [active, mode, activeViewId, actionQuery, wasmLoaded, registryVersion]);

    useEffect(() => {
        setSelectedIndex(0);
    }, [commands]);

    useEffect(() => {
        if (active) inputRef.current?.focus();
    }, [active]);

    const handleBlur = useCallback(() => {
        setTimeout(() => {
            if (!inputRef.current?.matches(':focus')) deactivate();
        }, 0);
    }, [deactivate]);

    const handleInput = useCallback((e) => {
        setQuery(e.target.value);
    }, [setQuery]);

    const executeCommand = useCallback((cmd) => {
        deactivate();
        cmd.run();
    }, [deactivate]);

    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            deactivate();
            return;
        }
        if (mode === 'search') {
            if (e.key === 'Enter') {
                e.preventDefault();
                e.stopPropagation();
                if (query.trim()) commitFilter();
                else deactivate();
            }
            return;
        }
        if (commands.length === 0) return;
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setSelectedIndex(i => i + 1 >= commands.length ? 0 : i + 1);
            return;
        }
        if (e.key === 'ArrowUp') {
            e.preventDefault();
            setSelectedIndex(i => i - 1 < 0 ? commands.length - 1 : i - 1);
            return;
        }
        if (e.key === 'Enter') {
            e.preventDefault();
            e.stopPropagation();
            const cmd = commands[selectedIndex];
            if (cmd) executeCommand(cmd);
        }
    }, [mode, query, commands, selectedIndex, deactivate, commitFilter, executeCommand]);

    const handleClick = useCallback(() => {
        if (!active) activate();
    }, [active, activate]);

    const clampedIndex = Math.min(selectedIndex, Math.max(0, commands.length - 1));

    if (!active) {
        if (committedFilter) {
            return html`<${Peripheral} camera=${camera} navigation=${navigation} edge="top"
                alwaysVisible=${true} className="palette-filter-pill">
                <button class="palette-pill-body" onClick=${reopenFilter}>
                    <span class="palette-pill-tag">filter</span>
                    <span class="palette-pill-value">${committedFilter}</span>
                </button>
                <button class="palette-pill-clear" title="Clear filter" onClick=${clearFilter}>✕</button>
            <//>`;
        }
        return html`<${Peripheral} camera=${camera} navigation=${navigation} edge="top"
            occludeSelector=".world-view-slot, .world-region-label"
            className="search-bar palette-hint" onClick=${handleClick}>
            <span class="palette-hint-text">Ctrl+E to search & run actions...</span>
        <//>`;
    }

    return html`<div class="palette-layer" data-mode=${mode}>
        <div class="palette-scrim" onMouseDown=${deactivate}></div>
        <div class="command-palette">
            <div class="palette-titlebar">
                <span class="palette-title">${mode === 'action' ? 'RUN' : 'SEARCH'}</span>
                <span class="palette-hintkeys">${mode === 'action' ? '↑↓ select · ⏎ run' : '⏎ lock filter · esc cancel'}</span>
            </div>
            <div class="search-bar">
                <input ref=${inputRef} class="search-input" type="text"
                    value=${query} onInput=${handleInput} onKeyDown=${handleKeyDown} onBlur=${handleBlur}
                    placeholder=${mode === 'action' ? 'Type a command...' : 'Filter this view...'} />
            </div>
            ${mode === 'action' && commands.length > 0 && html`
                <ul class="palette-dropdown">
                    ${commands.map((cmd, i) => html`
                        <${Surface} as="li" key=${cmd.id}
                            className="palette-item ${i === clampedIndex ? 'selected' : ''}"
                            selected=${i === clampedIndex}
                            data-selected-surface-priority="10"
                            onMouseDown=${() => executeCommand(cmd)}>
                            <span class="palette-item-label" data-selected-text="">${cmd.label}</span>
                        <//>
                    `)}
                </ul>
            `}
        </div>
    </div>`;
}
