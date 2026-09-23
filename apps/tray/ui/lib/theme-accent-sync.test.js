import assert from 'node:assert/strict';
import { test } from 'node:test';

function installBrowserGlobals(fetchImpl) {
    const originalWindow = globalThis.window;
    const originalFetch = globalThis.fetch;
    const originalDispatchEvent = globalThis.dispatchEvent;
    const originalDocument = globalThis.document;
    const originalLocation = globalThis.location;
    const originalCustomEvent = globalThis.CustomEvent;
    const events = [];
    const style = new Map();

    globalThis.fetch = fetchImpl;
    globalThis.dispatchEvent = (event) => {
        events.push(event);
        return true;
    };
    globalThis.window = globalThis;
    globalThis.document = {
        documentElement: {
            style: {
                setProperty(name, value) {
                    style.set(name, value);
                },
            },
        },
    };
    globalThis.location = { origin: 'http://localhost' };
    globalThis.CustomEvent = class CustomEvent {
        constructor(type, init = {}) {
            this.type = type;
            this.detail = init.detail;
        }
    };

    return {
        events,
        style,
        restore() {
            if (originalWindow === undefined) delete globalThis.window;
            else globalThis.window = originalWindow;
            if (originalFetch === undefined) delete globalThis.fetch;
            else globalThis.fetch = originalFetch;
            if (originalDispatchEvent === undefined) delete globalThis.dispatchEvent;
            else globalThis.dispatchEvent = originalDispatchEvent;
            if (originalDocument === undefined) delete globalThis.document;
            else globalThis.document = originalDocument;
            if (originalLocation === undefined) delete globalThis.location;
            else globalThis.location = originalLocation;
            if (originalCustomEvent === undefined) delete globalThis.CustomEvent;
            else globalThis.CustomEvent = originalCustomEvent;
        },
    };
}

test('theme accent save failures preserve local state and defer feedback to caller', async () => {
    const calls = [];
    const harness = installBrowserGlobals(async (url, options) => {
        calls.push({ url, options });
        return new Response('invalid accent', { status: 400 });
    });
    try {
        const moduleUrl = new URL(`./theme-accent-sync.js?case=${Date.now()}-${Math.random()}`, import.meta.url);
        const themeAccent = await import(moduleUrl.href);

        assert.equal(themeAccent.getThemeAccent(), null);
        await assert.rejects(
            () => themeAccent.setThemeAccent('blue'),
            /invalid accent/,
        );

        assert.equal(themeAccent.getThemeAccent(), null);
        assert.equal(calls.length, 1);
        assert.equal(calls[0].url, '/api/theme/accent');
        assert.equal(calls[0].options.qolSuppressErrorToast, undefined);
        assert.deepEqual(harness.events, []);
        assert.deepEqual([...harness.style.entries()], []);
    } finally {
        harness.restore();
    }
});
