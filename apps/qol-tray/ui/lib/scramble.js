import { useState, useEffect } from 'preact/hooks';

function shuffle(len) {
    const arr = Array.from({ length: len }, (_, i) => i);
    for (let i = len - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [arr[i], arr[j]] = [arr[j], arr[i]];
    }
    return arr;
}

const CHARS = '!@#$%&*<>[]{}|/\\^?+=_0123456789';

export function useScramble(text, delay = 0, observeRef) {
    const [output, setOutput] = useState(() => scrambleAll(text));

    useEffect(() => {
        let frame = null;
        let start = null;

        const run = () => {
            if (frame) cancelAnimationFrame(frame);
            start = null;

            if (!text) { setOutput(''); return; }

            setOutput(scrambleAll(text));

            const duration = Math.min(500, 80 + text.replace(/ /g, '').length * 22);
            const order = shuffle(text.length);

            const tick = (ts) => {
                if (start === null) start = ts + delay;
                if (ts < start) { frame = requestAnimationFrame(tick); return; }

                const progress = Math.min((ts - start) / duration, 1);
                const lockedCount = Math.floor(progress * text.length);
                const lockedSet = new Set(order.slice(0, lockedCount));

                const result = Array.from(text).map((c, i) =>
                    c === ' ' ? ' ' : lockedSet.has(i) ? c : CHARS[Math.floor(Math.random() * CHARS.length)]
                ).join('');
                setOutput(result);

                if (progress < 1) { frame = requestAnimationFrame(tick); return; }
                setOutput(text);
            };

            frame = requestAnimationFrame(tick);
        };

        const el = observeRef?.current;
        if (!el) { run(); return () => { if (frame) cancelAnimationFrame(frame); }; }

        const observer = new IntersectionObserver(
            (entries) => { if (entries[0].isIntersecting) run(); },
            { threshold: 0 }
        );
        observer.observe(el);

        return () => {
            observer.disconnect();
            if (frame) cancelAnimationFrame(frame);
        };
    }, [text, delay]);

    return output;
}

function scrambleAll(text) {
    if (!text) return '';
    return Array.from(text).map(c => c === ' ' ? ' ' : CHARS[Math.floor(Math.random() * CHARS.length)]).join('');
}
