import { html } from '../html.js';
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { Button } from './Button.js';

export function ConfirmButton({
    confirmWith = 'confirm',
    onActivate,
    onCancel,
    children,
    variant = 'btn-primary',
    className,
    ...rest
}) {
    const [confirming, setConfirming] = useState(false);
    const [value, setValue] = useState('');
    const [shake, setShake] = useState(false);
    const inputRef = useRef(null);
    const wrapperRef = useRef(null);
    const armedRef = useRef(false);
    const prevConfirmingRef = useRef(false);

    useEffect(() => {
        if (!confirming) return;
        const id = requestAnimationFrame(() => {
            inputRef.current?.focus();
            inputRef.current?.select?.();
            armedRef.current = true;
        });
        return () => {
            cancelAnimationFrame(id);
            armedRef.current = false;
        };
    }, [confirming]);

    useLayoutEffect(() => {
        const wasConfirming = prevConfirmingRef.current;
        prevConfirmingRef.current = confirming;
        if (!wasConfirming || confirming) return;
        if (document.activeElement && document.activeElement !== document.body) return;
        wrapperRef.current?.querySelector('button')?.focus({ preventScroll: true });
    }, [confirming]);

    const enterConfirm = useCallback(() => {
        setValue('');
        setConfirming(true);
    }, []);

    const cancel = useCallback(() => {
        if (!armedRef.current && confirming) return;
        setConfirming(false);
        setValue('');
        if (onCancel) onCancel();
    }, [confirming, onCancel]);

    const matches = (text) => text.trim().toLowerCase() === confirmWith.trim().toLowerCase();

    const submit = useCallback((event) => {
        if (!matches(value)) {
            setShake(true);
            setTimeout(() => setShake(false), 400);
            return;
        }
        setConfirming(false);
        setValue('');
        if (onActivate) onActivate(event);
    }, [value, onActivate]);

    const confirmingCls = [
        'btn', 'btn-confirm', variant,
        shake && 'btn-confirm-shake',
        className,
    ].filter(Boolean).join(' ');

    return html`
        <span ref=${wrapperRef}
            class=${confirming ? confirmingCls : ''}
            style=${confirming ? null : 'display: contents'}
            role=${confirming ? 'presentation' : null}>
            ${confirming
                ? html`
                    <input ref=${inputRef}
                        class="btn-confirm-input"
                        type="text"
                        autocomplete="off"
                        spellcheck="false"
                        placeholder=${`Type "${confirmWith}" + Enter`}
                        value=${value}
                        onInput=${(e) => setValue(e.currentTarget.value)}
                        onKeyDown=${(e) => {
                            if (e.key === 'Enter') {
                                e.preventDefault();
                                e.stopPropagation();
                                submit(e);
                            } else if (e.key === 'Escape') {
                                e.preventDefault();
                                e.stopPropagation();
                                cancel();
                            }
                        }}
                        onBlur=${cancel} />
                `
                : html`<${Button} variant=${variant} className=${className} onActivate=${enterConfirm} ...${rest}>${children}<//>`}
        </span>
    `;
}
