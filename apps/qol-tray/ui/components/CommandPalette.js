import { html } from '../lib/html.js';
import { useRef, useState, useEffect, useMemo, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { getCommands } from '../palette/registry.js';

function filterCommands(commands, query) {
    if (!query) return commands;
    const q = query.toLowerCase();
    return commands.filter(c => c.label.toLowerCase().includes(q));
}

export function CommandPalette() {
    const { active, query, mode, actionQuery, activeViewId, activate, deactivate, setQuery } = usePaletteContext();
    const inputRef = useRef(null);
    const [selectedIndex, setSelectedIndex] = useState(0);

    const commands = useMemo(
        () => mode === 'action' ? filterCommands(getCommands(activeViewId), actionQuery) : [],
        [mode, activeViewId, actionQuery]
    );

    useEffect(() => {
        setSelectedIndex(0);
    }, [commands]);

    useEffect(() => {
        if (active) inputRef.current?.focus();
    }, [active]);

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
        if (e.key === 'Tab') {
            e.preventDefault();
            return;
        }
        if (mode !== 'action' || commands.length === 0) return;
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setSelectedIndex(i => Math.min(i + 1, commands.length - 1));
            return;
        }
        if (e.key === 'ArrowUp') {
            e.preventDefault();
            setSelectedIndex(i => Math.max(i - 1, 0));
            return;
        }
        if (e.key === 'Enter') {
            e.preventDefault();
            const cmd = commands[selectedIndex];
            if (cmd) executeCommand(cmd);
        }
    }, [mode, commands, selectedIndex, deactivate, executeCommand]);

    const handleClick = useCallback(() => {
        if (!active) activate();
    }, [active, activate]);

    const clampedIndex = Math.min(selectedIndex, Math.max(0, commands.length - 1));

    if (!active) {
        return html`<div class="search-bar palette-hint" onClick=${handleClick}>
            <span class="palette-hint-text">Ctrl+E to search & run actions...</span>
        </div>`;
    }

    return html`<div class="command-palette">
        <div class="search-bar">
            <input ref=${inputRef} class="search-input" type="text"
                value=${query} onInput=${handleInput} onKeyDown=${handleKeyDown}
                placeholder=${mode === 'action' ? 'Type a command...' : 'Search...'} />
        </div>
        ${mode === 'action' && commands.length > 0 && html`
            <ul class="palette-dropdown">
                ${commands.map((cmd, i) => html`
                    <li key=${cmd.id} class="palette-item ${i === clampedIndex ? 'selected' : ''}"
                        onMouseDown=${() => executeCommand(cmd)}>
                        <span class="palette-item-label">${cmd.label}</span>
                    </li>
                `)}
            </ul>
        `}
    </div>`;
}
