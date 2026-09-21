import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import { toast } from '../../../lib/toast.js';
import { fetchConflicts, resolveConflicts } from '../actions.js';
import { allPicked, buildPicks, nextIndex, summarize, toChoices } from './lib.js';

export function useResolver({ active, onApplied }) {
    const [conflicts, setConflicts] = useState([]);
    const [picks, setPicks] = useState([]);
    const [index, setIndex] = useState(0);
    const [phase, setPhase] = useState('loading');
    const [applying, setApplying] = useState(false);
    const [showConfirm, setShowConfirm] = useState(false);

    useEffect(() => {
        if (!active) return undefined;
        let cancelled = false;
        setPhase('loading');
        setShowConfirm(false);
        fetchConflicts()
            .then(list => {
                if (cancelled) return;
                const next = Array.isArray(list) ? list : [];
                setConflicts(next);
                setPicks(buildPicks(next));
                setIndex(0);
                setPhase(next.length ? 'ready' : 'empty');
            })
            .catch(error => {
                if (cancelled) return;
                setPhase('error');
                toast('error', `Failed to load conflicts: ${error.message}`);
            });
        return () => { cancelled = true; };
    }, [active]);

    const total = conflicts.length;
    const current = conflicts[index] || null;
    const pick = useCallback(side => {
        setPicks(prev => {
            if (!prev.length) return prev;
            const next = prev.slice();
            next[index] = side;
            return next;
        });
    }, [index]);

    const moveTo = useCallback(dir => {
        setIndex(prev => nextIndex(prev, total, dir));
    }, [total]);

    const next = useCallback(() => moveTo(1), [moveTo]);
    const prev = useCallback(() => moveTo(-1), [moveTo]);

    const summary = useMemo(() => summarize(picks), [picks]);
    const ready = useMemo(() => allPicked(picks), [picks]);

    const openConfirm = useCallback(() => {
        if (!ready) return;
        setShowConfirm(true);
    }, [ready]);

    const backToSteps = useCallback(() => setShowConfirm(false), []);

    const apply = useCallback(async () => {
        if (!ready || applying) return;
        setApplying(true);
        try {
            const result = await resolveConflicts(toChoices(conflicts, picks));
            onApplied?.(result);
        } catch (error) {
            toast('error', `Failed to resolve conflicts: ${error.message}`);
        }
        setApplying(false);
    }, [applying, conflicts, onApplied, picks, ready]);

    return {
        applying,
        apply,
        backToSteps,
        conflicts,
        current,
        index,
        moveTo,
        next,
        openConfirm,
        phase,
        pick,
        picks,
        prev,
        ready,
        showConfirm,
        summary,
        total,
    };
}
