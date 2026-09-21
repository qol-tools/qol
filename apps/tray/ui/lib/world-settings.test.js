import assert from 'node:assert/strict';
import { test } from 'node:test';

function storage(initial = {}) {
    const entries = new Map(Object.entries(initial));
    return {
        getItem(key) {
            return entries.has(key) ? entries.get(key) : null;
        },
        setItem(key, value) {
            entries.set(key, String(value));
        },
        removeItem(key) {
            entries.delete(key);
        },
    };
}

async function loadWorldSettings(initialStorage) {
    const originalLocalStorage = globalThis.localStorage;
    globalThis.localStorage = storage(initialStorage);
    const moduleUrl = new URL(`./world-settings.js?case=${Date.now()}-${Math.random()}`, import.meta.url);
    const module = await import(moduleUrl.href);
    return {
        module,
        restore() {
            if (originalLocalStorage === undefined) delete globalThis.localStorage;
            else globalThis.localStorage = originalLocalStorage;
        },
    };
}

test('world settings ignores legacy accent persistence', async () => {
    const { module, restore } = await loadWorldSettings({
        'qol-world-settings': JSON.stringify({ accent: 'blue', panSpeed: 9 }),
    });
    try {
        assert.equal(module.getWorldSettings().accent, undefined);
        assert.equal(module.getWorldSettings().panSpeed, 9);
    } finally {
        restore();
    }
});

test('world settings refuses to persist theme-owned accent state', async () => {
    const { module, restore } = await loadWorldSettings();
    try {
        module.setWorldSetting('accent', 'blue');

        assert.equal(module.getWorldSettings().accent, undefined);
        assert.equal(globalThis.localStorage.getItem('qol-world-settings'), null);
    } finally {
        restore();
    }
});
