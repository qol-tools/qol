import { useCallback, useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { useKeyboard } from '../lib/hooks/useKeyboard.js';
import { usePluginConfigContext } from '../views/plugin-config/context.js';
import { useViewKeyboardContext } from './view-keyboard-context.js';
import { createDebug, elLabel } from '../lib/debug.js';
import { nearestSurfaceToCenter, isInViewport } from '../lib/viewport-spatial.js';
import {
    activeContainer,
    directSurfaces,
    isVisible,
    MODAL_SELECTOR,
    parentContainer,
    surfaceContainsChildContainer,
} from '../lib/surface-traits.js';
import { nearestSurfaceInDirection, surfaceLabel } from '../lib/spatial-nav.js';
import { focusGridRows, nextFocusGridElement } from '../lib/focus-grid.js';
import { getWorldSettings } from '../lib/world-settings.js';

const log = createDebug('qol:nav');
const PLUGIN_CONFIG_FIELD = '[data-plugin-config-field-id]';
const NAV_KEYS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };
const NAV_KEYS_EXTENDED = { ...NAV_KEYS, h: 'left', j: 'down', k: 'up', l: 'right' };

function cycleIndex(current, length, reverse) {
    if (current < 0) return reverse ? length - 1 : 0;
    return reverse
        ? (current - 1 + length) % length
        : (current + 1) % length;
}
let _cameraRef = { current: null };
let _ascendRef = { current: null };

export function useAppKeyboardRouting({
    activePluginId,
    activeViewId,
    camera,
    closePluginConfig,
    switchView,
    viewOrder,
    palette,
    ascend,
    navigation,
    registry,
}) {
    const pluginConfig = usePluginConfigContext();
    const { getViewKeyboard } = useViewKeyboardContext();
    _cameraRef.current = camera;
    _ascendRef.current = ascend;
    const viewOrderRef = useRef(viewOrder);
    viewOrderRef.current = viewOrder;
    const switchViewRef = useRef(switchView);
    switchViewRef.current = switchView;
    const cyclePluginSection = useCallback((shiftKey) => {
        if (!activePluginId || !navigation?.getCurrentConfinement?.()) return false;
        const target = registry?.getDiveTargetForSource?.(`[data-plugin-id="${activePluginId}"]`);
        const pages = target?.pages || [];
        if (pages.length <= 1) return false;
        const current = navigation.getCurrentAnchor()?.pageId;
        const idx = Math.max(0, pages.indexOf(current));
        const nextId = pages[cycleIndex(idx, pages.length, shiftKey)];
        log('tab:', current, '→', nextId, '(section)');
        navigation.setCurrentAnchor({ pageId: nextId });
        const s = getWorldSettings();
        navigation.gotoAnchor(
            { pageId: nextId },
            { respectKnob: false, resetZoom: s.resetZoomOnNav ? s.defaultZoom : null },
        );
        return true;
    }, [activePluginId, navigation, registry]);

    const cycleTopLevelView = useCallback((shiftKey) => {
        const order = viewOrderRef.current;
        const idx = order.indexOf(activeViewId);
        const nextIdx = cycleIndex(idx, order.length, shiftKey);
        const nextId = order[nextIdx];
        log('tab:', activeViewId, '→', nextId, `(idx=${idx} next=${nextIdx} len=${order.length} order=${order.join(',')})`);
        switchViewRef.current(nextId);
    }, [activeViewId]);

    const cycleView = useCallback((event) => {
        event.preventDefault();
        if (cyclePluginSection(event.shiftKey)) return;
        if (navigation?.stackDepth?.() > 0) return;
        cycleTopLevelView(event.shiftKey);
    }, [cyclePluginSection, cycleTopLevelView, navigation]);

    const prevPluginIdRef = useRef(activePluginId);
    useLayoutEffect(() => {
        const wasOpen = prevPluginIdRef.current;
        prevPluginIdRef.current = activePluginId;
        if (wasOpen && !activePluginId) {
            const surface = document.querySelector('#viewport [data-selected-surface][data-selected="true"]');
            if (surface) { surface.focus({ preventScroll: true }); return; }
            const fallback = document.querySelector('#viewport [data-selected-surface]');
            if (fallback) fallback.focus({ preventScroll: true });
        }
    }, [activePluginId]);

    const prevViewIdRef = useRef(activeViewId);
    useLayoutEffect(() => {
        const prev = prevViewIdRef.current;
        prevViewIdRef.current = activeViewId;
        if (prev === activeViewId) return;
        requestAnimationFrame(() => {
            const slot = document.querySelector(`.world-view-slot[data-view-id="${activeViewId}"]`);
            if (!slot) { log('viewChange: no slot for', activeViewId); return; }
            // Skip if CTRL+snap already focused a surface in this view
            const focused = document.activeElement;
            if (focused && focused !== document.body && slot.contains(focused)) {
                log('viewChange:', activeViewId, '→ already focused:', surfaceLabel(focused));
                return;
            }
            const surface = slot.querySelector('[data-selected-surface]');
            log('viewChange:', activeViewId, '→', surface ? surfaceLabel(surface) : 'no surfaces');
            if (!surface) return;
            surface.focus({ preventScroll: true });
            if (surface instanceof HTMLInputElement || surface instanceof HTMLTextAreaElement) {
                const end = surface.value?.length ?? 0;
                surface.setSelectionRange?.(end, end);
            }
        });
    }, [activeViewId]);

    useKeyboard(useCallback((event) => {
        const viewKeyboard = getViewKeyboard(activeViewId);
        if (handlePaletteToggle(event, palette, activePluginId)) return;
        if (palette.active && event.key !== 'Tab') return;
        if (event.key === 'Tab' && activePluginId && !viewKeyboard?.isBlocking?.()) {
            event.preventDefault();
            cycleView(event);
            return;
        }
        if (activePluginId) return delegateToPluginConfig(event, pluginConfig, closePluginConfig);
        routeToView(event, viewKeyboard, cycleView);
    }, [activePluginId, activeViewId, closePluginConfig, cycleView, getViewKeyboard, palette, pluginConfig]));
}

function handlePaletteToggle(event, palette, activePluginId) {
    if (!(event.ctrlKey || event.metaKey) || event.key !== 'e') return false;
    event.preventDefault();
    if (!palette.active && !activePluginId) palette.activate();
    return true;
}

function routeToView(event, viewKeyboard, cycleView) {
    if (event.key === 'Tab') {
        event.preventDefault();
        if (hasVisibleModal()) {
            if (viewKeyboard?.isBlocking?.() && viewKeyboard.handleKey) {
                viewKeyboard.handleKey(event);
            }
            return;
        }
        cycleView(event);
        return;
    }
    if (viewKeyboard?.isBlocking?.()) {
        if (viewKeyboard.handleKey) viewKeyboard.handleKey(event);
        if (!event.defaultPrevented) globalSurfaceNav(event);
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

function isEditableInput(el) {
    if (!(el instanceof HTMLElement)) return false;
    if (el.tagName === 'TEXTAREA') return true;
    if (el.tagName === 'INPUT' && el.type !== 'button' && el.type !== 'checkbox' && el.type !== 'radio') return true;
    if (el.contentEditable === 'true') return true;
    return false;
}

function isScrollSurfaceActive(el) {
    if (!(el instanceof HTMLElement)) return false;
    return el.hasAttribute('data-scroll-surface-active');
}

function globalSurfaceNav(event) {
    if (event.key === 'Escape') {
        if (isScrollSurfaceActive(document.activeElement)) {
            event.preventDefault();
            document.activeElement.dispatchEvent(new CustomEvent('exit-scroll-mode', { bubbles: true }));
            return;
        }
        if (ascendLayer()) { event.preventDefault(); }
        return;
    }
    if (isEditableInput(document.activeElement)) return;
    if (isScrollSurfaceActive(document.activeElement)) return;
    const direction = NAV_KEYS_EXTENDED[event.key];
    if (direction) {
        event.preventDefault();
        navigateInActiveContainer(direction);
        return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        activateAndMaybeDescend(event);
    }
}

function findSelectedSurface() {
    const vp = document.getElementById('viewport');
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && focused !== document.body) {
        const surface = focused.closest('[data-selected-surface]');
        if (surface && isVisible(surface)) return surface;
    }
    for (const container of document.querySelectorAll('[data-surface-container]')) {
        if (!isVisible(container)) continue;
        for (const el of directSurfaces(container)) {
            if (el.getAttribute('data-selected') === 'true' && isInViewport(el, vp)) return el;
        }
    }
    return null;
}

function navigateInActiveContainer(direction) {
    const current = findSelectedSurface();
    if (!current) {
        log('arrow', direction, '→ no current, snap fallback');
        const { surface: fallback } = nearestSurfaceToCenter();
        if (fallback) {
            log('arrow', direction, '→ snap:', surfaceLabel(fallback));
            fallback.focus({ preventScroll: true });
        } else {
            log('arrow', direction, '→ snap: nothing found');
        }
        return;
    }
    const container = activeContainer(current);
    if (!container) { log('arrow', direction, '→ no container'); return; }

    const currentViewId = current.closest('[data-view-id]')?.dataset?.viewId || null;
    const surfaces = directSurfaces(container).filter(el => {
        if (!currentViewId) return true;
        return el.closest('[data-view-id]')?.dataset?.viewId === currentViewId;
    });
    const next = nearestSurfaceInDirection(surfaces, current, direction);
    if (!next || next === current) return;

    const cr = current.getBoundingClientRect();
    const nr = next.getBoundingClientRect();
    const slot = next.closest('.world-view-slot');
    log('arrow', direction, surfaceLabel(current),
        `(${Math.round(cr.left)},${Math.round(cr.top)})`, '→',
        surfaceLabel(next), `(${Math.round(nr.left)},${Math.round(nr.top)})`,
        'view:', slot?.dataset?.viewId || '?');

    focusWithoutScroll(next);
    // Camera follow is handled globally by WorldViewport's focusin listener
}

function focusWithoutScroll(el) {
    el.focus({ preventScroll: true });
}

function activateAndMaybeDescend(keyEvent) {
    const current = findSelectedSurface();
    if (!current) return;
    dispatchModifierClick(current, keyEvent);
    if (keyEvent?.shiftKey || keyEvent?.ctrlKey || keyEvent?.metaKey) return;
    if (current.getAttribute('data-dive-target')) return;
    if (surfaceContainsChildContainer(current)) {
        requestAnimationFrame(() => descendIntoChild(current));
    }
}

function dispatchModifierClick(el, event) {
    if (!(el instanceof HTMLElement)) return;
    el.dispatchEvent(new MouseEvent('click', {
        bubbles: true,
        cancelable: true,
        shiftKey: Boolean(event?.shiftKey),
        ctrlKey: Boolean(event?.ctrlKey),
        metaKey: Boolean(event?.metaKey),
        altKey: Boolean(event?.altKey),
    }));
}

function descendIntoChild(surface) {
    const child = surface.querySelector('[data-surface-container]');
    if (child && isVisible(child)) descendInto(child);
}

function restoreDiveSourceFocus() {
    const source = document.querySelector('[data-dive-source]');
    if (!(source instanceof HTMLElement)) return;
    source.removeAttribute('data-dive-source');
    if (!isVisible(source)) return;
    source.focus({ preventScroll: true });
}

function descendInto(container) {
    const surfaces = directSurfaces(container);
    if (surfaces.length === 0) return;
    surfaces[0].focus({ preventScroll: true });
}

function ascendLayer() {
    const camera = _cameraRef.current;
    if (camera && camera.layer < 0 && _ascendRef.current) {
        const result = _ascendRef.current();
        requestAnimationFrame(restoreDiveSourceFocus);
        return result;
    }

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

function activeSectionDetail(pluginConfig) {
    const sectionId = pluginConfig?.activeSection?.id;
    const pluginId = pluginConfig?.pluginId;
    if (!sectionId || !pluginId) return document.querySelector('.plugin-config-detail');
    const slot = document.querySelector(`[data-view-id="${CSS.escape(`${pluginId}-${sectionId}`)}"]`);
    return slot?.querySelector('.plugin-config-detail') || null;
}

function delegateToPluginConfig(event, pluginConfig, closePluginConfig) {
    if (!pluginConfig) {
        if (event.key === 'Escape') { event.preventDefault(); closePluginConfig(); }
        return;
    }
    if (pluginConfig.loading || !pluginConfig.sections?.length) {
        if (event.key === 'Escape') { event.preventDefault(); closePluginConfig(); }
        return;
    }
    const detail = activeSectionDetail(pluginConfig);
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

const DIRECT_EDIT_HANDLERS = {
    string: (event, detail, _pluginConfig, field) => startStringFieldEdit(event, detail, field.id),
    number: (event, detail, _pluginConfig, field) =>
        field.variant === 'slider' ? false : startNumberFieldEdit(event, detail, field.id),
};

const FIELD_ACTION_HANDLERS = {
    boolean: (event, _detail, pluginConfig, field) => handleBooleanFieldAction(event, pluginConfig, field),
    select: (event, detail, pluginConfig, field) => handleSelectFieldAction(event, detail, pluginConfig, field),
    string: (event, detail, _pluginConfig, field) => handleTextFieldActivation(event, detail, field.id),
    number: (event, detail, _pluginConfig, field) =>
        field.variant === 'slider'
            ? handleSliderFieldAction(event, detail, field)
            : handleNumberFieldActivation(event, detail, field.id),
    action: (event, detail, _pluginConfig, field) => handleActionFieldActivation(event, detail, field.id),
    color: (event, detail, pluginConfig, field) => handleColorFieldAction(event, detail, pluginConfig, field),
};

function handlePluginConfigDirectEdit(event, detail, field) {
    const handler = DIRECT_EDIT_HANDLERS[field.kind];
    return handler ? handler(event, detail, null, field) : false;
}

function handlePluginConfigFieldAction(event, detail, pluginConfig, field) {
    const handler = FIELD_ACTION_HANDLERS[field.kind];
    if (handler) return handler(event, detail, pluginConfig, field);
    return handleGenericFieldActivation(event, detail, field.id);
}

function handleSliderFieldAction(event, detail, field) {
    const active = document.activeElement;
    const thumbFocused = active?.hasAttribute?.('data-slider-thumb');
    const fieldEl = queryFieldElement(detail, field.id);

    if (thumbFocused) {
        if (NAV_KEYS[event.key]) return true;
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            active.dispatchEvent(new CustomEvent('slider-commit'));
            fieldEl?.focus({ preventScroll: true });
            return true;
        }
        return false;
    }

    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        const thumb = fieldEl?.querySelector('[data-slider-thumb]');
        if (thumb) thumb.focus({ preventScroll: true });
        return true;
    }
    return false;
}

function handleActionFieldActivation(event, detail, fieldId) {
    if (event.key !== 'Enter' && event.key !== ' ') return false;
    event.preventDefault();
    const fieldElement = queryFieldElement(detail, fieldId);
    const button = fieldElement?.querySelector('button:not(.btn-remove)');
    if (button instanceof HTMLElement && isInteractable(button)) button.click();
    return true;
}


function handleColorFieldAction(event, detail, pluginConfig, field) {
    const active = document.activeElement;
    const colorThumbFocused = active?.hasAttribute?.('data-color-thumb');
    const brightnessThumbFocused = active?.hasAttribute?.('data-brightness-thumb');
    const fieldEl = queryFieldElement(detail, field.id);

    if (colorThumbFocused || brightnessThumbFocused) {
        if (NAV_KEYS[event.key]) return true;
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            const commitEvent = colorThumbFocused ? 'color-commit' : 'brightness-commit';
            fieldEl?.dispatchEvent(new CustomEvent(commitEvent));
            fieldEl?.focus({ preventScroll: true });
            return true;
        }
        return false;
    }

    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        const thumb = fieldEl?.querySelector('[data-color-thumb]');
        if (thumb) thumb.focus({ preventScroll: true });
        return true;
    }
    if (event.key === 'PageUp' || event.key === 'PageDown') {
        event.preventDefault();
        const brightnessThumb = fieldEl?.querySelector('[data-brightness-thumb]');
        if (brightnessThumb) brightnessThumb.focus({ preventScroll: true });
        return true;
    }
    return false;
}

function isInteractable(el) {
    if (!(el instanceof HTMLElement)) return false;
    if (el.matches(':disabled')) return false;
    if (el.getAttribute('aria-hidden') === 'true') return false;
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
    if (!isInteractable(trigger)) return false;
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
    if (isInteractable(input)) return input;
    const button = fieldElement.querySelector('button:not(.btn-remove)');
    if (isInteractable(button)) return button;
    return null;
}

function handleTextFieldActivation(event, detail, fieldId) {
    if (event.key !== 'Enter') return false;
    event.preventDefault();
    const input = queryFieldElement(detail, fieldId)?.querySelector('.text-input');
    if (!isInteractable(input)) return true;
    input.focus();
    input.select();
    return true;
}

function handleNumberFieldActivation(event, detail, fieldId) {
    if (event.key !== 'Enter') return false;
    event.preventDefault();
    const display = queryFieldElement(detail, fieldId)?.querySelector('.number-display');
    if (!isInteractable(display)) return true;
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
    if (!isInteractable(input)) return true;
    focusTextInput(input, event.key);
    return true;
}

function startNumberFieldEdit(event, detail, fieldId) {
    if (!isNumberEditKey(event)) return false;
    event.preventDefault();
    const display = queryFieldElement(detail, fieldId)?.querySelector('.number-display');
    if (!isInteractable(display)) return true;
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
        const editable = isTextEditable(active) ? active : target;
        editable.blur();
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
    if (active?.hasAttribute('data-color-thumb')) return false;
    if (active?.hasAttribute('data-brightness-thumb')) return false;
    if (active?.hasAttribute('data-slider-thumb')) return false;

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
    )).filter(isInteractable);
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

function hasVisibleModal() {
    const modal = document.querySelector(MODAL_SELECTOR);
    if (!modal) return false;
    const rect = modal.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
}
