import { useState, useEffect, useRef, useCallback } from 'preact/hooks';
import { buildFieldPathIndex, getFieldValue, setFieldValue, getFieldValueById } from '../../auto-config/normalized-config.js';
import { configFromForm, getDisplaySections, ownedConfigKeys } from '../../auto-config/form-model.js';

const SAVE_DEBOUNCE_MS = 400;
const preloadedForms = new Map();

export function preloadConfigForm(pluginId, data) { preloadedForms.set(pluginId, data); }

export function usePluginConfig(pluginId) {
    const [form, setForm] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    const [renderTick, setRenderTick] = useState(0);
    const configRef = useRef(null);
    const fieldPathsRef = useRef(null);
    const saveTimerRef = useRef(null);

    useEffect(() => {
        return startPluginConfigSessionLoad(
            pluginId,
            (session) => {
                applyConfigSession(session, setForm, configRef, fieldPathsRef, setError);
                setLoading(false);
                setRenderTick(t => t + 1);
            },
            (err) => {
                applyConfigSession({ ...emptyConfigSession(), error: errorMessage(err) }, setForm, configRef, fieldPathsRef, setError);
                setLoading(false);
                setRenderTick(t => t + 1);
            },
        );
    }, [pluginId]);

    const state = {
        config: configRef.current,
        fieldPaths: fieldPathsRef.current,
    };

    const save = useCallback(() => {
        clearTimeout(saveTimerRef.current);
        saveTimerRef.current = setTimeout(
            () => persistConfig(pluginId, configRef.current, form, extraKeysRef.current),
            SAVE_DEBOUNCE_MS,
        );
    }, [pluginId, form]);

    const saveNow = useCallback(() => {
        clearTimeout(saveTimerRef.current);
        return persistConfig(pluginId, configRef.current, form, extraKeysRef.current);
    }, [pluginId, form]);

    const bumpRender = useCallback(() => setRenderTick(t => t + 1), []);

    const sections = form ? getDisplaySections(form) : [];

    const extraKeysRef = useRef(new Set());

    const setConfigKey = useCallback((key, value) => {
        if (state.config) state.config[key] = value;
        extraKeysRef.current.add(key);
    }, [state]);

    return {
        loading,
        error,
        form,
        sections,
        state,
        save,
        saveNow,
        bumpRender,
        renderTick,
        runtime: form?.runtime || null,
        daemonPort: form?.daemonPort || null,
        getFieldValue: (field) => getFieldValue(state, field),
        setFieldValue: (field, value) => setFieldValue(state, field, value),
        setConfigKey,
        getFieldValueById: (fieldId) => getFieldValueById(state, fieldId),
    };
}

export function startPluginConfigSessionLoad(pluginId, applySession, applyError, loadSession = loadPluginConfigSession) {
    let cancelled = false;
    loadSession(pluginId)
        .then(session => {
            if (cancelled) return;
            applySession(session);
        })
        .catch(err => {
            if (cancelled) return;
            applyError(err);
        });
    return () => { cancelled = true; };
}

export async function loadPluginConfigSession(pluginId) {
    const result = await fetchConfigData(pluginId);
    return resolveConfigSession(result);
}

async function fetchConfigData(pluginId) {
    const preloaded = preloadedForms.get(pluginId);
    if (preloaded) { preloadedForms.delete(pluginId); return { type: 'form', data: preloaded, pluginId }; }
    const formResponse = await fetch(`/api/plugins/${pluginId}/config-form`);
    if (formResponse.ok) return { type: 'form', data: await tryParseJson(formResponse), pluginId };
    if (formResponse.status !== 404) throw new Error(await formResponse.text());
    const configResponse = await fetch(`/api/plugins/${pluginId}/config`);
    if (!configResponse.ok) return { type: 'none' };
    return { type: 'raw', data: await tryParseJson(configResponse) };
}

async function tryParseJson(response) {
    const text = await response.text();
    if (!text) return null;
    return JSON.parse(text);
}

async function fetchExistingConfig(pluginId) {
    try {
        const res = await fetch(`/api/plugins/${pluginId}/config`);
        if (!res.ok) return {};
        const data = await tryParseJson(res);
        return data && typeof data === 'object' ? data : {};
    } catch { return {}; }
}

async function resolveConfigSession(result) {
    if (result.type === 'none') {
        return { ...emptyConfigSession(), error: 'No configuration found for this plugin.' };
    }
    if (result.type === 'raw') {
        return { ...emptyConfigSession(), config: result.data };
    }
    const existingConfig = await fetchExistingConfig(result.pluginId);
    return {
        form: result.data,
        config: configFromForm(result.data, existingConfig),
        fieldPaths: buildFieldPathIndex(result.data),
        error: null,
    };
}

function emptyConfigSession() {
    return { form: null, config: null, fieldPaths: null, error: null };
}

function applyConfigSession(session, setForm, configRef, fieldPathsRef, setError) {
    configRef.current = session.config;
    fieldPathsRef.current = session.fieldPaths;
    setForm(session.form);
    setError(session.error);
}

function errorMessage(err) {
    return err instanceof Error ? err.message : String(err);
}

async function persistConfig(pluginId, config, form, extraKeys) {
    try {
        const payload = form ? filterOwnedKeys(config, form, extraKeys) : config;
        const response = await fetch(`/api/plugins/${pluginId}/config`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload, null, 2),
        });
        if (!response.ok) throw new Error(await response.text());
    } catch (err) {
        console.error('Save failed', err);
    }
}

function filterOwnedKeys(config, form, extraKeys) {
    if (!config || typeof config !== 'object') return config;
    const keys = ownedConfigKeys(form);
    if (extraKeys) for (const k of extraKeys) keys.add(k);
    const filtered = {};
    for (const key of keys) {
        if (key in config) filtered[key] = config[key];
    }
    return filtered;
}
