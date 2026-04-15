import { h, render } from 'preact';
import { App } from './components/App.js';
import { randomizeGlow } from './lib/glow.js';

try {
    if (window.localStorage?.getItem('qoltray.activePlugin')) {
        document.body.classList.add('qol-bootstrapping-dive');
    }
} catch {}

render(h(App, null), document.getElementById('app'));

new MutationObserver(() => {
    for (const el of document.querySelectorAll('.content-shell')) randomizeGlow(el);
}).observe(document.body, { childList: true, subtree: true });
