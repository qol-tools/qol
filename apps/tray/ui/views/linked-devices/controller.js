import { loadSnapshot, failureMessage } from './model.js';

export function createPeerController({ request, inspectInvitation, readCatalog, publish }) {
    let active = false;
    let sequence = 0;
    let controller;
    let uncertain = false;
    let flight = null;
    let state = { snapshot: null, catalog: null, busy: false, recoveryRequired: false, message: '', invitation: null, source: null };
    const update = patch => {
        state = { ...state, ...patch, recoveryRequired: uncertain };
        publish(state);
    };

    async function run(work, mutation = false) {
        if (!active || flight || (mutation && uncertain)) return;
        const ticket = ++sequence;
        controller = new AbortController();
        const signal = controller.signal;
        const current = () => active && ticket === sequence && !signal.aborted;
        const commit = patch => { if (current()) update(patch); };
        flight = { mutation };
        update({ busy: true });
        try {
            const result = await work(signal, commit);
            if (!current()) return;
            update({ message: uncertain ? failureMessage({ kind: 'outcome_unknown' }) : '' });
            return result;
        } catch (error) {
            uncertain = uncertain || mutation;
            if (!current()) return;
            update({ snapshot: null, message: failureMessage(error) });
        } finally {
            flight = null;
            update({ busy: false });
        }
    }

    async function read(signal, commit) {
        const snapshot = await loadSnapshot(query => request(query, signal));
        if (signal.aborted) return;
        commit({ snapshot });
        try {
            const catalog = await readCatalog(signal);
            commit({ catalog });
        } catch {
            commit({ catalog: null });
        }
    }

    const refresh = (explicit = true) => {
        if (!explicit && uncertain) return;
        return run(async (signal, commit) => {
            commit({ catalog: null });
            await read(signal, commit);
            if (signal.aborted) return;
            if (explicit) uncertain = false;
            commit({});
        });
    };

    return {
        setActive(value) {
            if (flight?.mutation) uncertain = true;
            ++sequence;
            controller?.abort();
            active = value;
            update({ snapshot: null, catalog: null, busy: Boolean(flight), invitation: null, source: null,
                message: uncertain ? failureMessage({ kind: 'outcome_unknown' }) : '' });
        },
        refresh: () => refresh(true),
        poll: () => refresh(false),
        mutate(query) {
            const frozen = structuredClone(query);
            return run(async (signal, commit) => {
                commit({ snapshot: null, catalog: null });
                const result = await request(frozen, signal);
                if (signal.aborted) return;
                if (result.result === 'invitation') commit({ invitation: result });
                if (frozen.request?.action === 'cancel_invitation') commit({ invitation: null });
                await read(signal, commit);
                return result;
            }, true);
        },
        inspect(document) {
            return run(async (signal, commit) => {
                commit({ source: null });
                const info = await inspectInvitation(document, signal);
                commit({ source: { document, ...info } });
            });
        },
    };
}
