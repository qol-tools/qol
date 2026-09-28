import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageShell } from '../../components/PageShell.js';
import { Button } from '../../lib/components/Button.js';
import { ConfirmButton } from '../../lib/components/ConfirmButton.js';
import { TextInput } from '../../lib/components/TextInput.js';
import { Surface } from '../../lib/components/Surface.js';
import { toast } from '../../lib/toast.js';
import { usePeers } from './use-peers.js';
import { enrollment, stamp, viewKey, operationId, permissionChanges, failureMessage, phoneRemoval, phonesMustPairAgain } from './model.js';

function Field({ label, value, onInput, maxLength, disabled }) {
    return html`<${Surface} className="form-group" onActivate=${event => event.currentTarget.querySelector('input')?.focus()}>
        <label>${label}<${TextInput} value=${value} maxLength=${maxLength} disabled=${disabled}
            autoComplete="off" spellCheck=${false} onInput=${event => onInput(event.currentTarget.value)} /></label>
    <//>`;
}

export function LinkedComputersView({ active }) {
    const ctrl = usePeers(active);
    const [name, setName] = useState('');
    const [document, setDocument] = useState('');
    useEffect(() => { if (!active) setDocument(''); }, [active]);
    const { snapshot, busy, mutate, source } = ctrl;
    const authority = snapshot?.status.authority;
    const expected = authority ? stamp(authority) : null;
    const send = (operation, fields = {}) => mutate({ operation, ...fields });
    const enroll = (action, fields = {}) => mutate(enrollment(action, { expected, ...fields }));
    const usable = authority?.status === 'ready' && snapshot?.status.lifecycle.state === 'active';
    const disabled = busy || ctrl.recoveryRequired || !usable;
    return html`<${PageShell} subtitle="Linked computers" frameId="linked-computers-settings">
        <p role="status">${ctrl.message || (busy ? 'Reading or updating core…' : snapshot?.status.lifecycle.state || 'State unavailable')}</p>
        ${snapshot?.status.lifecycle.error && html`<p role="status">${failureMessage({ kind: 'authority', error: snapshot.status.lifecycle.error })}</p>`}
        <${Button} disabled=${busy} onActivate=${ctrl.refresh}>Refresh<//>
        <${Field} label="Computer name" value=${name} maxLength=${128} disabled=${busy} onInput=${setName} />
        ${snapshot && !authority && ['inactive', 'unavailable'].includes(snapshot.status.lifecycle.state) && html`
            <p>Choose how long links belong to this computer. Persistent links require platform support.</p>
            <${Button} disabled=${busy || ctrl.recoveryRequired || !name.trim()} onActivate=${() => send('start_session', { name })}>Use this session<//>
            <${ConfirmButton} disabled=${busy || ctrl.recoveryRequired || !name.trim()} confirmWith="persist" onActivate=${() => send('create_persistent', { name })}>Create persistent links<//>
            <${Button} disabled=${busy || ctrl.recoveryRequired} onActivate=${() => send('open_persistent')}>Open existing persistent links<//>`}
        ${authority && html`<div key=${viewKey(authority)}>
            <p>${authority.name} · ${authority.lifetime} · ${authority.status}</p>
            <p style="overflow-wrap:anywhere">${authority.peer_id}</p>
            <${Button} disabled=${disabled || !name.trim()} onActivate=${() => send('rename', { expected, name })}>Rename<//>
            <${ConfirmButton} disabled=${disabled} confirmWith="stop" onActivate=${() => send('stop', { expected })}>Stop linking<//>
            <p>Stop before changing lifetime. New links receive no operation permissions.</p>
            <${Button} disabled=${disabled} onActivate=${() => enroll('create_invitation', { addresses: [] })}>Create invitation<//>
            ${ctrl.invitation && ctrl.invitation.authority.authority_id === authority.peer_id
                && ctrl.invitation.authority.activation_id === authority.activation_id && html`
                <p>Invitation ready. Copy only to the intended computer.</p>
                <${Button} disabled=${busy} onActivate=${async () => {
                    try { await navigator.clipboard.writeText(ctrl.invitation.document); }
                    catch { toast('error', 'Could not copy invitation'); }
                }}>Copy invitation<//>
                <${Button} disabled=${disabled} onActivate=${() => enroll('cancel_invitation', { invitation: ctrl.invitation.invitation })}>Cancel invitation<//>`}
            <${Field} label="Invitation code" value=${document} maxLength=${4096} disabled=${busy} onInput=${setDocument} />
            <${Button} disabled=${disabled || !document} onActivate=${() => { ctrl.inspect(document); setDocument(''); }}>Read invitation<//>
            ${source && html`<p style="overflow-wrap:anywhere">Invitation from ${source.peer}</p>
                <${Button} disabled=${disabled || snapshot.outbound.some(item => item.key.invitation === source.invitation)}
                    onActivate=${() => enroll('prepare', { document: source.document })}>Prepare link<//>`}
            <h3>Pairing requests</h3>
            ${snapshot.pending.map(item => html`<div key=${item.key.transaction}>
                <p style="overflow-wrap:anywhere">${item.name} · ${item.key.peer} · ${item.remote_lifetime}</p>
                <${ConfirmButton} disabled=${disabled} confirmWith="approve" onActivate=${() => enroll('approve', { key: item.key })}>Approve<//>
                <${Button} disabled=${disabled} onActivate=${() => enroll('reject', { key: item.key })}>Reject<//>
            </div>`)}
            <h3>Outgoing links</h3>
            ${snapshot.outbound.map(item => html`<${Outbound} key=${item.key.transaction} item=${item} attempt=${snapshot.attempts[item.key.transaction]} source=${source} expected=${expected} disabled=${disabled} mutate=${mutate} />`)}
            <h3>Linked computers</h3>
            ${snapshot.peers.map(peer => html`<${Peer} key=${peer.peer_id} peer=${peer} snapshot=${snapshot} catalog=${ctrl.catalog} expected=${expected} disabled=${disabled} mutate=${mutate} />`)}
            <${Phones} snapshot=${snapshot} disabled=${disabled} mutate=${mutate} />
            ${ctrl.catalog === null && html`<p>Permission catalog unavailable. Refresh to retry. Existing permissions are retained.</p>`}
            ${ctrl.catalog?.length === 0 && html`<p>No available peer operations. Existing permissions are retained.</p>`}
        </div>`}
    <//>`;
}

function Outbound({ item, attempt, source, expected, disabled, mutate }) {
    const matching = source?.invitation === item.key.invitation && source?.peer === item.key.peer;
    const send = (action, fields = {}) => mutate(enrollment(action, { expected, transaction: item.key.transaction, ...fields }));
    return html`<div>
        <p style="overflow-wrap:anywhere">${item.key.peer} · ${item.state.kind} · ${item.key.transaction}</p>
        <p>${attempt?.state || 'Unavailable'}</p>
        ${item.state.kind === 'pending' && html`
            <${Button} disabled=${disabled || !matching} onActivate=${() => send('redeem', { document: source.document })}>Send prepared request<//>
            <${Button} disabled=${disabled || !matching} onActivate=${() => send('recover', { endpoints: source.endpoints })}>Recover original transaction<//>
            <${ConfirmButton} disabled=${disabled} confirmWith="abandon" onActivate=${() => send('abandon')}>Abandon<//>`}
        ${item.state.kind === 'abandoned' && html`<${Button} disabled=${disabled} onActivate=${() => send('resume')}>Resume original transaction<//>`}
        ${item.state.kind !== 'committed' && !matching && html`<p>Paste the original invitation to send or recover this transaction.</p>`}
    </div>`;
}

function Phones({ snapshot, disabled, mutate }) {
    const status = snapshot.pointz;
    if (!status || status.plugin === 'absent') return null;
    if (status.plugin === 'legacy') return html`<h3>Phones</h3><p>PointZ keeps its own phone pairing until it is updated.</p>`;
    const listening = !['port_busy', 'failed'].includes(status.transport.state);
    return html`<h3>Phones</h3>
        ${phonesMustPairAgain(status.migration) && html`<p>Some phones must pair again: not every PointZ pairing could move into linked computers.</p>`}
        ${!listening && html`<p>PointZ is not listening. Another program may be using its network ports.</p>`}
        ${snapshot.phones.length === 0 && html`<p>No paired phones. Pair a phone from PointZ settings.</p>`}
        ${snapshot.phones.map(phone => html`<div key=${phone.device_id}>
            <p style="overflow-wrap:anywhere">${phone.name}</p>
            <${ConfirmButton} disabled=${disabled} confirmWith="remove-phone" onActivate=${() => mutate(phoneRemoval(snapshot, phone))}>Remove phone<//>
        </div>`)}`;
}

function Peer({ peer, snapshot, catalog, expected, disabled, mutate }) {
    const changes = permissionChanges(snapshot, peer, catalog);
    const connected = snapshot.sessions.some(session => session.peer_id === peer.peer_id);
    return html`<div>
        <p style="overflow-wrap:anywhere">${peer.name} · ${peer.peer_id} · ${connected ? 'Authenticated connection' : 'Not connected'}</p>
        <${ConfirmButton} disabled=${disabled} confirmWith="revoke" onActivate=${() => mutate({ operation: 'revoke', expected, peer_id: peer.peer_id })}>Revoke link<//>
        ${changes.map(change => html`<div key=${`${change.action}:${operationId(change.key)}`}>
            <p style="overflow-wrap:anywhere">${change.label}</p>
            <${ConfirmButton} disabled=${disabled} confirmWith=${change.action} onActivate=${() => mutate(change.request)}>${change.action === 'add' ? 'Add permission' : 'Remove permission'}<//>
        </div>`)}
    </div>`;
}
