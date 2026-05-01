import { html } from '../html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
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
    const armedRef = useRef(false);

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

    if (confirming) {
        const wrapperCls = [
            'btn', 'btn-confirm', variant,
            shake && 'btn-confirm-shake',
            className,
        ].filter(Boolean).join(' ');
        return html`
            <span class=${wrapperCls} role="presentation">
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
            </span>
        `;
    }

    return html`<${Button} variant=${variant} className=${className} onActivate=${enterConfirm} ...${rest}>${children}<//>`;
}
