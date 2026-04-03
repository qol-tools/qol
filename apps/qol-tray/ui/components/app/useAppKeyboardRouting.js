import { useCallback, useLayoutEffect, useRef } from 'preact/hooks';
import { useKeyboard } from '../../hooks/useKeyboard.js';
import { usePluginConfigContext } from '../../views/plugin-config/context.js';
import { useViewKeyboardContext } from './view-keyboard-context.js';
import { createDebug } from '../../lib/debug.js';
import {
    activateSurface,
    activeContainer,
    directSurfaces,
    isVisible,
    MODAL_SELECTOR,
    parentContainer,
    surfaceContainsChildContainer,
} from '../../lib/surface-traits.js';
import { nearestSurfaceInDirection, surfaceLabel } from '../../lib/spatial-nav.js';
import { focusGridRows, nextFocusGridElement } from '../../lib/focus-grid.js';

const log = createDebug('qol:nav');
const PLUGIN_CONFIG_FIELD = '[data-plugin-config-field-id]';
const NAV_KEYS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };
const NAV_KEYS_EXTENDED = { ...NAV_KEYS, h: 'left', j: 'down', k: 'up', l: 'right' };

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
            if (surface) { surface.focus({ preventScroll: true }); return; }
            const fallback = document.querySelector('#content [data-selected-surface]');
            if (fallback) fallback.focus({ preventScroll: true });
        }
    }, [activePluginId]);

    const prevViewIdRef = useRef(activeViewId);
    useLayoutEffect(() => {
        const prev = prevViewIdRef.current;
        prevViewIdRef.current = activeViewId;
        if (prev === activeViewId) return;
        requestAnimationFrame(() => {
            const target = firstVisibleSurface('#content [data-selected-surface][data-selected="true"]')
                || firstVisibleSurface('#content [data-selected-surface]');
            if (target) target.focus({ preventScroll: true });
        });
    }, [activeViewId]);

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

function routeToView(event, viewKeyboard, cycleView) {
    if (viewKeyboard?.isBlocking?.()) {
        if (viewKeyboard.handleKey) viewKeyboard.handleKey(event);
        if (!event.defaultPrevented) globalSurfaceNav(event);
        return;
    }
    if (event.key === 'Tab') {
        if (!hasVisibleModal()) cycleView(event);
        return;
    }
    const active = document.activeElement;
    if (active && active !== document.body && !active.closest(MODAL_SELECTOR)) {
        if (viewKeyboard?.handleKey) viewKeyboard.handleKey(event);
    }
    if (!event.defaultPrevented) globalSurfaceNav(event);
}

// ---------------------------------------------------------------------------
// Global surface navigation
// ---------------------------------------------------------------------------

function globalSurfaceNav(event) {
    if (event.key === 'Escape') {
        if (ascendLayer()) { event.preventDefault(); }
        return;
    }
    const direction = NAV_KEYS_EXTENDED[event.key];
    if (direction) {
        event.preventDefault();
        navigateInActiveContainer(direction);
        return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        activateAndMaybeDescend();
    }
}

function findSelectedSurface() {
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && focused !== document.body) {
        const surface = focused.closest('[data-selected-surface]');
        if (surface && isVisible(surface)) return surface;
    }
    for (const container of document.querySelectorAll('[data-surface-container]')) {
        if (!isVisible(container)) continue;
        for (const el of directSurfaces(container)) {
            if (el.getAttribute('data-selected') === 'true') return el;
        }
    }
    return null;
}

function navigateInActiveContainer(direction) {
    const current = findSelectedSurface();
    if (!current) {
        const fallback = firstVisibleSurface('#content [data-selected-surface]');
        if (fallback) fallback.focus({ preventScroll: true });
        return;
    }
    const container = activeContainer(current);
    if (!container) return;

    const surfaces = directSurfaces(container);
    const cr = current.getBoundingClientRect();
    log(direction, 'from', surfaceLabel(current),
        'at (' + Math.round(cr.left) + ',' + Math.round(cr.top) + ')',
        '| surfaces:', surfaces.length);
    const next = nearestSurfaceInDirection(surfaces, current, direction);
    if (!next || next === current) {
        log('  -> no match. All surfaces:');
        for (const el of surfaces) {
            if (el === current) continue;
            const r = el.getBoundingClientRect();
            log('    ', surfaceLabel(el), '(' + Math.round(r.left) + ',' + Math.round(r.top) + ')');
        }
        return;
    }
    const nr = next.getBoundingClientRect();
    log('  -> RESULT:', surfaceLabel(next),
        'at (' + Math.round(nr.left) + ',' + Math.round(nr.top) + ')');
    next.focus({ preventScroll: true });
    next.scrollIntoView({ behavior: 'auto', block: 'nearest' });
}

function activateAndMaybeDescend() {
    const current = findSelectedSurface();
    if (!current) return;

    if (current.getAttribute('role') === 'tab') {
        activateSurface(current);
        return;
    }

    activateSurface(current);

    if (surfaceContainsChildContainer(current)) {
        requestAnimationFrame(() => descendIntoChild(current));
    }
}

function descendIntoChild(surface) {
    const child = surface.querySelector('[data-surface-container]');
    if (child && isVisible(child)) descendInto(child);
}

function descendInto(container) {
    const surfaces = directSurfaces(container);
    if (surfaces.length === 0) return;
    surfaces[0].focus({ preventScroll: true });
}

function ascendLayer() {
    const current = findSelectedSurface();
    const container = current ? activeContainer(current) : null;
    if (!container) return false;
    if (container.closest(MODAL_SELECTOR)) return false;

    const parent = parentContainer(container);
    if (!parent) return false;

    const parentSurfaces = directSurfaces(parent);
    const diveSource = parentSurfaces.find(el => el.hasAttribute('data-dive-source'));
    if (diveSource) diveSource.removeAttribute('data-dive-source');
    const anchor = diveSource
        || parentSurfaces.find(el => el.getAttribute('data-selected') === 'true')
        || parentSurfaces.find(el => el.contains(container))
        || parentSurfaces[0];
    if (!anchor) return false;

    anchor.focus({ preventScroll: true });
    return true;
}

// ---------------------------------------------------------------------------
// Plugin config keyboard handling
// ---------------------------------------------------------------------------

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
    const direction = NAV_KEYS[event.key];
    if (!direction) return false;
    event.preventDefault();
    blurPluginConfigFocus(detail);
    const nextFieldId = nextPluginConfigFieldId(detail, pluginConfig.selectedFieldId, direction);
    if (!nextFieldId) return true;
    pluginConfig.setSelectedFieldId(nextFieldId);
    return true;
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

    const direction = NAV_KEYS[event.key];
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

function firstVisibleSurface(selector) {
    for (const el of document.querySelectorAll(selector)) {
        if (isVisible(el)) return el;
    }
    return null;
}

function hasVisibleModal() {
    const modal = document.querySelector(MODAL_SELECTOR);
    if (!modal) return false;
    const rect = modal.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
}
