import { toast } from '../lib/toast.js';

const originalFetch = window.fetch.bind(window);
const inflightGets = new Map();

window.fetch = function (url, options) {
    if (!isCoalescableGet(url, options)) {
        return wrappedFetch(url, options);
    }
    const key = typeof url === 'string' ? url : url.url ?? String(url);
    const pending = inflightGets.get(key);
    if (pending) {
        return pending.then(res => res.clone());
    }
    const promise = wrappedFetch(url, options);
    inflightGets.set(key, promise);
    const drop = () => inflightGets.delete(key);
    promise.then(drop, drop);
    return promise.then(res => res.clone());
};

function isCoalescableGet(url, options) {
    const method = (options?.method ?? 'GET').toUpperCase();
    return method === 'GET'
        && options?.body == null
        && (typeof url === 'string' || url instanceof URL);
}

async function wrappedFetch(url, options) {
    try {
        const response = await originalFetch(url, options);
        if (!response.ok) {
            toast('error', `${response.status} — ${extractPath(url)}`);
        }
        return response;
    } catch (error) {
        toast('error', error.message);
        throw error;
    }
}

function extractPath(url) {
    try { return new URL(url, location.origin).pathname; } catch { return String(url); }
}

function buildError(response, message) {
    const error = new Error(message || `Request failed (${response.status})`);
    error.status = response.status;
    error.response = response;
    return error;
}

async function readErrorMessage(response) {
    try {
        const text = (await response.text())?.trim();
        if (text) return text;
    } catch {}
    return `Request failed (${response.status})`;
}

export async function readResponseText(response) {
    try {
        return (await response.text())?.trim() || '';
    } catch {
        return '';
    }
}

export function jsonRequest(method, payload, options = {}) {
    const baseHeaders = options.headers || {};
    const headers = { ...baseHeaders, 'Content-Type': 'application/json' };
    const request = { ...options, method, headers };
    if (payload !== undefined) {
        request.body = JSON.stringify(payload);
    }
    return request;
}

export async function apiJson(url, options) {
    const response = await fetch(url, options);
    if (!response.ok) throw buildError(response, await readErrorMessage(response));
    if (response.status === 204) return null;
    return response.json();
}

export async function apiText(url, options) {
    const response = await fetch(url, options);
    if (!response.ok) throw buildError(response, await readErrorMessage(response));
    return response.text();
}

export async function apiResponse(url, options) {
    return fetch(url, options);
}

export async function tryFetchJson(url, options) {
    try {
        const response = await fetch(url, options);
        if (!response.ok) return null;
        return await response.json();
    } catch {
        return null;
    }
}
