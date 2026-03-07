function isSeparator(c) {
    return c === ' ' || c === '-' || c === '_' || c === '/';
}

function isBoundary(chars, idx) {
    if (idx === 0) return true;
    const prev = chars[idx - 1];
    const curr = chars[idx];
    return isSeparator(prev) || (curr === curr.toUpperCase() && curr !== curr.toLowerCase() && prev === prev.toLowerCase());
}

function isFullyContiguous(positions) {
    if (positions.length <= 1) return false;
    for (let i = 1; i < positions.length; i++) {
        if (positions[i] !== positions[i - 1] + 1) return false;
    }
    return true;
}

function computeScore(positions, candidate, queryOrig) {
    let score = 0;
    for (let i = 0; i < positions.length; i++) {
        const pos = positions[i];
        let gap;
        if (i === 0) {
            gap = (pos > 0 && isBoundary(candidate, pos)) ? Math.min(pos, 1) : pos;
        } else {
            gap = pos - positions[i - 1] - 1;
        }
        score += gap * 3;
        if (i > 0 && gap === 0) score -= 4;
        if (isBoundary(candidate, pos)) score -= 6;
        if (pos === 0) score -= 8;
        if (i < queryOrig.length && candidate[pos] === queryOrig[i]) score -= 2;
    }
    if (queryOrig.length > 1 && isFullyContiguous(positions)) {
        score -= 12 * queryOrig.length;
    }
    return score;
}

function scorePass(queryLower, queryOrig, candidate, candidateLower, preferBoundary) {
    const positions = [];
    let start = 0;
    for (let qi = 0; qi < queryLower.length; qi++) {
        const qc = queryLower[qi];
        let pos = -1;
        if (preferBoundary) {
            let first = -1;
            for (let i = start; i < candidateLower.length; i++) {
                if (candidateLower[i] !== qc) continue;
                if (first === -1) first = i;
                if (isBoundary(candidate, i)) { pos = i; break; }
            }
            if (pos === -1) pos = first;
        } else {
            pos = candidateLower.indexOf(qc, start);
        }
        if (pos === -1) return null;
        positions.push(pos);
        start = pos + 1;
    }
    return { score: computeScore(positions, candidate, queryOrig), positions };
}

function scoreContiguousPass(queryLower, queryOrig, candidate, candidateLower) {
    if (queryLower.length > candidateLower.length) return null;
    let best = null;
    const end = candidateLower.length - queryLower.length;
    for (let s = 0; s <= end; s++) {
        let match = true;
        for (let i = 0; i < queryLower.length; i++) {
            if (candidateLower[s + i] !== queryLower[i]) { match = false; break; }
        }
        if (!match) continue;
        const positions = [];
        for (let i = 0; i < queryLower.length; i++) positions.push(s + i);
        const m = { score: computeScore(positions, candidate, queryOrig), positions };
        if (!best || m.score < best.score) best = m;
    }
    return best;
}

function scoreWordMatchPass(queryLower, queryOrig, candidate, candidateLower) {
    if (queryLower.length > candidateLower.length) return null;
    let best = null;
    const limit = candidateLower.length - queryLower.length;
    for (let s = 0; s <= limit; s++) {
        let match = true;
        for (let i = 0; i < queryLower.length; i++) {
            if (candidateLower[s + i] !== queryLower[i]) { match = false; break; }
        }
        if (!match) continue;
        const e = s + queryLower.length;
        const atStart = s === 0 || isSeparator(candidate[s - 1]);
        const atEnd = e === candidateLower.length || isSeparator(candidate[e]);
        if (!atStart || !atEnd) continue;
        const positions = [];
        for (let i = 0; i < queryLower.length; i++) positions.push(s + i);
        const wordBonus = -10 * queryLower.length;
        const m = { score: computeScore(positions, candidate, queryOrig) + wordBonus, positions };
        if (!best || m.score < best.score) best = m;
    }
    return best;
}

export function fuzzyMatch(query, candidate) {
    if (!query) return { score: 0, positions: [] };
    const queryOrig = [...query];
    const queryLower = [...query.toLowerCase()];
    const cOrig = [...candidate];
    const cLower = [...candidate.toLowerCase()];

    const passes = [
        scorePass(queryLower, queryOrig, cOrig, cLower, false),
        scorePass(queryLower, queryOrig, cOrig, cLower, true),
        scoreContiguousPass(queryLower, queryOrig, cOrig, cLower),
        scoreWordMatchPass(queryLower, queryOrig, cOrig, cLower),
    ];

    let best = null;
    for (const m of passes) {
        if (m && (!best || m.score < best.score)) best = m;
    }
    return best;
}
