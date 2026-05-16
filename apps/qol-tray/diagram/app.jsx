// Root app — mounts the diagram and wires the Tweaks panel.
const { useEffect } = React;

const ACCENT_PALETTES = [
  ["#c87a3c", "#5e7b97"],  // amber · slate (default)
  ["#b65a2e", "#3f8a8e"],  // rust · teal
  ["#5d8a4e", "#7d5da0"],  // moss · violet
  ["#5a5550", "#8a857f"],  // ink · stone (mono)
];

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme":   "light",
  "accent":  ["#c87a3c", "#5e7b97"],
  "density": "comfortable",
  "level":           "minimal",
  "showAnnotations": false
}/*EDITMODE-END*/;

function App() {
  const [tweaks, setTweak] = window.useTweaks(TWEAK_DEFAULTS);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme       = tweaks.theme;
    root.dataset.density     = tweaks.density;
    root.dataset.annotations = tweaks.showAnnotations ? "on" : "off";
    // Level drives two attributes: the macro hide-set (data-level) and the
    // inline-content reveal (data-details). "detailed" auto-enables inline.
    root.dataset.level   = tweaks.level || "minimal";
    root.dataset.details = tweaks.level === "detailed" ? "on" : "off";
    // Map [amber-ish, slate-ish] hex pair → CSS custom props the diagram reads.
    const [a, b] = Array.isArray(tweaks.accent) ? tweaks.accent : ACCENT_PALETTES[0];
    root.style.setProperty("--accent-amber-hex", a);
    root.style.setProperty("--accent-slate-hex", b);
  }, [tweaks.theme, tweaks.accent, tweaks.density, tweaks.level, tweaks.showAnnotations]);

  return (
    <React.Fragment>
      <window.Diagram tweaks={tweaks} setTweak={setTweak} />

      <window.TweaksPanel title="Tweaks" noDeckControls={true}>
        <window.TweakSection label="Theme">
          <window.TweakRadio
            label="Mode"
            value={tweaks.theme}
            onChange={(v) => setTweak("theme", v)}
            options={[
              { value: "light", label: "Paper" },
              { value: "dark",  label: "Ink"   },
            ]}
          />
          <window.TweakColor
            label="Accent"
            value={tweaks.accent}
            onChange={(v) => setTweak("accent", v)}
            options={ACCENT_PALETTES}
          />
        </window.TweakSection>

        <window.TweakSection label="Density">
          <window.TweakRadio
            label="Layout"
            value={tweaks.density}
            onChange={(v) => setTweak("density", v)}
            options={[
              { value: "compact",     label: "Compact" },
              { value: "comfortable", label: "Roomy"   },
            ]}
          />
        </window.TweakSection>

        <window.TweakSection label="Annotations">
          <window.TweakToggle
            label="Plate marks"
            value={tweaks.showAnnotations}
            onChange={(v) => setTweak("showAnnotations", v)}
          />
        </window.TweakSection>
      </window.TweaksPanel>
    </React.Fragment>
  );
}

const root = ReactDOM.createRoot(document.getElementById("root"));
root.render(<App />);
