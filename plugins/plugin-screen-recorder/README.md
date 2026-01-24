# 🎬 Screen Recorder

A [qol-tray](https://github.com/qol-tools/qol-tray) plugin for recording screen regions with optional audio.

## Features

- **Region selection** — click and drag to select any area
- **Audio capture** — mic, system audio, or both
- **Edge snapping** — automatically snaps to monitor edges
- **One-click toggle** — same hotkey starts and stops

## Dependencies

```bash
# Ubuntu/Debian
sudo apt install slop ffmpeg jq xdotool

# Arch
sudo pacman -S slop ffmpeg jq xdotool
```

## Installation

```bash
git clone https://github.com/qol-tools/plugin-screen-recorder ~/.config/qol-tray/plugins/plugin-screen-recorder
```

## Usage

1. Click **Screen Recorder → Start/Stop Recording** in the tray
2. Select a region with your mouse
3. Recording starts immediately
4. Click again to stop — saved to `~/Videos/`

## Configuration

Edit `config.json`:

```json
{
  "audio": {
    "enabled": true,
    "inputs": ["mic", "system"],
    "mic_device": "default",
    "system_device": "default"
  },
  "video": {
    "crf": 18,
    "preset": "veryfast",
    "framerate": 60,
    "format": "mkv"
  }
}
```

| Option | Description |
|--------|-------------|
| `audio.enabled` | Toggle audio capture (also in tray menu) |
| `audio.inputs` | Array: `["mic"]`, `["system"]`, or `["mic", "system"]` |
| `video.crf` | Quality (0-51, lower = better, 18 is visually lossless) |
| `video.preset` | Encoding speed: `ultrafast`, `veryfast`, `fast`, `medium` |
| `video.framerate` | FPS (30 or 60) |
| `video.format` | Container format (`mkv` or `mp4`) |

## Tips

- Use `pavucontrol` to find your audio device names
- Bind to a global hotkey for quick access
- MKV is more resilient if recording crashes; convert to MP4 later if needed

## License

MIT









