import { h, render } from 'preact';
import { App } from './components/App.js';
import { randomizeGlow } from './lib/glow.js';

render(h(App, null), document.getElementById('app'));

new MutationObserver(() => {
    for (const el of document.querySelectorAll('.content-shell')) randomizeGlow(el);
}).observe(document.body, { childList: true, subtree: true });
