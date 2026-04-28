// View keyboard handler resolution.
//
// During a dive into an editor sub-page (hotkeys-editor, shortcuts-editor,
// task-runner-editor), `activeViewId` stays on the parent view because dive
// doesn't switch top-level views — only the navigation anchor moves. The
// resolver therefore tries the anchor's pageId first (e.g. 'hotkeys-editor'),
// then the active view id, then the parent of an editor sub-page. This lets
// editor sub-pages register their own keyboard handler while still falling
// back to the parent's handler when the sub-page hasn't registered.
//
// The fallback helper is intentionally narrow: it only handles the three
// `*-editor` sub-pages because those register their own viewId via
// `DiveEditorSubPage`. Other dive sub-pages (logs-detail, profile-backup-detail,
// dev-log-filters, dev-plugin-actions, plugins-uninstall-confirm,
// plugins-actions, task-runner-test-runner) do not register their own
// keyboard handler — they rely on `activeViewId` staying at the parent during
// the dive, so the direct lookup of `activeViewId` already resolves to the
// parent's handler. If a non-editor sub-page ever needs its own keyboard
// handler with parent fallback, register it directly under its own viewId.

const EDITOR_SUFFIX = '-editor';

export function editorParentViewId(viewId) {
    if (!viewId?.endsWith(EDITOR_SUFFIX)) return null;
    return viewId.slice(0, -EDITOR_SUFFIX.length);
}

export function resolveViewKeyboard(viewId, getViewKeyboard, anchorPageId = null) {
    if (anchorPageId && anchorPageId !== viewId) {
        const anchored = getViewKeyboard(anchorPageId);
        if (anchored) return anchored;
        const anchorParent = editorParentViewId(anchorPageId);
        if (anchorParent) {
            const fromAnchorParent = getViewKeyboard(anchorParent);
            if (fromAnchorParent) return fromAnchorParent;
        }
    }
    const direct = getViewKeyboard(viewId);
    if (direct) return direct;
    const parentId = editorParentViewId(viewId);
    return parentId ? getViewKeyboard(parentId) : null;
}
