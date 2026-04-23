export function resolveInitialWorktree({ persisted, serverActive, worktrees }) {
    const list = Array.isArray(worktrees) ? worktrees : [];
    if (serverActive && list.some(w => w.path === serverActive)) {
        return serverActive;
    }
    if (!persisted) return null;
    if (list.some(w => w.path === persisted)) return persisted;
    const byParent = list.find(w => parentDir(w.path) === persisted);
    if (byParent) return byParent.path;
    return null;
}

export function parentDir(path) {
    if (!path) return null;
    const separator = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    if (separator <= 0) return null;
    return path.slice(0, separator);
}
