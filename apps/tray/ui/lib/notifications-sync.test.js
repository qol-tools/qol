import assert from 'node:assert/strict';
import { test } from 'node:test';

function installBrowserGlobals(fetchImpl) {
    const originalWindow = globalThis.window;
    const originalFetch = globalThis.fetch;

    globalThis.fetch = fetchImpl;
    globalThis.window = globalThis;

    return {
        restore() {
            if (originalWindow === undefined) delete globalThis.window;
            else globalThis.window = originalWindow;
            if (originalFetch === undefined) delete globalThis.fetch;
            else globalThis.fetch = originalFetch;
        },
    };
}

function freshModule() {
    return new URL(`./notifications-sync.js?case=${Date.now()}-${Math.random()}`, import.meta.url);
}

test('notification save failures preserve local state and defer feedback to caller', async () => {
    const calls = [];
    const harness = installBrowserGlobals(async (url, options) => {
        calls.push({ url, options });
        return new Response('invalid notifications', { status: 400 });
    });
    try {
        const notifications = await import(freshModule().href);

        assert.equal(notifications.getSystemNotifications(), false);
        await assert.rejects(
            () => notifications.setSystemNotifications(true),
            /invalid notifications/,
        );

        assert.equal(notifications.getSystemNotifications(), false);
        assert.equal(calls.length, 1);
        assert.equal(calls[0].url, '/api/notifications');
        assert.equal(calls[0].options.method, 'PUT');
        assert.equal(calls[0].options.qolSuppressErrorToast, undefined);
    } finally {
        harness.restore();
    }
});

test('notification save commit notifies subscribers', async () => {
    const harness = installBrowserGlobals(async () => {
        return new Response(JSON.stringify({ useSystemNotifications: true }), { status: 200 });
    });
    try {
        const notifications = await import(freshModule().href);
        const seen = [];
        const unsubscribe = notifications.subscribeSystemNotifications((value) => seen.push(value));

        await notifications.setSystemNotifications(true);

        assert.equal(notifications.getSystemNotifications(), true);
        assert.deepEqual(seen, [true]);
        unsubscribe();
    } finally {
        harness.restore();
    }
});

test('notification setting initializes from the boot payload', async () => {
    const harness = installBrowserGlobals(async () => {
        throw new Error('no fetch expected');
    });
    try {
        globalThis.window.__QOL_BOOT__ = { notifications: { useSystemNotifications: true } };
        const notifications = await import(freshModule().href);
        assert.equal(notifications.getSystemNotifications(), true);
        globalThis.window.__QOL_BOOT__ = undefined;
    } finally {
        harness.restore();
    }
});
