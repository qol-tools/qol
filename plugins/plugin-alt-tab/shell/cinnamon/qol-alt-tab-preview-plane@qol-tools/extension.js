const Gio = imports.gi.Gio;
const GLib = imports.gi.GLib;
const Meta = imports.gi.Meta;
const St = imports.gi.St;

const Main = imports.ui.main;
const Mainloop = imports.mainloop;
const WindowUtils = imports.misc.windowUtils;

const DBUS_IFACE = `
<node>
  <interface name="org.qol.AltTabPreviewPlane">
    <method name="Ping">
      <arg type="s" direction="out" name="result" />
    </method>
    <method name="Show">
      <arg type="s" direction="in" name="payload_json" />
      <arg type="s" direction="out" name="result" />
    </method>
    <method name="ShowDemo">
      <arg type="s" direction="out" name="result" />
    </method>
    <method name="Hide">
      <arg type="s" direction="out" name="result" />
    </method>
  </interface>
</node>`;

const OBJECT_PATH = "/org/qol/AltTabPreviewPlane";
const DEFAULT_TTL_MS = 4500;

let plane = null;

function init(metadata) {
    plane = new PreviewPlane(metadata.uuid);
}

function enable() {
    plane.enable();
}

function disable() {
    plane.disable();
    plane = null;
}

class PreviewPlane {
    constructor(uuid) {
        this._uuid = uuid;
        this._dbusImpl = null;
        this._group = null;
        this._hideTimeout = 0;
        this._unredirectDisabled = false;
    }

    enable() {
        this._dbusImpl = Gio.DBusExportedObject.wrapJSObject(DBUS_IFACE, this);
        this._dbusImpl.export(Gio.DBus.session, OBJECT_PATH);
        global.log(`[qol-alt-tab-preview-plane] enabled path=${OBJECT_PATH}`);
    }

    disable() {
        this._clear("disable");
        if (this._dbusImpl) {
            this._dbusImpl.unexport();
            this._dbusImpl = null;
        }
        global.log("[qol-alt-tab-preview-plane] disabled");
    }

    Ping() {
        return JSON.stringify({
            ok: true,
            uuid: this._uuid,
            stage: {
                width: Math.round(global.stage.width),
                height: Math.round(global.stage.height)
            }
        });
    }

    Show(payloadJson) {
        const startedUs = GLib.get_monotonic_time();
        let payload;
        try {
            payload = JSON.parse(payloadJson || "{}");
        } catch (error) {
            return this._error("invalid_json", error.message);
        }

        const result = this._showPayload(payload, startedUs);
        return JSON.stringify(result);
    }

    ShowDemo() {
        const startedUs = GLib.get_monotonic_time();
        const payload = this._demoPayload();
        const result = this._showPayload(payload, startedUs);
        return JSON.stringify(result);
    }

    Hide() {
        this._clear("dbus-hide");
        return JSON.stringify({ ok: true });
    }

    _showPayload(payload, startedUs) {
        this._clearGroup("replace");
        this._disableUnredirect("show");

        const items = Array.isArray(payload.items) ? payload.items : [];
        const chrome = payload.chrome !== false;
        const backdrop = payload.backdrop === true;
        const ttlMs = Number.isFinite(payload.ttl_ms) ? payload.ttl_ms : DEFAULT_TTL_MS;

        this._group = new St.Widget({
            name: "qol-alt-tab-preview-plane",
            reactive: false,
            style: backdrop ? "background-color: rgba(0, 0, 0, 28);" : null
        });
        this._group.set_position(0, 0);
        this._group.set_size(global.stage.width, global.stage.height);
        global.stage.add_actor(this._group);
        this._group.raise_top();

        const results = [];
        for (let i = 0; i < items.length; i++) {
            results.push(this._addItem(items[i], chrome));
        }

        if (ttlMs > 0) {
            this._hideTimeout = Mainloop.timeout_add(ttlMs, () => {
                this._clear("ttl");
                return false;
            });
        }

        const buildMs = Math.round((GLib.get_monotonic_time() - startedUs) / 1000);
        const shown = results.filter(r => r.ok).length;
        const missed = results.filter(r => !r.ok).length;
        global.log(
            `[qol-alt-tab-preview-plane] show show_id=${payload.show_id || "none"} ` +
            `items=${items.length} shown=${shown} missed=${missed} build_ms=${buildMs}`
        );

        return {
            ok: true,
            show_id: payload.show_id || null,
            parent: "global.stage",
            items: items.length,
            shown,
            missed,
            build_ms: buildMs,
            results
        };
    }

    _addItem(item, chrome) {
        const wid = Number(item.wid);
        const rect = item.rect || item.preview_rect;
        if (!Number.isFinite(wid) || !rect) {
            return { ok: false, wid, reason: "bad_item" };
        }

        const meta = this._findMetaWindow(wid);
        if (!meta) {
            return { ok: false, wid, reason: "window_not_found" };
        }

        const x = Math.round(Number(rect.x) || 0);
        const y = Math.round(Number(rect.y) || 0);
        const w = Math.max(1, Math.round(Number(rect.w) || Number(rect.width) || 1));
        const h = Math.max(1, Math.round(Number(rect.h) || Number(rect.height) || 1));
        const selected = item.selected === true;
        const title = String(item.label || item.title || meta.get_title() || "");

        const container = new St.Widget({
            reactive: false,
            style: chrome ? this._cardStyle(selected) : null
        });
        container.set_position(x, y);
        container.set_size(w, h);
        container.set_clip(0, 0, w, h);
        this._group.add_actor(container);

        const inset = chrome ? 8 : 0;
        const labelH = chrome && title.length > 0 ? 26 : 0;
        const previewW = Math.max(1, w - inset * 2);
        const previewH = Math.max(1, h - inset * 2 - labelH);
        const clones = WindowUtils.createWindowClone(meta, 0, 0, true, true);
        const fit = this._fitClonesCover(container, clones, inset, previewW, previewH);

        if (labelH > 0) {
            const label = new St.Label({
                text: title.slice(0, 72),
                style: "font-size: 12px; color: #edf3ff; padding-left: 8px; padding-right: 8px;"
            });
            label.set_position(0, h - labelH);
            label.set_size(w, labelH);
            container.add_actor(label);
        }

        return {
            ok: fit.ok,
            wid,
            clones: clones.length,
            scale: fit.scale,
            source: { w: fit.sourceW, h: fit.sourceH },
            rect: { x, y, w, h }
        };
    }

    _fitClonesCover(container, clones, inset, previewW, previewH) {
        if (!clones || clones.length === 0) {
            return { ok: false, scale: 0, sourceW: 0, sourceH: 0 };
        }

        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        let actorCount = 0;
        for (let i = 0; i < clones.length; i++) {
            const clone = clones[i];
            const actor = clone.actor;
            if (!actor) {
                continue;
            }
            actorCount += 1;
            const width = Number(actor.width) || 1;
            const height = Number(actor.height) || 1;
            const x = Number(clone.x) || 0;
            const y = Number(clone.y) || 0;
            minX = Math.min(minX, x);
            minY = Math.min(minY, y);
            maxX = Math.max(maxX, x + width);
            maxY = Math.max(maxY, y + height);
        }

        if (actorCount === 0) {
            return { ok: false, scale: 0, sourceW: 0, sourceH: 0 };
        }

        const sourceW = Math.max(1, maxX - minX);
        const sourceH = Math.max(1, maxY - minY);
        if (!Number.isFinite(sourceW) || !Number.isFinite(sourceH)) {
            return { ok: false, scale: 0, sourceW: 0, sourceH: 0 };
        }

        const scale = Math.max(previewW / sourceW, previewH / sourceH);
        const offsetX = inset + (previewW - sourceW * scale) / 2 - minX * scale;
        const offsetY = inset + (previewH - sourceH * scale) / 2 - minY * scale;

        for (let i = 0; i < clones.length; i++) {
            const clone = clones[i];
            const actor = clone.actor;
            if (!actor) {
                continue;
            }
            const x = Number(clone.x) || 0;
            const y = Number(clone.y) || 0;
            actor.set_scale(scale, scale);
            actor.set_position(Math.round(offsetX + x * scale), Math.round(offsetY + y * scale));
            container.add_actor(actor);
        }

        return { ok: true, scale, sourceW, sourceH };
    }

    _cardStyle(selected) {
        if (selected) {
            return "background-color: rgba(40, 55, 78, 210); " +
                "border: 2px solid rgba(180, 215, 255, 255); border-radius: 8px;";
        }
        return "background-color: rgba(20, 24, 30, 200); " +
            "border: 1px solid rgba(160, 170, 190, 180); border-radius: 8px;";
    }

    _findMetaWindow(wid) {
        const actors = global.get_window_actors();
        for (let i = 0; i < actors.length; i++) {
            const meta = actors[i].get_meta_window();
            if (meta && meta.get_xwindow && Number(meta.get_xwindow()) === wid) {
                return meta;
            }
        }
        return null;
    }

    _demoPayload() {
        const focus = global.display.focus_window;
        const windows = global.get_window_actors()
            .map(actor => actor.get_meta_window())
            .filter(meta => meta && meta.get_xwindow && !meta.skip_taskbar &&
                meta.get_window_type() !== Meta.WindowType.DESKTOP)
            .sort((a, b) => {
                if (focus && a === focus) {
                    return -1;
                }
                if (focus && b === focus) {
                    return 1;
                }
                return b.get_stable_sequence() - a.get_stable_sequence();
            })
            .slice(0, 4);

        const monitorIndex = focus && focus.get_monitor ? focus.get_monitor() : Main.layoutManager.primaryIndex;
        const monitor = Main.layoutManager.monitors[monitorIndex] || Main.layoutManager.primaryMonitor;
        const cardW = Math.round(Math.min(360, monitor.width * 0.18));
        const cardH = Math.round(cardW * 0.72);
        const gap = 28;
        const baseX = monitor.x + Math.round(monitor.width * 0.08);
        const baseY = monitor.y + Math.round(monitor.height * 0.12);

        return {
            show_id: `demo-${Date.now()}`,
            chrome: true,
            backdrop: true,
            ttl_ms: DEFAULT_TTL_MS,
            items: windows.map((meta, index) => ({
                wid: meta.get_xwindow(),
                selected: index === 0,
                title: meta.get_title(),
                rect: {
                    x: baseX + index * (cardW + gap),
                    y: baseY,
                    w: cardW,
                    h: cardH
                }
            }))
        };
    }

    _clear(reason) {
        this._clearGroup(reason);
        this._enableUnredirect(reason);
    }

    _clearGroup(reason) {
        if (this._hideTimeout) {
            Mainloop.source_remove(this._hideTimeout);
            this._hideTimeout = 0;
        }
        if (this._group) {
            this._group.destroy();
            this._group = null;
            global.log(`[qol-alt-tab-preview-plane] hide reason=${reason}`);
        }
    }

    _disableUnredirect(reason) {
        if (this._unredirectDisabled) {
            return false;
        }
        Meta.disable_unredirect_for_display(global.display);
        this._unredirectDisabled = true;
        global.log(`[qol-alt-tab-preview-plane] unredirect disabled reason=${reason}`);
        return true;
    }

    _enableUnredirect(reason) {
        if (!this._unredirectDisabled) {
            return false;
        }
        Meta.enable_unredirect_for_display(global.display);
        this._unredirectDisabled = false;
        global.log(`[qol-alt-tab-preview-plane] unredirect enabled reason=${reason}`);
        return true;
    }

    _error(code, detail) {
        global.log(`[qol-alt-tab-preview-plane] error code=${code} detail=${detail}`);
        return JSON.stringify({ ok: false, code, detail });
    }
}
