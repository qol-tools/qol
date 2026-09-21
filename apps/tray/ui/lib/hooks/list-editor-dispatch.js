export function composeListEditorHandler({ modalRef, onModal, onList, preIntercept, listIntercept }) {
    return (e) => {
        if (preIntercept?.(e)) return;
        if (modalRef.current) { onModal(e); return; }
        if (listIntercept?.(e)) return;
        onList(e);
    };
}
