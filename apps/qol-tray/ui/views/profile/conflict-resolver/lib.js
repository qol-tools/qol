export function summarize(picks) {
    let keptMine = 0;
    let tookRemote = 0;
    for (const pick of picks) {
        if (pick === 'mine') keptMine += 1;
        if (pick === 'remote') tookRemote += 1;
    }
    return { keptMine, tookRemote };
}

export function allPicked(picks) {
    if (!picks.length) return false;
    for (const pick of picks) {
        if (pick !== 'mine' && pick !== 'remote') return false;
    }
    return true;
}

export function formatValue(value) {
    if (value === null || value === undefined) return 'null';
    if (typeof value === 'string') return JSON.stringify(value);
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    try {
        return JSON.stringify(value);
    } catch (_) {
        return String(value);
    }
}

export function formatValueShort(value, max = 44) {
    if (value === undefined) return '—';
    const text = formatValue(value);
    return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

export function fieldDiff(local, remote) {
    const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
    if (!isObject(local) || !isObject(remote)) return null;
    const keys = [...new Set([...Object.keys(local), ...Object.keys(remote)])];
    const rows = [];
    for (const key of keys) {
        if (JSON.stringify(local[key]) === JSON.stringify(remote[key])) continue;
        rows.push({ key, mine: local[key], remote: remote[key] });
    }
    return rows.length ? rows : null;
}

export function relativeTime(isoString, now = Date.now()) {
    if (!isoString) return 'unknown time';
    const ts = Date.parse(isoString);
    if (!Number.isFinite(ts)) return 'unknown time';
    const deltaMs = now - ts;
    if (deltaMs < 0) return 'in the future';
    const seconds = Math.round(deltaMs / 1000);
    if (seconds < 60) return 'just now';
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `edited ${minutes} minute${minutes === 1 ? '' : 's'} ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `edited ${hours} hour${hours === 1 ? '' : 's'} ago`;
    const days = Math.round(hours / 24);
    if (days < 30) return `edited ${days} day${days === 1 ? '' : 's'} ago`;
    const months = Math.round(days / 30);
    if (months < 12) return `edited ${months} month${months === 1 ? '' : 's'} ago`;
    const years = Math.round(months / 12);
    return `edited ${years} year${years === 1 ? '' : 's'} ago`;
}

export function buildPicks(conflicts) {
    return conflicts.map(() => null);
}

export function nextIndex(index, total, dir) {
    if (!total) return 0;
    return Math.max(0, Math.min(total - 1, index + dir));
}

export function toChoices(conflicts, picks) {
    const choices = [];
    for (let i = 0; i < conflicts.length; i += 1) {
        const pick = picks[i];
        if (pick !== 'mine' && pick !== 'remote') continue;
        const conflict = conflicts[i];
        choices.push({
            file: conflict.file,
            key_path: conflict.key_path,
            side: pick,
        });
    }
    return choices;
}

export function conflictKey(conflict) {
    return `${conflict.file}::${conflict.key_path}`;
}

export function dottedKeyParts(keyPath) {
    if (!keyPath) return [];
    return keyPath.split('.');
}

export function leafKey(keyPath) {
    const parts = dottedKeyParts(keyPath);
    return parts.length ? parts[parts.length - 1] : keyPath || '';
}
