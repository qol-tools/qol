export function resolveInitialBranch({ persisted, serverActive, branches }) {
    const list = Array.isArray(branches) ? branches : [];
    if (serverActive && list.includes(serverActive)) return serverActive;
    if (persisted && list.includes(persisted)) return persisted;
    return null;
}
