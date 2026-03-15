import { useState, useEffect, useRef, useCallback } from 'preact/hooks';
import { buildFieldPathIndex, getFieldValue, setFieldValue, getFieldValueById } from '../../auto-config/normalized-config.js';
import { configFromForm, getDisplaySections } from '../../auto-config/form-model.js';

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

    useEffect(() => { loadConfig(pluginId, setForm, configRef, fieldPathsRef, setLoading, setError); }, [pluginId]);

    const state = {
        config: configRef.current,
        fieldPaths: fieldPathsRef.current,
    };

    const save = useCallback(() => {
        clearTimeout(saveTimerRef.current);
        saveTimerRef.current = setTimeout(() => persistConfig(pluginId, configRef.current), SAVE_DEBOUNCE_MS);
    }, [pluginId]);

    const bumpRender = useCallback(() => setRenderTick(t => t + 1), []);

    const sections = form ? getDisplaySections(form) : [];

    return {
        loading,
        error,
        form,
        sections,
        state,
        save,
        bumpRender,
        renderTick,
        getFieldValue: (field) => getFieldValue(state, field),
        setFieldValue: (field, value) => setFieldValue(state, field, value),
        getFieldValueById: (fieldId) => getFieldValueById(state, fieldId),
    };
}

async function loadConfig(pluginId, setForm, configRef, fieldPathsRef, setLoading, setError) {
    try {
        const result = await fetchConfigData(pluginId);
        applyConfigResult(result, setForm, configRef, fieldPathsRef, setError);
        setLoading(false);
    } catch (err) {
        setError(err.message);
        setLoading(false);
    }
}

async function fetchConfigData(pluginId) {
    const preloaded = preloadedForms.get(pluginId);
    if (preloaded) { preloadedForms.delete(pluginId); return { type: 'form', data: preloaded }; }
    const formResponse = await fetch(`/api/plugins/${pluginId}/config-form`);
    if (formResponse.ok) return { type: 'form', data: await tryParseJson(formResponse) };
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

function applyConfigResult(result, setForm, configRef, fieldPathsRef, setError) {
    if (result.type === 'none') {
        setError('No configuration found for this plugin.');
        return;
    }
    if (result.type === 'raw') {
        configRef.current = result.data;
        return;
    }
    fieldPathsRef.current = buildFieldPathIndex(result.data);
    configRef.current = configFromForm(result.data);
    setForm(result.data);
}

async function persistConfig(pluginId, config) {
    try {
        const response = await fetch(`/api/plugins/${pluginId}/config`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(config, null, 2),
        });
        if (!response.ok) throw new Error(await response.text());
    } catch (err) {
        console.error('Save failed', err);
    }
}
