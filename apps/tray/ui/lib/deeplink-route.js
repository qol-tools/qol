export function parseDeepRoute(raw) {
    const s = String(raw ?? '').replace(/^#/, '');
    const [pathPart, queryPart = ''] = s.split('?');
    const segs = pathPart.split('/').filter(Boolean);
    const params = {};
    for (const [k, v] of new URLSearchParams(queryPart)) params[k] = v;
    return { page: segs[0] || null, action: segs[1] || null, params };
}
