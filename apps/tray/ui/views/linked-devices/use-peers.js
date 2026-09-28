import { useEffect, useState } from 'preact/hooks';
import { peerRequest, invitationInfo, peerCatalog } from '../../api/peers.js';
import { createPeerController } from './controller.js';

let publish = () => {};
const peer = createPeerController({ request: peerRequest, inspectInvitation: invitationInfo,
    readCatalog: peerCatalog, publish: value => publish(value) });

export function usePeers(active) {
    const [state, setState] = useState({ snapshot: null, catalog: null, busy: false, recoveryRequired: false, message: '', invitation: null, source: null });
    useEffect(() => {
        publish = setState;
        peer.setActive(active);
        if (active) peer.poll();
        const timer = active ? setInterval(() => {
            if (!document.hidden) peer.poll();
        }, 3000) : null;
        return () => {
            peer.setActive(false);
            publish = () => {};
            if (timer) clearInterval(timer);
        };
    }, [active]);
    return { ...state, refresh: peer.refresh, mutate: peer.mutate, inspect: peer.inspect };
}
