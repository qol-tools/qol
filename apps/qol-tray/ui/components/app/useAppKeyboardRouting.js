import { useCallback, useLayoutEffect, useRef } from 'preact/hooks';
import { useKeyboard } from '../../hooks/useKeyboard.js';
import { usePluginConfigContext } from '../../views/plugin-config/context.js';
import { useViewKeyboardContext } from './view-keyboard-context.js';

const PLUGIN_CONFIG_FIELD = '[data-plugin-config-field-id]';
const ROW_ALIGNMENT_THRESHOLD = 6;
export function useAppKeyboardRouting({
    activePluginId,
    activeViewId,
    closePluginConfig,
    switchView,
    viewOrder,
    palette
}) {
    const pluginConfig = usePluginConfigContext();
    const { getViewKeyboard } = useViewKeyboardContext();
    const cycleView = useCallback((event) => {
        event.preventDefault();
        const idx = viewOrder.indexOf(activeViewId);
        const next = event.shiftKey
            ? (idx - 1 + viewOrder.length) % viewOrder.length
            : (idx + 1) % viewOrder.length;
        switchView(viewOrder[next]);
    }, [activeViewId, switchView, viewOrder]);

    const prevPluginIdRef = useRef(activePluginId);
    useLayoutEffect(() => {
        const wasOpen = prevPluginIdRef.current;
        prevPluginIdRef.current = activePluginId;
        if (wasOpen && !activePluginId) {
            const surface = document.querySelector('#content [data-selected-surface][data-selected="true"]');
            if (surface) { surface.focus(); return; }
            const fallback = document.querySelector('#content [data-selected-surface]');
            if (fallback) fallback.focus();
        }
    }, [activePluginId]);

    useKeyboard(useCallback((event) => {
        const viewKeyboard = getViewKeyboard(activeViewId);
        if (handlePaletteToggle(event, palette, activePluginId, viewKeyboard)) return;
        if (palette.active && event.key !== 'Tab') return;
        if (activePluginId) return delegateToPluginConfig(event, pluginConfig, closePluginConfig);
        routeToView(event, viewKeyboard, cycleView);
    }, [activePluginId, activeViewId, closePluginConfig, cycleView, getViewKeyboard, palette, pluginConfig]));
}

function handlePaletteToggle(event, palette, activePluginId, viewKeyboard) {
    if (!(event.ctrlKey || event.metaKey) || event.key !== 'e') return false;
    event.preventDefault();
    if (!palette.active && !activePluginId && !viewKeyboard?.isBlocking?.()) palette.activate();
    return true;
}

function delegateToPluginConfig(event, pluginConfig, closePluginConfig) {
    if (!pluginConfig || pluginConfig.mode === 'ui') {
        if (event.key === 'Escape') { event.preventDefault(); closePluginConfig(); }
        return;
    }
    if (pluginConfig.loading || !pluginConfig.sections?.length) {
        if (event.key === 'Escape') { event.preventDefault(); closePluginConfig(); }
        return;
    }
    const detail = document.querySelector('.plugin-config-detail');
    if (event.key === 'Tab') {
        event.preventDefault();
        blurPluginConfigFocus(detail);
        pluginConfig.navigate(event.shiftKey ? -1 : 1);
        return;
    }
    if (!detail || pluginConfig.visibleFields.length === 0) {
        if (event.key === 'Escape') { event.preventDefault(); closePluginConfig(); }
        return;
    }
    const selectedField = pluginConfig.selectedField;
    if (!selectedField) {
        if (event.key === 'Escape') { event.preventDefault(); closePluginConfig(); }
        return;
    }
    if (handleFieldSubmode(event, detail, selectedField.id)) return;
    if (event.key === 'Escape') {
        event.preventDefault();
        blurPluginConfigFocus(detail);
        closePluginConfig();
        return;
    }
    if (isEditingText(detail)) return;
    if (handlePluginConfigDirectEdit(event, detail, selectedField)) return;
    if (handlePluginConfigFieldAction(event, detail, pluginConfig, selectedField)) return;
    handlePluginConfigMove(event, detail, pluginConfig);
}

function routeToView(event, viewKeyboard, cycleView) {
    if (viewKeyboard?.isBlocking?.()) {
        if (viewKeyboard.handleKey) viewKeyboard.handleKey(event);
        return;
    }
    if (hasVisibleModal()) return;
    if (event.key === 'Tab') {
        cycleView(event);
        return;
    }
    if (viewKeyboard?.handleKey) viewKeyboard.handleKey(event);
    if (!event.defaultPrevented) globalSurfaceNav(event);
}

const NAV_KEYS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', h: 'left', j: 'down', k: 'up', l: 'right' };

function globalSurfaceNav(event) {
    const direction = NAV_KEYS[event.key];
    if (direction) {
        event.preventDefault();
        navigateVisibleSurfaces(direction);
        return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        activateSelectedSurface();
    }
}

function navigateVisibleSurfaces(direction) {
    const surfaces = visibleSurfaceElements();
    if (surfaces.length === 0) return;

    const current = surfaces.find(el => el.getAttribute('data-selected') === 'true');
    const rows = buildSurfaceGrid(surfaces);
    if (rows.length === 0) return;

    const pos = current ? findGridPosition(rows, current) : null;
    const next = pos ? gridStep(rows, pos, direction) : rows[0][0];
    if (!next || next === current) return;

    for (const el of surfaces) el.setAttribute('data-selected', 'false');
    next.setAttribute('data-selected', 'true');
    next.focus();
}

function activateSelectedSurface() {
    const surfaces = visibleSurfaceElements();
    const current = surfaces.find(el => el.getAttribute('data-selected') === 'true');
    if (current) current.click();
}

function visibleSurfaceElements() {
    const surfaces = [];
    for (const container of document.querySelectorAll('[data-surface-container]')) {
        if (container.getClientRects().length === 0) continue;
        for (const el of container.querySelectorAll('[data-selected-surface]')) {
            if (el.getClientRects().length > 0) surfaces.push(el);
        }
    }
    return surfaces;
}

function buildSurfaceGrid(elements) {
    const positioned = elements
        .map(el => ({ el, top: el.getBoundingClientRect().top, left: el.getBoundingClientRect().left }))
        .sort((a, b) => Math.abs(a.top - b.top) > 6 ? a.top - b.top : a.left - b.left);
    const rows = [];
    for (const item of positioned) {
        const last = rows[rows.length - 1];
        if (!last || Math.abs(last[0].top - item.top) > 6) {
            rows.push([item]);
        } else {
            last.push(item);
        }
    }
    return rows.map(row => row.map(item => item.el));
}

function findGridPosition(rows, target) {
    for (let r = 0; r < rows.length; r++) {
        const c = rows[r].indexOf(target);
        if (c >= 0) return { r, c };
    }
    return null;
}

function gridStep(rows, pos, direction) {
    const { r, c } = pos;
    if (direction === 'left') return rows[r][Math.max(0, c - 1)];
    if (direction === 'right') return rows[r][Math.min(rows[r].length - 1, c + 1)];
    if (direction === 'up') {
        const prev = rows[r - 1];
        return prev ? prev[Math.min(c, prev.length - 1)] : rows[r][c];
    }
    const next = rows[r + 1];
    return next ? next[Math.min(c, next.length - 1)] : rows[r][c];
}

function handlePluginConfigDirectEdit(event, detail, field) {
    if (field.kind === 'string') return startStringFieldEdit(event, detail, field.id);
    if (field.kind === 'number') return startNumberFieldEdit(event, detail, field.id);
    return false;
}

function handlePluginConfigFieldAction(event, detail, pluginConfig, field) {
    if (field.kind === 'boolean') return handleBooleanFieldAction(event, pluginConfig, field);
    if (field.kind === 'select') return handleSelectFieldAction(event, detail, pluginConfig, field);
    if (field.kind === 'string') return handleTextFieldActivation(event, detail, field.id);
    if (field.kind === 'number') return handleNumberFieldActivation(event, detail, field.id);
    return handleGenericFieldActivation(event, detail, field.id);
}

function isActuallyFocusable(el) {
    if (!(el instanceof HTMLElement)) return false;
    if (el.matches(':disabled')) return false;
    if (el.getAttribute('aria-hidden') === 'true') return false;
    if (el.tabIndex < 0) return false;
    if (el.offsetParent === null && getComputedStyle(el).position !== 'fixed') return false;
    return true;
}

function blurPluginConfigFocus(detail) {
    const active = document.activeElement;
    if (!(active instanceof HTMLElement)) return false;
    if (!detail?.contains(active)) return false;
    if (active === document.body) return false;
    active.blur();
    return true;
}

function isTextEditable(el) {
    return el.matches('input:not([type="checkbox"]):not([type="radio"]):not([type="button"]), textarea, [contenteditable="true"]');
}

function isEditingText(detail) {
    const active = document.activeElement;
    if (!(active instanceof HTMLElement)) return false;
    if (!detail?.contains(active)) return false;
    return active.matches('.text-input, .key-input, .number-edit, textarea');
}

function handleBooleanFieldAction(event, pluginConfig, field) {
    if (event.key === 'ArrowLeft') {
        event.preventDefault();
        updatePluginConfigField(pluginConfig, field, false);
        return true;
    }
    if (event.key === 'ArrowRight') {
        event.preventDefault();
        updatePluginConfigField(pluginConfig, field, true);
        return true;
    }
    if (event.key !== 'Enter' && event.key !== ' ') return false;
    event.preventDefault();
    updatePluginConfigField(pluginConfig, field, !pluginConfig.getFieldValue(field));
    return true;
}

function handleSelectFieldAction(event, detail, pluginConfig, field) {
    if (isVariantSelectorField(detail, field.id)) {
        if (event.key === 'ArrowLeft') {
            event.preventDefault();
            cycleSelectField(pluginConfig, field, -1);
            return true;
        }
        if (event.key === 'ArrowRight') {
            event.preventDefault();
            cycleSelectField(pluginConfig, field, 1);
            return true;
        }
    }
    if (event.key !== 'Enter' && event.key !== ' ') return false;
    const trigger = queryFieldElement(detail, field.id)?.querySelector('.custom-select-trigger');
    if (!isActuallyFocusable(trigger)) return false;
    event.preventDefault();
    trigger.click();
    return true;
}

function handleGenericFieldActivation(event, detail, fieldId) {
    if (event.key !== 'Enter' && event.key !== 'ArrowRight' && event.key !== ' ') return false;
    event.preventDefault();
    const fieldElement = queryFieldElement(detail, fieldId);
    const target = firstFieldEntryPoint(fieldElement);
    if (target instanceof HTMLElement) target.focus();
    return true;
}

function firstFieldEntryPoint(fieldElement) {
    if (!(fieldElement instanceof HTMLElement)) return null;
    const input = fieldElement.querySelector('input:not([type="hidden"]):not(.btn-remove), select, [tabindex="0"]');
    if (isActuallyFocusable(input)) return input;
    const button = fieldElement.querySelector('button:not(.btn-remove)');
    if (isActuallyFocusable(button)) return button;
    return null;
}

function handleTextFieldActivation(event, detail, fieldId) {
    if (event.key !== 'Enter') return false;
    event.preventDefault();
    const input = queryFieldElement(detail, fieldId)?.querySelector('.text-input');
    if (!isActuallyFocusable(input)) return true;
    input.focus();
    input.select();
    return true;
}

function handleNumberFieldActivation(event, detail, fieldId) {
    if (event.key !== 'Enter') return false;
    event.preventDefault();
    const display = queryFieldElement(detail, fieldId)?.querySelector('.number-display');
    if (!isActuallyFocusable(display)) return true;
    display.focus();
    dispatchFieldKey(display, 'Enter');
    return true;
}

function handlePluginConfigMove(event, detail, pluginConfig) {
    const direction = keyToDirection(event.key);
    if (!direction) return false;
    event.preventDefault();
    blurPluginConfigFocus(detail);
    const nextFieldId = nextPluginConfigFieldId(detail, pluginConfig.selectedFieldId, direction);
    if (!nextFieldId) return true;
    pluginConfig.setSelectedFieldId(nextFieldId);
    return true;
}

function keyToDirection(key) {
    if (key === 'ArrowUp') return 'up';
    if (key === 'ArrowDown') return 'down';
    if (key === 'ArrowLeft') return 'left';
    if (key === 'ArrowRight') return 'right';
    return null;
}

function nextPluginConfigFieldId(detail, selectedFieldId, direction) {
    const fields = getPluginConfigFieldElements(detail);
    if (fields.length === 0) return null;

    const fallback = fields[0]?.dataset.pluginConfigFieldId || null;
    const current = selectedFieldId
        ? fields.find(f => f.dataset.pluginConfigFieldId === selectedFieldId)
        : fields[0];

    const rows = focusGridRows(fields);
    const next = nextFocusGridElement(rows, current, direction);
    return next?.dataset?.pluginConfigFieldId || fallback;
}

function getPluginConfigFieldElements(detail) {
    return Array.from(detail.querySelectorAll(PLUGIN_CONFIG_FIELD))
        .filter(el => el instanceof HTMLElement
            && (el.offsetParent !== null || getComputedStyle(el).position === 'fixed'));
}

function startStringFieldEdit(event, detail, fieldId) {
    if (!isStringEditKey(event)) return false;
    event.preventDefault();
    const input = queryFieldElement(detail, fieldId)?.querySelector('.text-input');
    if (!isActuallyFocusable(input)) return true;
    focusTextInput(input, event.key);
    return true;
}

function startNumberFieldEdit(event, detail, fieldId) {
    if (!isNumberEditKey(event)) return false;
    event.preventDefault();
    const display = queryFieldElement(detail, fieldId)?.querySelector('.number-display');
    if (!isActuallyFocusable(display)) return true;
    display.focus();
    dispatchFieldKey(display, event.key);
    return true;
}

function handleFieldSubmode(event, detail, fieldId) {
    const fieldElement = queryFieldElement(detail, fieldId);
    if (!(fieldElement instanceof HTMLElement)) return false;

    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const target = event.target instanceof HTMLElement ? event.target : active;
    if (!target) return false;
    if (target.isConnected && !fieldElement.contains(target) && !(active && fieldElement.contains(active) && active !== fieldElement)) return false;
    if (!target.isConnected) return false;
    if (active === fieldElement) return false;

    if (event.key === 'Escape') {
        event.preventDefault();
        fieldElement.focus();
        return true;
    }

    if (event.key === 'Enter' && (isTextEditable(target) || isTextEditable(active))) {
        event.preventDefault();
        fieldElement.focus();
        return true;
    }

    const matchesRemove = (el) => el?.matches('.btn-remove');
    if ((event.key === 'Delete' || event.key === 'Backspace') && (matchesRemove(active) || matchesRemove(target))) {
        event.preventDefault();
        const btn = matchesRemove(active) ? active : target;
        const stops = genericFieldStops(fieldElement);
        const idx = stops.indexOf(btn);
        const next = stops[idx + 1] || stops[idx - 1];
        btn.click();
        if (next instanceof HTMLElement) requestAnimationFrame(() => next.focus());
        return true;
    }

    const matchesInteractive = (el) => el?.matches('button, input[type="checkbox"], [role="switch"]');
    if ((event.key === 'Enter' || event.key === ' ') && (matchesInteractive(active) || matchesInteractive(target))) {
        return true;
    }

    if (active && shouldKeepHorizontalCaret(event, active)) return false;

    const direction = keyToDirection(event.key);
    if (!direction) return false;
    event.preventDefault();
    const rows = focusGridRows(genericFieldStops(fieldElement));
    const nextStop = nextFocusGridElement(rows, active || target, direction);
    if (nextStop instanceof HTMLElement) nextStop.focus();
    return true;
}

function genericFieldStops(fieldElement) {
    if (!(fieldElement instanceof HTMLElement)) return [];
    return Array.from(fieldElement.querySelectorAll(
        'input:not([type="hidden"]), select, button, [tabindex="0"]'
    )).filter(isActuallyFocusable);
}

function isStringEditKey(event) {
    if (event.ctrlKey || event.metaKey || event.altKey) return false;
    if (event.key === 'Backspace') return true;
    return event.key.length === 1;
}

function isNumberEditKey(event) {
    if (event.ctrlKey || event.metaKey || event.altKey) return false;
    if (event.key === 'Backspace') return true;
    return /^[0-9.\-]$/.test(event.key);
}

function shouldKeepHorizontalCaret(event, active) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return false;
    if (!(active instanceof HTMLInputElement) && !(active instanceof HTMLTextAreaElement)) return false;
    if (active.readOnly || active.disabled) return false;
    if (active.selectionStart === null || active.selectionEnd === null) return false;
    if (active.selectionStart !== active.selectionEnd) return true;
    if (event.key === 'ArrowLeft') return active.selectionStart > 0;
    return active.selectionEnd < active.value.length;
}

function focusGridRows(elements) {
    const positioned = elements
        .map(element => {
            const rect = element.getBoundingClientRect();
            return {
                element,
                top: rect.top,
                left: rect.left,
            };
        })
        .sort((left, right) => {
            const topDelta = Math.abs(left.top - right.top);
            if (topDelta > ROW_ALIGNMENT_THRESHOLD) return left.top - right.top;
            return left.left - right.left;
        });

    const rows = [];
    for (const item of positioned) {
        const row = rows[rows.length - 1];
        if (!row) {
            rows.push([item]);
            continue;
        }
        if (Math.abs(row[0].top - item.top) > ROW_ALIGNMENT_THRESHOLD) {
            rows.push([item]);
            continue;
        }
        row.push(item);
    }

    return rows;
}

function nextFocusGridElement(rows, active, direction) {
    if (rows.length === 0) return null;

    const position = findFocusGridPosition(rows, active);
    if (!position) return rows[0][0]?.element || null;

    const currentRow = rows[position.row];
    if (direction === 'left') return currentRow[Math.max(0, position.column - 1)]?.element || null;
    if (direction === 'right') return currentRow[Math.min(currentRow.length - 1, position.column + 1)]?.element || null;
    if (direction === 'up') {
        const previousRow = rows[position.row - 1];
        if (!previousRow) return currentRow[position.column]?.element || null;
        return previousRow[Math.min(position.column, previousRow.length - 1)]?.element || null;
    }
    const nextRow = rows[position.row + 1];
    if (!nextRow) return currentRow[position.column]?.element || null;
    return nextRow[Math.min(position.column, nextRow.length - 1)]?.element || null;
}

function findFocusGridPosition(rows, active) {
    for (let rowIndex = 0; rowIndex < rows.length; rowIndex += 1) {
        const columnIndex = rows[rowIndex].findIndex(item => item.element === active);
        if (columnIndex < 0) continue;
        return {
            row: rowIndex,
            column: columnIndex,
        };
    }
    return null;
}

function focusTextInput(input, key) {
    input.focus();
    const nextValue = key === 'Backspace' ? '' : key;
    input.value = nextValue;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const caret = nextValue.length;
    input.setSelectionRange(caret, caret);
}

function dispatchFieldKey(target, key) {
    target.dispatchEvent(new KeyboardEvent('keydown', {
        key,
        bubbles: true,
        cancelable: true,
    }));
}

function updatePluginConfigField(pluginConfig, field, value) {
    pluginConfig.setFieldValue(field, value);
    pluginConfig.bumpRender();
    pluginConfig.save();
}

function cycleSelectField(pluginConfig, field, delta) {
    const options = field.options || [];
    if (options.length === 0) return;
    const currentValue = pluginConfig.getFieldValue(field);
    const currentIndex = options.indexOf(currentValue);
    const startIndex = currentIndex < 0 ? 0 : currentIndex;
    const nextIndex = (startIndex + delta + options.length) % options.length;
    updatePluginConfigField(pluginConfig, field, options[nextIndex]);
}

function isVariantSelectorField(detail, fieldId) {
    const fieldElement = queryFieldElement(detail, fieldId);
    if (!(fieldElement instanceof HTMLElement)) return false;
    return fieldElement.classList.contains('variant-selector');
}

function queryFieldElement(detail, fieldId) {
    if (!fieldId) return null;
    return detail.querySelector(`[data-plugin-config-field-id="${CSS.escape(fieldId)}"]`);
}

function hasVisibleModal() {
    const modal = document.querySelector('.edit-modal, .confirm-modal');
    if (!modal) return false;
    const rect = modal.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
}
