import { html } from '../html.js';
import { useRef } from 'preact/hooks';
import { useScramble } from '../scramble.js';

export function ScrambleText({ text, delay = 0 }) {
    const ref = useRef(null);
    const output = useScramble(text, delay, ref);
    return html`<span ref=${ref}>${output}</span>`;
}
