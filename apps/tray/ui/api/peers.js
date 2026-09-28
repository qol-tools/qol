import './client.js';

export async function peerRequest(request, signal) {
    return post('/api/peers/admin', request, signal, isMutation(request));
}

export async function invitationInfo(document, signal) {
    return post('/api/peers/invitation', document, signal, false);
}

export async function peerCatalog(signal) {
    try {
        const response = await fetch('/api/peers/catalog', {
            signal, cache: 'no-store', qolSuppressErrorToast: true,
        });
        if (!response.ok) throw new Error('Operation catalog unavailable');
        const operations = await response.json();
        if (!Array.isArray(operations)) throw new Error('Invalid operation catalog');
        return operations;
    } catch {
        throw { kind: 'transport' };
    }
}

export function isMutation(request) {
    if (request.operation === 'enrollment') {
        return !['pending', 'outbound', 'attempt'].includes(request.request.action);
    }
    return !['status', 'network', 'sessions', 'peers', 'grants', 'tombstones'].includes(request.operation);
}

async function post(route, body, signal, mutation) {
    try {
        const response = await fetch(route, {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body), signal, cache: 'no-store', qolSuppressErrorToast: true,
        });
        if ([400, 413].includes(response.status)) throw { peerFailure: true, kind: 'invalid_request' };
        if ([401, 403].includes(response.status)) throw { peerFailure: true, kind: 'access_denied' };
        const value = await response.json();
        if (value.kind) throw { peerFailure: true, ...value };
        if (!response.ok) throw new Error('Peer request failed');
        if (value.result === 'error') throw { peerFailure: true, kind: 'authority', error: value.error };
        return value;
    } catch (error) {
        if (error?.peerFailure) throw error;
        throw { kind: mutation ? 'outcome_unknown' : 'transport' };
    }
}
