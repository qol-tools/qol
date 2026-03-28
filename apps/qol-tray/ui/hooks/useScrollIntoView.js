import { useEffect, useRef } from 'preact/hooks';
import { findActiveSelectedSurface } from '../lib/selected-surface.js';

const NAV_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight']);
const FOCUS_SCROLL_KEYS = new Set([
    'ArrowUp',
    'ArrowDown',
    'ArrowLeft',
    'ArrowRight',
    'Tab',
    'Enter',
    ' ',
    'Home',
    'End',
    'PageUp',
    'PageDown',
]);
function scrollForKeyboardSelection(target) {
    const mode = target.getAttribute('data-scroll-follow-mode');
    if (mode === 'nearest') {
        target.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
        return;
    }

    const scroller = findScrollParent(target);
    if (!scroller) {
        target.scrollIntoView({ behavior: 'auto', block: 'center', inline: 'nearest' });
        return;
    }

    const scrollerRect = scroller.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const targetTop = scroller.scrollTop + targetRect.top - scrollerRect.top;
    const desiredTop = targetTop - (scroller.clientHeight - targetRect.height) / 2;
    const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    const nextTop = clamp(desiredTop, 0, maxTop);

    scroller.scrollTo({ top: nextTop, behavior: 'auto' });
}

function findScrollParent(target) {
    let current = target.parentElement;

    while (current && current !== document.body) {
        if (isScrollable(current)) {
            return current;
        }
        current = current.parentElement;
    }

    const root = document.scrollingElement;
    if (root instanceof HTMLElement) {
        return root;
    }
    return null;
}

function isScrollable(el) {
    const style = getComputedStyle(el);
    if (style.overflowY !== 'auto' && style.overflowY !== 'scroll') {
        return false;
    }
    return el.scrollHeight > el.clientHeight + 1;
}

function clamp(value, min, max) {
    if (value < min) {
        return min;
    }
    if (value > max) {
        return max;
    }
    return value;
}

export function useScrollFollow() {
    const navKeyPressedRef = useRef(false);
    const keyboardScrollPendingRef = useRef(false);
    const scheduledSelectionRef = useRef(false);
    const selectionTargetRef = useRef(null);
    const programmaticScrollRef = useRef(false);
    const programmaticScrollFrameRef = useRef(0);

    useEffect(() => {
        const setInputMode = (mode) => {
            const app = document.querySelector('.app-container');
            if (!(app instanceof HTMLElement)) {
                return;
            }
            app.dataset.inputMode = mode;
        };

        const markProgrammaticScroll = () => {
            programmaticScrollRef.current = true;
            if (programmaticScrollFrameRef.current) {
                cancelAnimationFrame(programmaticScrollFrameRef.current);
            }
            programmaticScrollFrameRef.current = requestAnimationFrame(() => {
                programmaticScrollFrameRef.current = 0;
                programmaticScrollRef.current = false;
            });
        };

        function scrollSelectedSurface() {
            scheduledSelectionRef.current = false;
            if (!navKeyPressedRef.current) return;

            const target = findActiveSelectedSurface({
                currentTarget: selectionTargetRef.current,
                includeFocus: false,
            });
            if (!(target instanceof HTMLElement)) return;

            selectionTargetRef.current = target;
            navKeyPressedRef.current = false;
            markProgrammaticScroll();
            scrollForKeyboardSelection(target);
        }

        function scheduleSelectedSurfaceScroll() {
            if (!navKeyPressedRef.current) return;
            if (scheduledSelectionRef.current) return;
            scheduledSelectionRef.current = true;
            queueMicrotask(scrollSelectedSurface);
        }

        const onKeyDown = (e) => {
            setInputMode('keyboard');
            if (NAV_KEYS.has(e.key)) navKeyPressedRef.current = true;
            if (!FOCUS_SCROLL_KEYS.has(e.key)) return;
            keyboardScrollPendingRef.current = true;
        };
        const onPointerDown = () => {
            setInputMode('mouse');
            keyboardScrollPendingRef.current = false;
            navKeyPressedRef.current = false;
        };
        const onWheel = () => {
            setInputMode('mouse');
            keyboardScrollPendingRef.current = false;
            navKeyPressedRef.current = false;
        };
        const onScroll = () => {
            if (programmaticScrollRef.current) {
                return;
            }
            setInputMode('mouse');
        };
        const onFocusIn = (e) => {
            if (!keyboardScrollPendingRef.current) return;
            const target = e.target;
            if (!(target instanceof HTMLElement)) return;
            queueMicrotask(() => {
                if (document.activeElement !== target) return;
                scrollForKeyboardSelection(target);
            });
        };
        const observer = new MutationObserver(() => {
            scheduleSelectedSurfaceScroll();
        });
        document.addEventListener('keydown', onKeyDown, true);
        document.addEventListener('pointerdown', onPointerDown, true);
        document.addEventListener('wheel', onWheel, { capture: true, passive: true });
        document.addEventListener('scroll', onScroll, true);
        document.addEventListener('focusin', onFocusIn, true);
        observer.observe(document.body, {
            attributes: true,
            attributeFilter: ['data-selected', 'data-selected-surface', 'data-selected-surface-priority'],
            childList: true,
            subtree: true,
        });
        return () => {
            document.removeEventListener('keydown', onKeyDown, true);
            document.removeEventListener('pointerdown', onPointerDown, true);
            document.removeEventListener('wheel', onWheel, true);
            document.removeEventListener('scroll', onScroll, true);
            document.removeEventListener('focusin', onFocusIn, true);
            observer.disconnect();
            if (programmaticScrollFrameRef.current) {
                cancelAnimationFrame(programmaticScrollFrameRef.current);
            }
        };
    }, []);
}
